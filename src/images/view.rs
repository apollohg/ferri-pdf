//! Images as an `<image>` uses them: the region a media fragment selects, re-oriented for the
//! element (CSS Images 4 §2.5).
use crate::images::{Image, NaturalDimensions, Orientation};
use crate::paint::{ImageRequest, Rect};
use std::sync::Arc;

const FRAGMENT: char = '#';
/// Media Fragments 1 §4.1 name-value pairs.
const PAIR_SEPARATOR: char = '&';
const VALUE_SEPARATOR: char = '=';
/// Media Fragments 1 §4.2.2 spatial dimension.
const SPATIAL: &str = "xywh";
const UNIT_SEPARATOR: char = ':';
const PIXEL_UNIT: &str = "pixel";
const PERCENT_UNIT: &str = "percent";
const COORDINATE_SEPARATOR: char = ',';
const COORDINATES: usize = 4;
const PERCENT: f32 = 100.0;

/// A fragment identifier `image()` cannot use (CSS Images 4 §2.5.2).
#[derive(Debug, thiserror::Error)]
#[error("unsupported image fragment identifier in {0}")]
pub struct InvalidFragment(pub String);

/// Part of an image, turned and flipped after the image's own orientation.
#[derive(Debug)]
pub struct ImageView {
    pub image: Arc<Image>,
    /// The selected region in the image's natural pixels.
    pub region: Option<Rect>,
    pub orientation: Orientation,
}
impl ImageView {
    pub(crate) fn natural_size(&self) -> NaturalDimensions {
        let natural = self.image.natural_size();
        let size = match self.region {
            Some(region) => NaturalDimensions {
                width: Some(region.width),
                height: Some(region.height),
                ratio: Some(region.width / region.height),
            },
            None => natural,
        };
        if transposes(self.orientation) {
            NaturalDimensions {
                width: size.height,
                height: size.width,
                ratio: size.ratio.map(f32::recip),
            }
        } else {
            size
        }
    }
    pub(crate) fn pixel_size(&self, available_width: f32) -> [f32; 2] {
        let [width, height] = self.region.map_or_else(
            || self.image.pixel_size(available_width),
            |region| [region.width, region.height],
        );
        if transposes(self.orientation) {
            [height, width]
        } else {
            [width, height]
        }
    }
    /// The size of the selected region before the view's orientation, for a view drawn at
    /// `size`.
    pub(crate) fn region_size(&self, [width, height]: [f32; 2]) -> [f32; 2] {
        if transposes(self.orientation) {
            [height, width]
        } else {
            [width, height]
        }
    }
    /// Where the whole image draws when the region fills `0, 0, size`.
    pub(crate) fn image_rect(&self, size: [f32; 2]) -> Rect {
        let Some(region) = self.region else {
            return Rect {
                x: 0.0,
                y: 0.0,
                width: size[0],
                height: size[1],
            };
        };
        let natural = self.image.natural_size();
        let scale = [size[0] / region.width, size[1] / region.height];
        Rect {
            x: -region.x * scale[0],
            y: -region.y * scale[1],
            width: natural.width.unwrap_or(region.width) * scale[0],
            height: natural.height.unwrap_or(region.height) * scale[1],
        }
    }
}

/// `image` as `request` uses it on an element whose direction is `rtl`, with a vertical inline
/// axis when `vertical`: the region its spatial media fragment selects, flipped in the inline
/// direction when its directionality is the element's opposite (CSS Images 4 §2.5.2, §2.5.4).
/// A `url()` ignores a fragment it cannot use; `image()` is then invalid.
pub(crate) fn view(
    image: Arc<Image>,
    request: &ImageRequest<'_>,
    rtl: bool,
    vertical: bool,
) -> Result<Arc<Image>, InvalidFragment> {
    let region = match fragment_region(request.url, image.natural_size()) {
        Ok(region) => region,
        Err(error) if request.function.is_some() => return Err(error),
        Err(_) => None,
    };
    let flipped = request
        .function
        .and_then(|function| function.rtl)
        .is_some_and(|image_rtl| image_rtl != rtl);
    let orientation = match (flipped, vertical) {
        (false, _) => Orientation::NoTransforms,
        (true, false) => Orientation::FlipHorizontal,
        (true, true) => Orientation::FlipVertical,
    };
    if region.is_none() && orientation == Orientation::NoTransforms {
        return Ok(image);
    }
    Ok(Arc::new(Image::View(ImageView {
        image,
        region,
        orientation,
    })))
}

