//! Creating and using bitmap images.
//!
//! krilla allows you to add bitmap images to your PDF very easily.
//! The currently supported formats include
//! - PNG
//! - JPG
//! - GIF
//! - WEBP
//! - Custom image formats via [`CustomImage`]

use std::fmt::{Debug, Formatter};
use std::hash::{Hash, Hasher};
use std::io::Cursor;
use std::ops::DerefMut;
use std::sync::Arc;

use pdf_writer::{Finish, Name, Ref};
use png::{BitDepth, ColorType, Transformations};
use zune_jpeg::zune_core::{colorspace::ColorSpace, options::DecoderOptions};
use zune_jpeg::JpegDecoder;

use crate::graphics::image_usage::DeferredImage;
use crate::Data;

mod resize;
use resize::Resampling;

use crate::chunk_container::ChunkContainer;
use crate::configure::ValidationError;
use crate::error::KrillaError;
use crate::graphics::color::DEVICE_GRAY;
use crate::graphics::color::{cmyk, luma, rgb};
use crate::graphics::icc::{GenericICCProfile, ICCBasedColorSpace, ICCProfile};
use crate::serialize::SerializeContext;
use crate::stream::{deflate_encode, FilterStreamBuilder};
use crate::util::{set_colorspace, Deferred, NameExt, SipHashable};

/// A positive finite physical image resolution, in dots per inch.
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct ImageDpi(f32);
impl ImageDpi {
    /// Rejects zero, negative and non-finite resolutions.
    pub fn new(value: f32) -> Result<Self, InvalidImageDpi> {
        if value.is_finite() && value > 0.0 {
            Ok(Self(value))
        } else {
            Err(InvalidImageDpi(value))
        }
    }
    /// The selected physical resolution.
    pub const fn get(self) -> f32 {
        self.0
    }
}
/// An invalid physical image resolution.
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct InvalidImageDpi(pub f32);
impl std::fmt::Display for InvalidImageDpi {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "image DPI must be positive and finite: {}", self.0)
    }
}
impl std::error::Error for InvalidImageDpi {}

const DEFAULT_JPEG_QUALITY: JpegQuality = JpegQuality(75);

/// A JPEG quality in the inclusive range 1 through 100.
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub struct JpegQuality(u8);

impl JpegQuality {
    /// Lowest accepted quality.
    pub const MIN: u8 = 1;
    /// Highest accepted quality.
    pub const MAX: u8 = 100;

    /// Rejects values outside the supported range instead of silently clamping.
    pub const fn new(value: u8) -> Result<Self, InvalidJpegQuality> {
        if value >= Self::MIN && value <= Self::MAX {
            Ok(Self(value))
        } else {
            Err(InvalidJpegQuality(value))
        }
    }

    /// The selected quality value.
    pub const fn get(self) -> u8 {
        self.0
    }
}

/// A requested JPEG quality outside the supported range.
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub struct InvalidJpegQuality(pub u8);

impl std::fmt::Display for InvalidJpegQuality {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "JPEG quality must be between {} and {}: {}",
            JpegQuality::MIN,
            JpegQuality::MAX,
            self.0
        )
    }
}
impl std::error::Error for InvalidJpegQuality {}

/// The number of bits per color component.
#[derive(Debug, Hash, Eq, PartialEq, Copy, Clone)]
pub enum BitsPerComponent {
    /// Eight bits per component.
    Eight,
    /// Sixteen bits per component.
    Sixteen,
}

impl BitsPerComponent {
    fn as_u8(&self) -> u8 {
        match self {
            BitsPerComponent::Eight => 8,
            BitsPerComponent::Sixteen => 16,
        }
    }
}

/// The color space of the image.
#[derive(Debug, Hash, Eq, PartialEq, Copy, Clone)]
pub enum ImageColorspace {
    /// The RGB color space.
    Rgb,
    /// The luma color space.
    Luma,
    /// The CMYK color space.
    Cmyk,
}

impl ImageColorspace {
    fn num_components(&self) -> u8 {
        match self {
            ImageColorspace::Luma => 1,
            ImageColorspace::Rgb => 3,
            ImageColorspace::Cmyk => 4,
        }
    }

    fn matches_icc_profile(&self, profile: &GenericICCProfile) -> bool {
        match self {
            ImageColorspace::Rgb => matches!(profile, GenericICCProfile::Rgb(_)),
            ImageColorspace::Luma => matches!(profile, GenericICCProfile::Luma(_)),
            ImageColorspace::Cmyk => matches!(profile, GenericICCProfile::Cmyk(_)),
        }
    }
}

impl TryFrom<ColorSpace> for ImageColorspace {
    type Error = ();

    fn try_from(value: ColorSpace) -> Result<Self, Self::Error> {
        match value {
            ColorSpace::RGB => Ok(ImageColorspace::Rgb),
            ColorSpace::RGBA => Ok(ImageColorspace::Rgb),
            ColorSpace::YCbCr => Ok(ImageColorspace::Rgb),
            ColorSpace::Luma => Ok(ImageColorspace::Luma),
            ColorSpace::LumaA => Ok(ImageColorspace::Luma),
            ColorSpace::YCCK => Ok(ImageColorspace::Cmyk),
            ColorSpace::CMYK => Ok(ImageColorspace::Cmyk),
            _ => Err(()),
        }
    }
}

struct SampledRepr {
    color_channel: Vec<u8>,
    alpha_channel: Option<Vec<u8>>,
    bits_per_component: BitsPerComponent,
}

struct JpegRepr {
    data: Data,
    bits_per_component: BitsPerComponent,
    invert_cmyk: bool,
}

enum Repr {
    Sampled(SampledRepr),
    Jpeg(JpegRepr),
}

