//! Images as an `<image>` uses them: the region a media fragment selects, re-oriented for the
//! element (CSS Images 4 §2.5).
use crate::images::{Image, NaturalDimensions, Orientation};
use crate::paint::{BasicShape, ImageRequest, POINTS_PER_PIXEL, Rect};
use crate::style::{ImageOrientation, Style};
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
const QUARTER_TURNS: u8 = 4;
/// One image pixel per CSS pixel.
const UNIT_DENSITY: [f32; 2] = [1.0; 2];

/// A fragment identifier `image()` cannot use (CSS Images 4 §2.5.2).
#[derive(Debug, thiserror::Error)]
#[error("unsupported image fragment identifier in {0}")]
pub struct InvalidFragment(pub String);

/// Part of an image at a resolution, turned and flipped after the image's own orientation.
#[derive(Debug)]
pub struct ImageView {
    pub image: Arc<Image>,
    /// The selected region in the image's natural pixels.
    pub region: Option<Rect>,
    pub orientation: Orientation,
    /// Image pixels per CSS pixel along the region's axes (CSS Images 4 §6.1).
    pub density: [f32; 2],
}
impl ImageView {
    pub(crate) fn natural_size(&self) -> NaturalDimensions {
        let natural = self.image.natural_size();
        let [x, y] = self.density;
        let size = match self.region {
            Some(region) => NaturalDimensions {
                width: Some(region.width / x),
                height: Some(region.height / y),
                ratio: (region.height > 0.0).then(|| region.width * y / (region.height * x)),
            },
            None => NaturalDimensions {
                width: natural.width.map(|width| width / x),
                height: natural.height.map(|height| height / y),
                ratio: natural.ratio.map(|ratio| ratio * y / x),
            },
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
        let [width, height] = [width / self.density[0], height / self.density[1]];
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

/// `image` as `request` uses it in a decorative image of the element `style` belongs to: the
/// region its spatial media fragment selects, oriented by `image-orientation`, and flipped in
/// the inline direction when its directionality is the element's opposite.
pub(crate) fn view(
    image: Arc<Image>,
    request: &ImageRequest<'_>,
    style: &Style,
) -> Result<Arc<Image>, InvalidFragment> {
    let (region, inline_flip) = requested(&image, request, style)?;
    let orientation = compose(
        reorientation(&image, style.image_orientation, false),
        inline_flip,
    );
    let density = if request.resolved {
        UNIT_DENSITY
    } else {
        preferred_density(&image, style)
    };
    Ok(with_view(image, region, orientation, density))
}

/// `image` as `request` uses it in the `content` of the element `style` belongs to: as
/// [`view`] selects and flips it, then as a content image, at the `resolution` `image-set()`
/// chose for it, if any.
pub(crate) fn content_view(
    image: Arc<Image>,
    request: &ImageRequest<'_>,
    style: &Style,
    resolution: Option<f32>,
) -> Result<Arc<Image>, InvalidFragment> {
    let (region, inline_flip) = requested(&image, request, style)?;
    let preferred = resolution.map_or_else(|| preferred_density(&image, style), |dppx| [dppx; 2]);
    Ok(oriented_content(
        image,
        region,
        inline_flip,
        style,
        preferred,
        UNIT_DENSITY[0],
    ))
}

/// A content image of the element `style` belongs to: oriented by `image-orientation`, at the
/// resolution `image-resolution` prefers unless its source chose a pixel `density`, then
/// showing its `object-view-box`.
pub(crate) fn content_image(image: Arc<Image>, style: &Style, density: Option<f32>) -> Arc<Image> {
    let preferred = match density {
        Some(_) => UNIT_DENSITY,
        None => preferred_density(&image, style),
    };
    oriented_content(
        image,
        None,
        Orientation::NoTransforms,
        style,
        preferred,
        density.unwrap_or(UNIT_DENSITY[0]),
    )
}

/// The `region` of a content image turned by `image-orientation` then `inline_flip`, at
/// `preferred` image pixels per CSS pixel, showing its `object-view-box`; `density` is the
/// pixel density its replaced box divides its natural size by.
fn oriented_content(
    image: Arc<Image>,
    region: Option<Rect>,
    inline_flip: Orientation,
    style: &Style,
    preferred: [f32; 2],
    density: f32,
) -> Arc<Image> {
    let orientation = compose(
        reorientation(&image, style.image_orientation, true),
        inline_flip,
    );
    let oriented = with_view(image, region, orientation, preferred);
    view_box(oriented, style.object_view_box.as_ref(), density)
}

/// The region a request's media fragment selects, and the inline flip `image()`'s
/// directionality asks of the element `style` belongs to (CSS Images 4 §2.5.2, §2.5.4). A
/// `url()` ignores a fragment it cannot use; `image()` is then invalid.
fn requested(
    image: &Image,
    request: &ImageRequest<'_>,
    style: &Style,
) -> Result<(Option<Rect>, Orientation), InvalidFragment> {
    let region = match fragment_region(request.url, image.natural_size()) {
        Ok(region) => region,
        Err(error) if request.function.is_some() => return Err(error),
        Err(_) => None,
    };
    let flipped = request
        .function
        .and_then(|function| function.rtl)
        .is_some_and(|image_rtl| image_rtl != style.rtl);
    let inline_flip = match (flipped, style.used_writing_mode().vertical()) {
        (false, _) => Orientation::NoTransforms,
        (true, false) => Orientation::FlipHorizontal,
        (true, true) => Orientation::FlipVertical,
    };
    Ok((region, inline_flip))
}

/// CSS Images 4 §6.1: `image-resolution` sets the natural resolution of raster images, from
/// what the image records when it asks for `from-image`; it has no effect on vector images.
/// A resolution set by `image-set()` or `srcset` is the image's own and wins (§6.1 issue 1).
fn preferred_density(image: &Image, style: &Style) -> [f32; 2] {
    let recorded = match image {
        Image::Raster(raster) => raster.density.map(|[x, y]| {
            if transposes(raster.orientation) {
                [y, x]
            } else {
                [x, y]
            }
        }),
        Image::Pixels(_) => None,
        _ => return UNIT_DENSITY,
    };
    style.image_resolution.density(recorded)
}

/// CSS Images 5 §3.1: the view box resolves against the natural size, which it replaces; the
/// image draws scaled and translated so that its content keeps its place in the view box. It
/// has no effect on an image without both a natural width and height. The view box is the
/// rectangle's bounds, as an SVG `viewBox` is, so rounding its corners does not clip it.
fn view_box(image: Arc<Image>, shape: Option<&BasicShape>, density: f32) -> Arc<Image> {
    let natural = image.natural_size();
    let (Some(shape), Some(width), Some(height)) = (shape, natural.width, natural.height) else {
        return image;
    };
    // The shape's lengths are points; the region is in the image's natural pixels.
    let points = POINTS_PER_PIXEL / density;
    let region = shape
        .rectangle(Rect {
            x: 0.0,
            y: 0.0,
            width: width * points,
            height: height * points,
        })
        .map(|rect| Rect {
            x: rect.x / points,
            y: rect.y / points,
            width: rect.width / points,
            height: rect.height / points,
        });
    with_view(image, region, Orientation::NoTransforms, UNIT_DENSITY)
}

fn with_view(
    image: Arc<Image>,
    region: Option<Rect>,
    orientation: Orientation,
    density: [f32; 2],
) -> Arc<Image> {
    if region.is_none() && orientation == Orientation::NoTransforms && density == UNIT_DENSITY {
        return image;
    }
    Arc::new(Image::View(ImageView {
        image,
        region,
        orientation,
        density,
    }))
}

/// What turns `image`, as its metadata orients it, to the orientation `property` gives it
/// (CSS Images 3 §5.1): `none` undoes the metadata's orientation, and an angle or `flip`
/// replaces it in content images only. Images without orientation metadata are oriented as
/// encoded.
fn reorientation(image: &Image, property: ImageOrientation, content: bool) -> Orientation {
    let metadata = match image {
        Image::Raster(raster) => raster.orientation,
        _ => Orientation::NoTransforms,
    };
    let target = match property {
        ImageOrientation::None => Orientation::NoTransforms,
        ImageOrientation::Turned { turns, flip } if content => from_turns_and_flip(turns, flip),
        ImageOrientation::FromImage | ImageOrientation::Turned { .. } => metadata,
    };
    compose(inverse(metadata), target)
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
fn from_turns_and_flip(turns: u8, flip: bool) -> Orientation {
    match (turns % QUARTER_TURNS, flip) {
        (0, false) => Orientation::NoTransforms,
        (1, false) => Orientation::Rotate90,
        (2, false) => Orientation::Rotate180,
        (3, false) => Orientation::Rotate270,
        (0, true) => Orientation::FlipHorizontal,
        (1, true) => Orientation::Rotate90FlipH,
        (2, true) => Orientation::FlipVertical,
        _ => Orientation::Rotate270FlipH,
    }
}
/// Whether `orientation` swaps width and height.
pub(crate) fn transposes(orientation: Orientation) -> bool {
    turns_and_flip(orientation).0 % 2 == 1
}
/// `first` followed by `then`: a flip reverses the sense of the turns after it.
fn compose(first: Orientation, then: Orientation) -> Orientation {
    let (first_turns, first_flip) = turns_and_flip(first);
    let (then_turns, then_flip) = turns_and_flip(then);
    let then_turns = if first_flip {
        QUARTER_TURNS - then_turns
    } else {
        then_turns
    };
    from_turns_and_flip(first_turns + then_turns, first_flip != then_flip)
}
/// The orientation that undoes `orientation`; a turn followed by a flip undoes itself.
fn inverse(orientation: Orientation) -> Orientation {
    match turns_and_flip(orientation) {
        (turns, false) => from_turns_and_flip(QUARTER_TURNS - turns, false),
        (turns, true) => from_turns_and_flip(turns, true),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [Orientation; 8] = [
        Orientation::NoTransforms,
        Orientation::Rotate90,
        Orientation::Rotate180,
        Orientation::Rotate270,
        Orientation::FlipHorizontal,
        Orientation::FlipVertical,
        Orientation::Rotate90FlipH,
        Orientation::Rotate270FlipH,
    ];

    /// A 3×2 grid of distinct cells after each orientation in turn, with its size.
    fn applied(orientations: &[Orientation]) -> (Vec<u8>, [u32; 2]) {
        let mut grid = image::DynamicImage::ImageLuma8(
            image::GrayImage::from_raw(3, 2, vec![1, 2, 3, 4, 5, 6]).expect("six cells"),
        );
        for orientation in orientations {
            grid.apply_orientation(*orientation);
        }
        let size = [grid.width(), grid.height()];
        (grid.into_luma8().into_raw(), size)
    }

    #[test]
    fn composition_and_inversion_match_applying_orientations_in_turn() {
        for first in ALL {
            for then in ALL {
                assert_eq!(
                    applied(&[compose(first, then)]),
                    applied(&[first, then]),
                    "{first:?} then {then:?} composed to {:?}",
                    compose(first, then)
                );
            }
            assert_eq!(
                applied(&[first, inverse(first)]),
                applied(&[]),
                "{first:?} is not undone by {:?}",
                inverse(first)
            );
            assert_eq!(
                transposes(first),
                applied(&[first]).1 == [2, 3],
                "{first:?} transposition"
            );
        }
    }
}