/// The region a URL's `xywh` media fragment selects from an image of `natural` size, clipped
/// to the image (Media Fragments 1 §6.1.2). A fragment that is not a list of name-value
/// pairs, a malformed spatial value, or a region the image cannot have is an error (§6.2.3,
/// §6.3.3); other dimensions do not apply to still images (§6.3.1).
pub(crate) fn fragment_region(
    url: &str,
    natural: NaturalDimensions,
) -> Result<Option<Rect>, InvalidFragment> {
    let Some((_, fragment)) = url.split_once(FRAGMENT) else {
        return Ok(None);
    };
    let invalid = || InvalidFragment(url.to_owned());
    let mut spatial = None;
    for pair in fragment.split(PAIR_SEPARATOR) {
        let (name, value) = pair.split_once(VALUE_SEPARATOR).ok_or_else(invalid)?;
        let decode = |text: &str| {
            percent_encoding::percent_decode_str(text)
                .decode_utf8()
                .map(std::borrow::Cow::into_owned)
                .map_err(|_| invalid())
        };
        if decode(name)? == SPATIAL {
            spatial = Some(decode(value)?);
        }
    }
    let Some(spatial) = spatial else {
        return Ok(None);
    };
    let (percent, coordinates) = match spatial.split_once(UNIT_SEPARATOR) {
        Some((PIXEL_UNIT, coordinates)) => (false, coordinates),
        Some((PERCENT_UNIT, coordinates)) => (true, coordinates),
        Some(_) => return Err(invalid()),
        None => (false, spatial.as_str()),
    };
    let values = coordinates
        .split(COORDINATE_SEPARATOR)
        .map(|digits| {
            (!digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit()))
                .then(|| digits.parse::<f32>().ok())
                .flatten()
                .ok_or_else(invalid)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let [x, y, width, height] = <[f32; COORDINATES]>::try_from(values).map_err(|_| invalid())?;
    if width <= 0.0
        || height <= 0.0
        || (percent && [x, y, width, height].iter().any(|v| *v > PERCENT))
    {
        return Err(invalid());
    }
    let (Some(image_width), Some(image_height)) = (natural.width, natural.height) else {
        return Err(invalid());
    };
    let [x, y, width, height] = if percent {
        [
            (x * image_width / PERCENT).floor(),
            (y * image_height / PERCENT).floor(),
            (width * image_width / PERCENT).ceil(),
            (height * image_height / PERCENT).ceil(),
        ]
    } else {
        [x, y, width, height]
    };
    if x >= image_width || y >= image_height {
        return Err(invalid());
    }
    Ok(Some(Rect {
        x,
        y,
        width: width.min(image_width - x),
        height: height.min(image_height - y),
    }))
}

/// Clockwise quarter turns, then whether the result is flipped horizontally.
pub(crate) fn turns_and_flip(orientation: Orientation) -> (u8, bool) {
    match orientation {
        Orientation::NoTransforms => (0, false),
        Orientation::Rotate90 => (1, false),
        Orientation::Rotate180 => (2, false),
        Orientation::Rotate270 => (3, false),
        Orientation::FlipHorizontal => (0, true),
        Orientation::Rotate90FlipH => (1, true),
        Orientation::FlipVertical => (2, true),
        Orientation::Rotate270FlipH => (3, true),
    }
}
/// Whether `orientation` swaps width and height.
pub(crate) fn transposes(orientation: Orientation) -> bool {
    turns_and_flip(orientation).0 % 2 == 1
}