impl Repr {
    fn bits_per_component(&self) -> BitsPerComponent {
        match self {
            Repr::Sampled(s) => s.bits_per_component,
            Repr::Jpeg(j) => j.bits_per_component,
        }
    }
}

/// A trait for custom images, which you can use if the
/// current methods provided by krilla (JPEG/PNG/WEBP/GIF) images
/// are not suitable for your own purpose.
///
/// Note that a struct implementing this trait should be cheap to
/// hash and clone, otherwise performance might be bad!
pub trait CustomImage: Hash + Clone + Send + Sync + 'static {
    /// Return the raw bytes of the color channel.
    fn color_channel(&self) -> &[u8];
    /// Return the raw bytes of the alpha channel, if available.
    fn alpha_channel(&self) -> Option<&[u8]>;
    /// Return the bits per component of the image.
    fn bits_per_component(&self) -> BitsPerComponent;
    /// Return the dimensions of the image.
    fn size(&self) -> (u32, u32);
    /// Return the ICC profile of the image, if available.
    fn icc_profile(&self) -> Option<&[u8]>;
    /// Return the color space of the image.
    fn color_space(&self) -> ImageColorspace;
}

struct ImageMetadata {
    size: (u32, u32),
    color_space: ImageColorspace,
    has_alpha: bool,
    bits_per_component: BitsPerComponent,
    icc: Option<GenericICCProfile>,
}

struct ImageRepr {
    inner: Deferred<Result<Repr, String>>,
    metadata: ImageMetadata,
    sip: u128,
    interpolate: bool,
}

impl ImageRepr {
    fn size(&self) -> (u32, u32) {
        self.metadata.size
    }

    fn icc(&self) -> Option<GenericICCProfile> {
        self.metadata.icc.clone()
    }

    fn color_space(&self) -> ImageColorspace {
        self.metadata.color_space
    }
}

impl Debug for ImageRepr {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "ImageRepr {{..}}")
    }
}

impl Hash for ImageRepr {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.sip.hash(state);
    }
}

impl PartialEq for ImageRepr {
    fn eq(&self, other: &Self) -> bool {
        self.sip == other.sip
    }
}

impl Eq for ImageRepr {}

/// A bitmap image.
///
/// This type is cheap to hash and clone, but expensive to create.
#[derive(Debug, Hash, Eq, PartialEq, Clone)]
pub struct Image {
    inner: Arc<ImageRepr>,
    allow_jpeg: bool,
}

impl Image {
    fn new(inner: ImageRepr) -> Self {
        Self {
            inner: Arc::new(inner),
            allow_jpeg: false,
        }
    }

    /// Allow optional eight-bit JPEG compression of opaque RGB or gray samples.
    pub fn with_jpeg_compression(mut self) -> Self {
        self.allow_jpeg = true;
        self
    }

    /// Create a new bitmap image from a `.png` file.
    pub fn from_png(data: Data, interpolate: bool) -> Result<Image, String> {
        let hash = (data.as_ref().sip_hash(), interpolate).sip_hash();
        let metadata = png_metadata(data.as_ref())?;

        Ok(Self::new(ImageRepr {
            inner: Deferred::new(move || decode_png(data.as_ref())),
            metadata,
            sip: hash,
            interpolate,
        }))
    }

    /// Create a new bitmap image from a `.jpg` file.
    pub fn from_jpeg(data: Data, interpolate: bool) -> Result<Image, String> {
        let hash = (data.as_ref().sip_hash(), interpolate).sip_hash();
        let metadata = jpeg_metadata(data.as_ref())?;

        Ok(Self::new(ImageRepr {
            inner: Deferred::new(move || decode_jpeg(data)),
            metadata,
            sip: hash,
            interpolate,
        }))
    }

    /// Create a new bitmap image from a `.jpg` file with custom ICC profile.
    #[doc(hidden)]
    pub fn from_jpeg_with_icc(
        data: Data,
        icc_profile: Option<Data>,
        interpolate: bool,
    ) -> Result<Image, String> {
        let hash = (data.as_ref().sip_hash(), interpolate).sip_hash();
        let mut metadata = jpeg_metadata(data.as_ref())?;
        let icc_profile =
            icc_profile.and_then(|d| get_icc_profile_type(d.as_ref(), metadata.color_space));
        metadata.icc = icc_profile;

        Ok(Self::new(ImageRepr {
            inner: Deferred::new(move || decode_jpeg(data)),
            metadata,
            sip: hash,
            interpolate,
        }))
    }

    /// Create a new bitmap image from a `.gif` file.
    pub fn from_gif(data: Data, interpolate: bool) -> Result<Image, String> {
        let hash = (data.as_ref().sip_hash(), interpolate).sip_hash();
        let metadata = gif_metadata(data.as_ref())?;

        Ok(Self::new(ImageRepr {
            inner: Deferred::new(move || decode_gif(data)),
            metadata,
            sip: hash,
            interpolate,
        }))
    }

    /// Create a new bitmap image from a `.webp` file.
    ///
    /// Returns `None` if krilla was unable to parse the file.
    pub fn from_webp(data: Data, interpolate: bool) -> Result<Image, String> {
        let hash = (data.as_ref().sip_hash(), interpolate).sip_hash();
        let metadata = webp_metadata(data.as_ref())?;

        Ok(Self::new(ImageRepr {
            inner: Deferred::new(move || decode_webp(data)),
            metadata,
            sip: hash,
            interpolate,
        }))
    }

    /// Create a new image from a custom image.
    ///
    /// Panics if the dimensions of the image and the length of the
    /// data doesn't match.
    pub fn from_custom<T: CustomImage>(image: T, interpolate: bool) -> Result<Image, String> {
        let hash = (image.clone(), interpolate).sip_hash();
        let metadata = ImageMetadata {
            size: image.size(),
            color_space: image.color_space(),
            has_alpha: image.alpha_channel().is_some(),
            bits_per_component: image.bits_per_component(),
            icc: image
                .icc_profile()
                .and_then(|d| get_icc_profile_type(d, image.color_space())),
        };

        Ok(Self::new(ImageRepr {
            inner: Deferred::new(move || {
                let bytes_per_component = (image.bits_per_component().as_u8() / 8) as u32;
                let color_channel_len = bytes_per_component
                    * image.color_space().num_components() as u32
                    * metadata.size.0
                    * metadata.size.1;
                let color_channel = image.color_channel();
                assert_eq!(color_channel.len(), color_channel_len as usize);

                let alpha_channel_len = bytes_per_component * metadata.size.0 * metadata.size.1;
                let alpha_channel = image.alpha_channel();
                if let Some(alpha_channel) = alpha_channel {
                    assert_eq!(alpha_channel.len(), alpha_channel_len as usize);
                }

                Ok(Repr::Sampled(SampledRepr {
                    color_channel: deflate_encode(color_channel),
                    alpha_channel: image.alpha_channel().map(deflate_encode),
                    bits_per_component: image.bits_per_component(),
                }))
            }),
            metadata,
            sip: hash,
            interpolate,
        }))
    }

    /// Create a new RGB image from raw RGBA pixels.
    pub fn from_rgba8(data: Vec<u8>, width: u32, height: u32) -> Self {
        let hash = data.sip_hash();
        let metadata = ImageMetadata {
            has_alpha: true,
            size: (width, height),
            bits_per_component: BitsPerComponent::Eight,
            color_space: ImageColorspace::Rgb,
            icc: None,
        };

        Self::new(ImageRepr {
            inner: Deferred::new(move || {
                let (color_channel, alpha_channel, bits_per_component) =
                    handle_u8_image(&data, ColorSpace::RGBA);

                Ok(Repr::Sampled(SampledRepr {
                    color_channel,
                    alpha_channel,
                    bits_per_component,
                }))
            }),
            metadata,
            sip: hash,
            interpolate: false,
        })
    }

    /// The image enlarged by whole `multiples` of its size along each axis, every sample
    /// repeated over a block of that many, as nearest-neighbour scaling enlarges it. Multiples
    /// beyond the resampling working budget are halved until the enlargement fits it.
    pub fn with_nearest_multiples(&self, mut multiples: (u32, u32)) -> Image {
        const NOT_ENLARGED: (u32, u32) = (1, 1);
        let size = self.size();
        let enlarged = |(x, y): (u32, u32)| {
            size.0
                .checked_mul(x)
                .zip(size.1.checked_mul(y))
                .filter(|target| {
                    resize::working_budget(
                        self.color_space(),
                        self.inner.metadata.bits_per_component,
                        self.inner.metadata.has_alpha,
                        size,
                        *target,
                    )
                    .is_ok()
                })
        };
        let target = loop {
            multiples = (multiples.0.max(1), multiples.1.max(1));
            if multiples == NOT_ENLARGED {
                return self.clone();
            }
            if let Some(target) = enlarged(multiples) {
                break target;
            }
            multiples = (multiples.0 / 2, multiples.1 / 2);
        };
        let source = self.clone();
        let metadata = ImageMetadata {
            size: target,
            color_space: self.color_space(),
            has_alpha: self.inner.metadata.has_alpha,
            bits_per_component: self.inner.metadata.bits_per_component,
            icc: self.icc(),
        };
        Self {
            inner: Arc::new(ImageRepr {
                inner: Deferred::new(move || source.enlarged(target)),
                metadata,
                sip: (self.inner.sip, multiples).sip_hash(),
                interpolate: self.inner.interpolate,
            }),
            allow_jpeg: self.allow_jpeg,
        }
    }

    /// The samples resampled to `target` by nearest neighbour.
    fn enlarged(&self, target: (u32, u32)) -> Result<Repr, String> {
        let size = self.size();
        let color_space = self.color_space();
        match self.inner.inner.wait().as_ref().map_err(Clone::clone)? {
            Repr::Sampled(sampled) => {
                resize::sampled(sampled, color_space, size, target, Resampling::Nearest)
                    .map(Repr::Sampled)
            }
            Repr::Jpeg(jpeg) => {
                let (samples, _) = jpeg_samples(jpeg, color_space, size, false)?;
                let color =
                    resize::jpeg(samples, color_space, size, target, Resampling::Nearest)?;
                Ok(Repr::Sampled(SampledRepr {
                    color_channel: deflate_encode(&color),
                    alpha_channel: None,
                    bits_per_component: BitsPerComponent::Eight,
                }))
            }
        }
    }

    /// The resampling that keeps the look `interpolate` asks the viewer for.
    fn resampling(&self) -> Resampling {
        if self.inner.interpolate {
            Resampling::Smooth
        } else {
            Resampling::Nearest
        }
    }

    /// Resolve deferred decoding without decoding the same image again during serialization.
    pub fn validate(&self) -> Result<(), String> {
        self.inner
            .inner
            .wait()
            .as_ref()
            .map(|_| ())
            .map_err(Clone::clone)
    }

    /// Return the size of the image.
    pub fn size(&self) -> (u32, u32) {
        self.inner.size()
    }

    fn icc(&self) -> Option<GenericICCProfile> {
        self.inner.icc()
    }

    fn color_space(&self) -> ImageColorspace {
        self.inner.color_space()
    }

    pub(crate) fn serialize(
        self,
        sc: &mut SerializeContext,
        chunk_container: &mut ChunkContainer,
        root_ref: Ref,
    ) {
        let soft_mask_id = self.inner.metadata.has_alpha.then(|| {
            sc.register_validation_error(ValidationError::Transparency(sc.location));
            sc.new_ref()
        });

        let icc_ref = self.icc().and_then(|ic| {
            if sc
                .serialize_settings()
                .pdf_version()
                .supports_icc(ic.metadata())
                && self.color_space().matches_icc_profile(&ic)
            {
                let ref_ = match ic {
                    GenericICCProfile::Luma(l) => {
                        sc.register_cacheable(chunk_container, ICCBasedColorSpace(l))
                    }
                    GenericICCProfile::Rgb(r) => {
                        sc.register_cacheable(chunk_container, ICCBasedColorSpace(r))
                    }
                    GenericICCProfile::Cmyk(c) => {
                        sc.register_cacheable(chunk_container, ICCBasedColorSpace(c))
                    }
                };

                Some(ref_)
            } else {
                // Don't embed ICC profiles from images if the current
                // PDF version does not support it.
                None
            }
        });

        if self.inner.interpolate {
            sc.register_validation_error(ValidationError::ImageInterpolation(sc.location));
        }

        let serialize_settings = sc.serialize_settings().clone();

        let cs = {
            let cs = match self.color_space() {
                ImageColorspace::Rgb => {
                    rgb::color_space(sc.serialize_settings().no_device_cs).into()
                }
                ImageColorspace::Luma => {
                    luma::color_space(sc.serialize_settings().no_device_cs).into()
                }
                ImageColorspace::Cmyk => match cmyk::color_space(&sc.serialize_settings()) {
                    None => {
                        sc.register_validation_error(ValidationError::MissingCMYKProfile);
                        crate::color::DeviceColorSpace::Cmyk.into()
                    }
                    Some(cs) => cs.into(),
                },
            };

            sc.register_colorspace(chunk_container, cs)
        };

        let supports_bit_depth = sc
            .serialize_settings()
            .configuration
            .version()
            .supports_bit_depth(self.inner.metadata.bits_per_component);
        let location = sc.location;
        let mut chunk = sc.new_chunk();

        let source_size = self.size();
        let hash = self.sip_hash();
        let create_chunk = move |mut output_size: (u32, u32)| {
            Deferred::new(move || {
                if !supports_bit_depth {
                    return Err(KrillaError::SixteenBitImage(self.clone(), location));
                }

                let repr = self
                    .inner
                    .inner
                    .wait()
                    .as_ref()
                    .map_err(|e| KrillaError::Image(self.clone(), location, e.clone()))?;

                let can_resample = output_size == source_size
                    || resize::working_budget(
                        self.color_space(),
                        repr.bits_per_component(),
                        self.inner.metadata.has_alpha,
                        source_size,
                        output_size,
                    )
                    .is_ok();
                if !can_resample {
                    output_size = source_size;
                }
                let resized = output_size != source_size;
                let mut encoded = match repr {
                    Repr::Jpeg(jpeg)
                        if can_resample
                            && (resized || serialize_settings.jpeg_quality.is_some()) =>
                    {
                        match reencode_jpeg(
                            jpeg,
                            self.color_space(),
                            source_size,
                            output_size,
                            self.resampling(),
                            serialize_settings
                                .jpeg_quality
                                .unwrap_or(DEFAULT_JPEG_QUALITY),
                        ) {
                            Ok(encoded) => Some(encoded),
                            Err(_) => {
                                // Optional optimisation must not invalidate a pass-through JPEG.
                                output_size = source_size;
                                None
                            }
                        }
                    }
                    Repr::Sampled(sampled) if resized => Some(Repr::Sampled(
                        resize::sampled(
                            sampled,
                            self.color_space(),
                            source_size,
                            output_size,
                            self.resampling(),
                        )
                        .map_err(|error| KrillaError::Image(self.clone(), location, error))?,
                    )),
                    _ => None,
                };
                if (self.allow_jpeg
                    || self.inner.metadata.bits_per_component == BitsPerComponent::Sixteen)
                    && can_resample
                {
                    if let Some(quality) = serialize_settings.jpeg_quality {
                        if let Some(jpeg) = sampled_jpeg(
                            encoded.as_ref().unwrap_or(repr),
                            self.color_space(),
                            output_size,
                            quality,
                        )
                        .map_err(|error| KrillaError::Image(self.clone(), location, error))?
                        {
                            encoded = Some(jpeg);
                        }
                    }
                }
                let repr = encoded.as_ref().unwrap_or(repr);

                let alpha_mask = match repr {
                    Repr::Sampled(sampled) => sampled.alpha_channel.as_ref().map(|mask_data| {
                        let soft_mask_id = soft_mask_id.unwrap();
                        let mask_stream = FilterStreamBuilder::new_from_deflated(mask_data)
                            .finish(&serialize_settings);
                        let mut s_mask =
                            chunk.image_xobject(soft_mask_id, mask_stream.encoded_data());
                        mask_stream.write_filters(s_mask.deref_mut().deref_mut());
                        s_mask.width(output_size.0 as i32);
                        s_mask.height(output_size.1 as i32);
                        s_mask.pair(
                            Name(b"ColorSpace"),
                            // Mask color space must be device gray -- see Table 145.
                            DEVICE_GRAY.to_pdf_name(),
                        );

                        if self.inner.interpolate {
                            s_mask.interpolate(true);
                        }

                        s_mask.bits_per_component(repr.bits_per_component().as_u8() as i32);
                        soft_mask_id
                    }),
                    Repr::Jpeg(_) => None,
                };

                let filter_stream = match repr {
                    Repr::Sampled(s) => FilterStreamBuilder::new_from_deflated(&s.color_channel)
                        .finish(&serialize_settings),
                    Repr::Jpeg(j) => FilterStreamBuilder::new_from_jpeg_data(j.data.as_ref())
                        .finish(&serialize_settings),
                };

                let mut image_x_object =
                    chunk.image_xobject(root_ref, filter_stream.encoded_data());
                filter_stream.write_filters(image_x_object.deref_mut().deref_mut());
                image_x_object.width(output_size.0 as i32);
                image_x_object.height(output_size.1 as i32);

                if let Some(icc_ref) = icc_ref {
                    image_x_object.pair(Name(b"ColorSpace"), icc_ref);
                } else {
                    set_colorspace(cs, image_x_object.deref_mut());
                }

                if self.inner.interpolate {
                    image_x_object.interpolate(true);
                }

                // Photoshop CMYK images need to be inverted, see
                // https://github.com/sile-typesetter/libtexpdf/blob/1891bee5e0b73165e4a259f910d3ea3fe1df0b42/jpegimage.c#L25-L51
                // I'm not sure if this applies to all JPEG CMYK images out there, but for now we just
                // always do it. In libtexpdf, they only seem to do it if they can find the Adobe APP
                // marker.
                if let Repr::Jpeg(j) = repr {
                    if j.invert_cmyk {
                        image_x_object.decode(
                            [1.0, 0.0].repeat(self.color_space().num_components() as usize),
                        );
                    }
                }

                image_x_object.bits_per_component(repr.bits_per_component().as_u8() as i32);
                if let Some(soft_mask_id) = alpha_mask {
                    image_x_object.s_mask(soft_mask_id);
                }
                image_x_object.finish();

                Ok(chunk)
            })
        };

        if sc.tracks_image_uses() {
            sc.deferred_images.push(DeferredImage {
                hash,
                source_size,
                encode: Box::new(create_chunk),
            });
        } else {
            chunk_container
                .streams
                .images
                .push(create_chunk(source_size));
        }
    }
}

const PNG_TRANSFORMATIONS: Transformations = Transformations::EXPAND;

fn png_metadata(data: &[u8]) -> Result<ImageMetadata, String> {
    let cursor = Cursor::new(data);
    let mut decoder = png::Decoder::new(cursor);
    decoder.set_transformations(PNG_TRANSFORMATIONS);
    let reader = decoder
        .read_info()
        .map_err(|e| e.to_string().to_ascii_lowercase())?;
    let info = reader.info();

    let size = (info.width, info.height);
    let (color_type, bit_depth) = reader.output_color_type();
    let bits_per_component = match bit_depth {
        // We will normalize to 8 when decoding.
        BitDepth::One | BitDepth::Two | BitDepth::Four | BitDepth::Eight => BitsPerComponent::Eight,
        BitDepth::Sixteen => BitsPerComponent::Sixteen,
    };

    let (image_color_space, has_alpha) = match color_type {
        ColorType::Grayscale => (ImageColorspace::Luma, false),
        ColorType::GrayscaleAlpha => (ImageColorspace::Luma, true),
        ColorType::Rgb => (ImageColorspace::Rgb, false),
        ColorType::Rgba => (ImageColorspace::Rgb, true),
        ColorType::Indexed => {
            return Err("image uses an indexed color space, which is unsupported".to_string());
        }
    };
    let icc = info
        .icc_profile
        .as_ref()
        .and_then(|i| get_icc_profile_type(i, image_color_space));

    Ok(ImageMetadata {
        has_alpha,
        size,
        bits_per_component,
        color_space: image_color_space,
        icc,
    })
}

fn decode_png(data: &[u8]) -> Result<Repr, String> {
    let cursor = Cursor::new(data);
    let mut decoder = png::Decoder::new(cursor);
    decoder.set_transformations(PNG_TRANSFORMATIONS);
    let mut reader = decoder
        .read_info()
        .map_err(|e| e.to_string().to_ascii_lowercase())?;
    let mut img_data = vec![0; reader.output_buffer_size().ok_or("image is too large")?];
    let _ = reader
        .next_frame(&mut img_data)
        .map_err(|e| e.to_string())?;
    let (color_type, bit_depth) = reader.output_color_type();

    let color_space = match color_type {
        ColorType::Rgb => ColorSpace::RGB,
        ColorType::Rgba => ColorSpace::RGBA,
        ColorType::Grayscale => ColorSpace::Luma,
        ColorType::GrayscaleAlpha => ColorSpace::LumaA,
        _ => unreachable!(),
    };

    let (color_channel, alpha_channel, bits_per_component) = match bit_depth {
        BitDepth::Eight => handle_u8_image(&img_data, color_space),
        BitDepth::Sixteen => handle_u16_image(&img_data, color_space),
        _ => return Err("image has an unsupported bit-depth".to_string()),
    };

    Ok(Repr::Sampled(SampledRepr {
        color_channel,
        alpha_channel,
        bits_per_component,
    }))
}

fn jpeg_decoder_options() -> DecoderOptions {
    DecoderOptions::new_safe()
        .set_max_width(usize::from(u16::MAX))
        .set_max_height(usize::from(u16::MAX))
}

fn jpeg_metadata(data: &[u8]) -> Result<ImageMetadata, String> {
    let reader = Cursor::new(data);
    let mut decoder = JpegDecoder::new_with_options(reader, jpeg_decoder_options());
    decoder
        .decode_headers()
        .map_err(|e| e.to_string().to_ascii_lowercase())?;

    let size = {
        let dimensions = decoder
            .dimensions()
            .ok_or("failed to read image dimensions".to_string())?;
        (dimensions.0 as u32, dimensions.1 as u32)
    };

    let image_color_space = decoder
        .input_colorspace()
        .and_then(|c| c.try_into().ok())
        .ok_or("failed to read image colorspace".to_string())?;

    let icc = decoder
        .icc_profile()
        .and_then(|d| get_icc_profile_type(&d, image_color_space));

    Ok(ImageMetadata {
        has_alpha: false,
        size,
        bits_per_component: BitsPerComponent::Eight,
        color_space: image_color_space,
        icc,
    })
}

fn decode_jpeg(data: Data) -> Result<Repr, String> {
    let reader = Cursor::new(data.as_ref());
    let mut decoder = JpegDecoder::new_with_options(reader, jpeg_decoder_options());
    decoder
        .decode_headers()
        .map_err(|e| e.to_string().to_ascii_lowercase())?;

    let input_color_space = decoder
        .input_colorspace()
        .ok_or("failed to read image colorspace".to_string())?;

    if matches!(
        input_color_space,
        ColorSpace::Luma
            | ColorSpace::YCbCr
            | ColorSpace::RGB
            | ColorSpace::CMYK
            | ColorSpace::YCCK
    ) {
        Ok(Repr::Jpeg(JpegRepr {
            data,
            bits_per_component: BitsPerComponent::Eight,
            invert_cmyk: matches!(input_color_space, ColorSpace::YCCK | ColorSpace::CMYK),
        }))
    } else {
        // JPEGs shouldn't be able to have a different color space?
        Err("image has an unknown color space".to_string())
    }
}

fn reencode_jpeg(
    jpeg: &JpegRepr,
    color_space: ImageColorspace,
    size: (u32, u32),
    target: (u32, u32),
    resampling: Resampling,
    quality: JpegQuality,
) -> Result<Repr, String> {
    use jpeg_encoder::ColorType;

    let (mut samples, decoded_space) = jpeg_samples(jpeg, color_space, size, true)?;
    let encoded_space = match decoded_space {
        ColorSpace::Luma => ColorType::Luma,
        ColorSpace::YCCK => ColorType::Ycck,
        ColorSpace::CMYK => ColorType::Cmyk,
        _ => ColorType::Rgb,
    };
    if size != target {
        samples = resize::jpeg(samples, color_space, size, target, resampling)?;
    }
    encode_jpeg(&samples, encoded_space, target, quality)
}

/// The JPEG's samples in `color_space` and the space they were decoded to, with CMYK as
/// ordinary ink values; with `keep_ycck`, YCCK data stays YCCK.
fn jpeg_samples(
    jpeg: &JpegRepr,
    color_space: ImageColorspace,
    size: (u32, u32),
    keep_ycck: bool,
) -> Result<(Vec<u8>, ColorSpace), String> {
    let options = jpeg_decoder_options()
        .set_strict_mode(true)
        .set_max_width(size.0 as usize)
        .set_max_height(size.1 as usize);
    let mut decoder = JpegDecoder::new_with_options(Cursor::new(jpeg.data.as_ref()), options);
    decoder
        .decode_headers()
        .map_err(|error| error.to_string())?;
    let decoded_space = match color_space {
        ImageColorspace::Rgb => ColorSpace::RGB,
        ImageColorspace::Luma => ColorSpace::Luma,
        ImageColorspace::Cmyk
            if keep_ycck && decoder.input_colorspace() == Some(ColorSpace::YCCK) =>
        {
            ColorSpace::YCCK
        }
        ImageColorspace::Cmyk => ColorSpace::CMYK,
    };
    decoder.set_options(decoder.options().jpeg_set_out_colorspace(decoded_space));
    let mut samples = decoder.decode().map_err(|error| error.to_string())?;
    drop(decoder);
    if decoded_space == ColorSpace::CMYK && jpeg.invert_cmyk {
        // The encoder accepts ordinary CMYK; JPEG stores Adobe CMYK inverted.
        samples
            .iter_mut()
            .for_each(|sample| *sample = u8::MAX - *sample);
    }
    Ok((samples, decoded_space))
}

fn sampled_jpeg(
    source: &Repr,
    color_space: ImageColorspace,
    size: (u32, u32),
    quality: JpegQuality,
) -> Result<Option<Repr>, String> {
    let Repr::Sampled(source) = source else {
        return Ok(None);
    };
    if source.alpha_channel.is_some() {
        return Ok(None);
    }
    let color_type = match color_space {
        ImageColorspace::Rgb => jpeg_encoder::ColorType::Rgb,
        ImageColorspace::Luma => jpeg_encoder::ColorType::Luma,
        ImageColorspace::Cmyk => return Ok(None),
    };
    let length = (size.0 as usize)
        .checked_mul(size.1 as usize)
        .and_then(|pixels| pixels.checked_mul(color_space.num_components() as usize))
        .ok_or("image plane size overflows")?;
    let depth = usize::from(source.bits_per_component.as_u8() / 8);
    let mut samples = resize::inflate(
        &source.color_channel,
        length
            .checked_mul(depth)
            .ok_or("image plane size overflows")?,
    )?;
    if source.bits_per_component == BitsPerComponent::Sixteen {
        for index in 0..length {
            let value = u16::from_be_bytes([samples[index * 2], samples[index * 2 + 1]]);
            samples[index] = ((u32::from(value) * u32::from(u8::MAX) + u32::from(u16::MAX) / 2)
                / u32::from(u16::MAX)) as u8;
        }
        samples.truncate(length);
    }
    // Keep lossless samples if optional JPEG encoding cannot represent this image.
    Ok(encode_jpeg(&samples, color_type, size, quality).ok())
}

fn encode_jpeg(
    samples: &[u8],
    encoded_space: jpeg_encoder::ColorType,
    target: (u32, u32),
    quality: JpegQuality,
) -> Result<Repr, String> {
    use jpeg_encoder::{ChromaSubsamplingMethod, Encoder, QuantizationTableType, SamplingFactor};
    let width = u16::try_from(target.0).map_err(|_| "JPEG width exceeds the format limit")?;
    let height = u16::try_from(target.1).map_err(|_| "JPEG height exceeds the format limit")?;
    let mut output = Vec::new();
    let mut encoder = Encoder::new(&mut output, quality.get());
    encoder.set_chroma_subsampling_method(ChromaSubsamplingMethod::Average);
    if matches!(encoded_space, jpeg_encoder::ColorType::Cmyk) {
        // All four channels are ink values, not subsampleable chroma.
        encoder.set_sampling_factor(SamplingFactor::F_1_1);
        let table = jpeg_ink_quantization(quality);
        encoder.set_quantization_tables(
            QuantizationTableType::Custom(Box::new(table)),
            QuantizationTableType::Custom(Box::new(table)),
        );
    }
    encoder
        .encode(samples, width, height, encoded_space)
        .map_err(|error| error.to_string())?;
    decode_jpeg(Arc::new(output).into())
}

// ITU-T T.81 Annex K luminance table: every CMYK component carries ink detail.
fn jpeg_ink_quantization(quality: JpegQuality) -> [u16; 64] {
    const LUMA: [u32; 64] = [
        16, 11, 10, 16, 24, 40, 51, 61, 12, 12, 14, 19, 26, 58, 60, 55, 14, 13, 16, 24, 40, 57, 69,
        56, 14, 17, 22, 29, 51, 87, 80, 62, 18, 22, 37, 56, 68, 109, 103, 77, 24, 35, 55, 64, 81,
        104, 113, 92, 49, 64, 78, 87, 103, 121, 120, 101, 72, 92, 95, 98, 112, 100, 103, 99,
    ];
    const QUALITY_MIDPOINT: u32 = 50;
    const QUALITY_MAXIMUM: u32 = 100;
    let quality = u32::from(quality.get());
    let scale = if quality < QUALITY_MIDPOINT {
        QUALITY_MIDPOINT * QUALITY_MAXIMUM / quality
    } else {
        2 * (QUALITY_MAXIMUM - quality)
    };
    LUMA.map(|value| {
        ((value * scale + QUALITY_MIDPOINT) / QUALITY_MAXIMUM).clamp(1, u32::from(u8::MAX)) as u16
    })
}

fn decode_gif(data: Data) -> Result<Repr, String> {
    const RGB_COMPONENTS: usize = 3;
    const RGBA_COMPONENTS: usize = 4;
    const OPAQUE_ALPHA: u8 = u8::MAX;
    let size = gif_metadata(data.as_ref())?.size;
    let frame_bytes = u64::from(size.0)
        .checked_mul(u64::from(size.1))
        .and_then(|pixels| pixels.checked_mul(RGBA_COMPONENTS as u64))
        .and_then(std::num::NonZeroU64::new)
        .ok_or("invalid GIF canvas dimensions")?;
    let mut decoder = gif::DecodeOptions::new();
    decoder.set_memory_limit(gif::MemoryLimit::Bytes(frame_bytes));
    decoder.set_color_output(gif::ColorOutput::RGBA);
    let mut decoder = decoder
        .read_info(data.as_ref())
        .map_err(|e| e.to_string().to_ascii_lowercase())?;
    let width = usize::from(decoder.width());
    let height = usize::from(decoder.height());
    let background = decoder
        .bg_color()
        .and_then(|index| {
            decoder
                .global_palette()?
                .get(index * RGB_COMPONENTS..(index + 1) * RGB_COMPONENTS)
                .map(|rgb| [rgb[0], rgb[1], rgb[2], OPAQUE_ALPHA])
        })
        .unwrap_or([0; RGBA_COMPONENTS]);
    let first_frame = decoder
        .read_next_frame()
        .map_err(|e| e.to_string())?
        .ok_or("GIF image seems to be empty".to_string())?;
    let background = if first_frame.transparent.is_some() {
        [0; RGBA_COMPONENTS]
    } else {
        background
    };
    let byte_count = width
        .checked_mul(height)
        .and_then(|pixels| pixels.checked_mul(RGBA_COMPONENTS))
        .ok_or("GIF canvas size overflows".to_string())?;
    let mut canvas = vec![0; byte_count];
    for pixel in canvas.chunks_exact_mut(RGBA_COMPONENTS) {
        pixel.copy_from_slice(&background);
    }
    let frame_width = usize::from(first_frame.width);
    let frame_height = usize::from(first_frame.height);
    let left = usize::from(first_frame.left);
    let top = usize::from(first_frame.top);
    for y in 0..frame_height.min(height.saturating_sub(top)) {
        for x in 0..frame_width.min(width.saturating_sub(left)) {
            let source = (y * frame_width + x) * RGBA_COMPONENTS;
            let target = ((top + y) * width + left + x) * RGBA_COMPONENTS;
            if first_frame.buffer[source + RGB_COMPONENTS] != 0 {
                canvas[target..target + RGBA_COMPONENTS]
                    .copy_from_slice(&first_frame.buffer[source..source + RGBA_COMPONENTS]);
            }
        }
    }
    let (color_channel, alpha_channel, bits_per_component) =
        handle_u8_image(&canvas, ColorSpace::RGBA);

    Ok(Repr::Sampled(SampledRepr {
        color_channel,
        alpha_channel,
        bits_per_component,
    }))
}

fn gif_metadata(data: &[u8]) -> Result<ImageMetadata, String> {
    let size = imagesize::blob_size(data).map_err(|e| e.to_string().to_ascii_lowercase())?;

    Ok(ImageMetadata {
        // We always decode GIFs using RGBA, see `decode_gif`
        has_alpha: true,
        bits_per_component: BitsPerComponent::Eight,
        size: (size.width as u32, size.height as u32),
        color_space: ImageColorspace::Rgb,
        icc: None,
    })
}

fn webp_metadata(data: &[u8]) -> Result<ImageMetadata, String> {
    let mut decoder = image_webp::WebPDecoder::new(std::io::Cursor::new(data))
        .map_err(|e| e.to_string().to_ascii_lowercase())?;
    let size = decoder.dimensions();
    let color_space = ImageColorspace::Rgb;
    let icc = decoder
        .icc_profile()
        .map_err(|e| e.to_string().to_ascii_lowercase())?
        .and_then(|d| get_icc_profile_type(&d, color_space));

    Ok(ImageMetadata {
        has_alpha: decoder.has_alpha(),
        bits_per_component: BitsPerComponent::Eight,
        size,
        color_space,
        icc,
    })
}

fn decode_webp(data: Data) -> Result<Repr, String> {
    let mut decoder = image_webp::WebPDecoder::new(std::io::Cursor::new(data.as_ref()))
        .map_err(|e| e.to_string().to_ascii_lowercase())?;
    let mut first_frame = vec![0; decoder.output_buffer_size().ok_or("image is too large")?];
    decoder
        .read_image(&mut first_frame)
        .map_err(|e| e.to_string().to_ascii_lowercase())?;

    let color_space = if decoder.has_alpha() {
        ColorSpace::RGBA
    } else {
        ColorSpace::RGB
    };

    let (color_channel, alpha_channel, bits_per_component) =
        handle_u8_image(&first_frame, color_space);

    Ok(Repr::Sampled(SampledRepr {
        color_channel,
        alpha_channel,
        bits_per_component,
    }))
}

fn handle_u8_image(data: &[u8], cs: ColorSpace) -> (Vec<u8>, Option<Vec<u8>>, BitsPerComponent) {
    let mut alphas = if cs.has_alpha() {
        Vec::with_capacity(data.len() / cs.num_components())
    } else {
        Vec::new()
    };

    let color_channel = match cs {
        ColorSpace::RGB => deflate_encode(data),
        ColorSpace::RGBA => {
            let mut buf = Vec::with_capacity(data.len() * 3 / 4);
            data.chunks_exact(4).for_each(|data| {
                buf.extend_from_slice(&data[0..3]);
                alphas.push(data[3]);
            });
            deflate_encode(&buf)
        }
        ColorSpace::Luma => deflate_encode(data),
        ColorSpace::LumaA => {
            let mut buf = Vec::with_capacity(data.len() / 2);
            data.chunks_exact(2).for_each(|data| {
                buf.push(data[0]);
                alphas.push(data[1]);
            });
            deflate_encode(&buf)
        }
        // PNG/WEBP/GIF only support those three, so should be enough?
        _ => unimplemented!(),
    };

    let alpha_channel = if !alphas.is_empty() && alphas.iter().any(|v| *v != 255) {
        Some(deflate_encode(&alphas))
    } else {
        None
    };

    (color_channel, alpha_channel, BitsPerComponent::Eight)
}

fn handle_u16_image(data: &[u8], cs: ColorSpace) -> (Vec<u8>, Option<Vec<u8>>, BitsPerComponent) {
    let mut alphas = if cs.has_alpha() {
        Vec::with_capacity(data.len() / cs.num_components())
    } else {
        Vec::new()
    };

    let encoded_image = match cs {
        ColorSpace::RGB => deflate_encode(data),
        ColorSpace::RGBA => {
            let mut buf = Vec::with_capacity(data.len() * 3 / 4);
            data.chunks_exact(8).for_each(|data| {
                buf.extend_from_slice(&data[0..6]);
                alphas.extend_from_slice(&data[6..]);
            });
            deflate_encode(&buf)
        }
        ColorSpace::Luma => deflate_encode(data),
        ColorSpace::LumaA => {
            let mut buf = Vec::with_capacity(data.len() / 2);
            data.chunks_exact(4).for_each(|data| {
                buf.extend_from_slice(&data[0..2]);
                alphas.extend_from_slice(&data[2..]);
            });
            deflate_encode(&buf)
        }
        // PNG/WEBP/GIF only support those three, so should be enough?
        _ => unimplemented!(),
    };

    let encoded_mask = if !alphas.is_empty() {
        Some(deflate_encode(&alphas))
    } else {
        None
    };

    (encoded_image, encoded_mask, BitsPerComponent::Sixteen)
}

fn get_icc_profile_type(data: &[u8], color_space: ImageColorspace) -> Option<GenericICCProfile> {
    let wrapper = match color_space {
        ImageColorspace::Rgb => GenericICCProfile::Rgb(ICCProfile::new(data)?),
        ImageColorspace::Luma => GenericICCProfile::Luma(ICCProfile::new(data)?),
        ImageColorspace::Cmyk => GenericICCProfile::Cmyk(ICCProfile::new(data)?),
    };

    Some(wrapper)
}

#[cfg(test)]
mod tests {
    use crate::image::Image;
    use std::sync::Arc;

    #[test]
    fn invalid_png_image() {
        let e = Image::from_png(Arc::new(b"dfngiudfg".to_vec()).into(), false);
        assert!(e.is_err());
    }

    #[test]
    fn invalid_jpeg_image() {
        let e = Image::from_jpeg(Arc::new(b"dfngiudfg".to_vec()).into(), false);
        assert!(e.is_err());
    }
}
