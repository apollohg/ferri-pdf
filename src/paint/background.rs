//! CSS Backgrounds 3 layers: computed value lists, image sizing, tiling and painting areas.
use crate::paint::gradient::{Gradient, ResolvedGradient};
use crate::paint::{
    AbsoluteColor, Border, BoxPaint, Clip, Color, LengthPercentage, POINTS_PER_PIXEL, Rect,
};
use crate::paint::{ImageInfo, NaturalDimensions};
use serde::Serialize;
use std::convert::Infallible;
use std::sync::Arc;

/// Tolerance for `space` counting tiles that fit exactly.
const SPACE_FIT_TOLERANCE: f32 = 1.0e-4;
pub(crate) const INITIAL_IMAGE_SCALE: f32 = 1.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[expect(
    clippy::enum_variant_names,
    reason = "variants name the CSS border-box, padding-box and content-box keywords"
)]
pub enum BoxArea {
    #[default]
    BorderBox,
    PaddingBox,
    ContentBox,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[expect(
    clippy::enum_variant_names,
    reason = "variants name the CSS repeat and no-repeat keywords"
)]
pub enum Repeat {
    #[default]
    Repeat,
    Space,
    Round,
    NoRepeat,
}
/// `local` behaves as `scroll` because paged boxes never scroll.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub enum Attachment {
    #[default]
    Scroll,
    Fixed,
    Local,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub enum SizeComponent {
    Auto,
    Value(LengthPercentage),
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub enum BackgroundSize {
    Cover,
    Contain,
    Explicit([SizeComponent; 2]),
}
impl Default for BackgroundSize {
    fn default() -> Self {
        Self::Explicit([const { SizeComponent::Auto }; 2])
    }
}
/// One axis of a `<position>`: an offset from the start edge or from the end edge.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PositionComponent {
    pub from_end: bool,
    pub offset: LengthPercentage,
}
impl PositionComponent {
    /// The initial `background-position` component, `0%`.
    pub const START: Self = Self {
        from_end: false,
        offset: LengthPercentage::Percentage(0.0),
    };
    /// Center alignment, initially used by object-position and gradient positions.
    pub const CENTER: Self = Self {
        from_end: false,
        offset: LengthPercentage::Percentage(0.5),
    };
    /// Top-left offsets with explicit length and percentage terms for computed-value comparisons.
    #[must_use]
    pub fn canonical(&self) -> Self {
        use crate::paint::Calculation;
        let offset = match &self.offset {
            LengthPercentage::Length(value) => Calculation::Value(*value),
            LengthPercentage::Percentage(value) => Calculation::Percentage(*value),
            LengthPercentage::Calc(value) => (**value).clone(),
        };
        let offset = if self.from_end {
            Calculation::Sum(vec![Calculation::Percentage(1.0), offset.negate()]).simplify()
        } else {
            offset
        };
        Self {
            from_end: false,
            offset: Calculation::Sum(vec![
                Calculation::Value(0.0),
                Calculation::Percentage(0.0),
                offset,
            ])
            .into(),
        }
    }
    /// Offset within `free` space; percentages align that point of the object and the area.
    pub fn resolve(&self, free: f32) -> f32 {
        let offset = self.offset.resolve(free);
        if self.from_end { free - offset } else { offset }
    }
}
/// Horizontal and vertical components.
pub(crate) type Position = [PositionComponent; 2];

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(bound(serialize = ""))]
pub enum BackgroundImage<I = Infallible> {
    None,
    /// Absolute reference that the document builder loads into `Image`; `resolution` is a
    /// raster image's natural resolution in dots per CSS pixel (CSS Images 4 §2.4).
    Url {
        url: String,
        resolution: f32,
        /// Accumulated zoom of the image's natural dimensions, independent of density.
        scale: f32,
        /// The `image()` notation that wrote the URL, if any.
        #[serde(skip_serializing_if = "Option::is_none")]
        function: Option<ImageFunction>,
    },
    Image {
        #[serde(skip)]
        image: I,
        resolution: f32,
        scale: f32,
    },
    Gradient(Gradient),
    /// A solid-color image with no natural dimensions (CSS Images 4 §2.5.3).
    Color(AbsoluteColor),
    /// An invalid image (CSS Images 4 §2.1), which renders as a transparent image with no
    /// natural dimensions unless its context treats it as `none`.
    Invalid,
}

/// The annotations `image()` adds to a URL (CSS Images 4 §2.5).
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct ImageFunction {
    /// `rtl` or `ltr`: the image's directionality, if it has one (§2.5.4).
    pub rtl: Option<bool>,
    /// The solid color rendered when the URL is an invalid image (§2.5.1).
    pub fallback: Option<AbsoluteColor>,
}

/// An image URL to load for a box.
#[derive(Clone, Copy, Debug)]
pub struct ImageRequest<'a> {
    pub url: &'a str,
    /// Written in `image()`: the image is optional, and a fragment it does not understand
    /// makes it invalid (CSS Images 4 §2.5.2).
    pub function: Option<&'a ImageFunction>,
}

/// Computed background longhands; layer `i` uses item `i` of each list, cycling short lists.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(bound(serialize = ""))]
pub struct Backgrounds<I = Infallible> {
    pub images: Vec<BackgroundImage<I>>,
    /// `background-position-x` and `background-position-y`, which cycle independently.
    pub positions: [Vec<PositionComponent>; 2],
    pub sizes: Vec<BackgroundSize>,
    pub repeats: Vec<[Repeat; 2]>,
    pub origins: Vec<BoxArea>,
    pub clips: Vec<BoxArea>,
    pub attachments: Vec<Attachment>,
    pub blend_modes: Vec<crate::paint::BlendMode>,
}
impl<I> Default for Backgrounds<I> {
    fn default() -> Self {
        Self {
            images: vec![BackgroundImage::None],
            positions: [
                vec![PositionComponent::START],
                vec![PositionComponent::START],
            ],
            sizes: vec![BackgroundSize::default()],
            repeats: vec![[Repeat::Repeat; 2]],
            origins: vec![BoxArea::PaddingBox],
            clips: vec![BoxArea::BorderBox],
            attachments: vec![Attachment::Scroll],
            blend_modes: vec![crate::paint::BlendMode::Normal],
        }
    }
}
fn cycle<T>(values: &[T], index: usize) -> &T {
    &values[index % values.len()]
}
impl<I> Backgrounds<I> {
    pub fn has_images(&self) -> bool {
        self.images
            .iter()
            .any(|image| !matches!(image, BackgroundImage::None))
    }
    /// The background colour is clipped like the bottom layer.
    pub fn color_clip(&self) -> BoxArea {
        *cycle(&self.clips, self.images.len() - 1)
    }
    /// Treats `fixed` layers as `scroll`, as in a transformed box or its descendants
    /// (CSS Transforms 1 §6).
    pub fn scroll_fixed(backgrounds: &mut Arc<Self>)
    where
        I: Clone,
    {
        if backgrounds.attachments.contains(&Attachment::Fixed) {
            for attachment in &mut Arc::make_mut(backgrounds).attachments {
                if *attachment == Attachment::Fixed {
                    *attachment = Attachment::Scroll;
                }
            }
        }
    }
    /// Replaces every `Url` image with the image `load` returns for it, copying shared lists
    /// only when one is present.
    pub(crate) fn resolve_images<E>(
        backgrounds: &mut Arc<Self>,
        mut load: impl FnMut(&ImageRequest<'_>) -> Result<Option<I>, E>,
    ) -> Result<(), E>
    where
        I: Clone,
    {
        if !backgrounds
            .images
            .iter()
            .any(|image| matches!(image, BackgroundImage::Url { .. }))
        {
            return Ok(());
        }
        for image in &mut Arc::make_mut(backgrounds).images {
            image.load(&mut load)?;
        }
        Ok(())
    }
}
impl<I> BackgroundImage<I> {
    /// Neither `none` nor an invalid image, which contexts with error handling treat as
    /// `none` (CSS Images 4 §2.1).
    pub fn is_displayable(&self) -> bool {
        !matches!(self, Self::None | Self::Invalid)
    }
    /// Replaces a `Url` with the image `load` returns for it, which is `None` when it could
    /// not be loaded without aborting the document: an `image()` URL then renders its
    /// fallback color, else the image is invalid (CSS Images 4 §2.5.1).
    pub(crate) fn load<E>(
        &mut self,
        load: &mut impl FnMut(&ImageRequest<'_>) -> Result<Option<I>, E>,
    ) -> Result<(), E> {
        if let Self::Url {
            url,
            resolution,
            scale,
            function,
        } = self
        {
            let request = ImageRequest {
                url,
                function: function.as_ref(),
            };
            *self = match load(&request)? {
                Some(image) => Self::Image {
                    image,
                    resolution: *resolution,
                    scale: *scale,
                },
                None => function
                    .and_then(|function| function.fallback)
                    .map_or(Self::Invalid, Self::Color),
            };
        }
        Ok(())
    }
}

impl Backgrounds<Infallible> {
    pub fn with_image_type<I>(&self) -> Backgrounds<I> {
        Backgrounds {
            images: self
                .images
                .iter()
                .map(BackgroundImage::with_image_type)
                .collect(),
            positions: self.positions.clone(),
            sizes: self.sizes.clone(),
            repeats: self.repeats.clone(),
            origins: self.origins.clone(),
            clips: self.clips.clone(),
            attachments: self.attachments.clone(),
            blend_modes: self.blend_modes.clone(),
        }
    }
}

impl BackgroundImage<Infallible> {
    pub fn with_image_type<I>(&self) -> BackgroundImage<I> {
        match self {
            Self::None => BackgroundImage::None,
            Self::Url {
                url,
                resolution,
                scale,
                function,
            } => BackgroundImage::Url {
                url: url.clone(),
                resolution: *resolution,
                scale: *scale,
                function: *function,
            },
            Self::Gradient(gradient) => BackgroundImage::Gradient(gradient.clone()),
            Self::Color(color) => BackgroundImage::Color(*color),
            Self::Invalid => BackgroundImage::Invalid,
            Self::Image { image, .. } => match *image {},
        }
    }
}

/// A `url()` image that reached painting without being loaded.
#[derive(Debug)]
pub struct UnresolvedImage(pub String);
impl std::fmt::Display for UnresolvedImage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "background image {} was not loaded", self.0)
    }
}
impl std::error::Error for UnresolvedImage {}

pub(crate) enum Source<'a, I> {
    /// A loaded image, natural resolution in dots per CSS pixel, and used size scale.
    Image(&'a I, f32, f32),
    Gradient(&'a Gradient),
    Color(Color),
}

impl<'a, I: ImageInfo> Source<'a, I> {
    /// The source an image paints from, `None` for `none`.
    pub(crate) fn of(image: &'a BackgroundImage<I>) -> Result<Option<Self>, UnresolvedImage> {
        Ok(Some(match image {
            BackgroundImage::None => return Ok(None),
            BackgroundImage::Url { url, .. } => return Err(UnresolvedImage(url.clone())),
            BackgroundImage::Image {
                image,
                resolution,
                scale,
            } => Self::Image(image, *resolution, *scale),
            BackgroundImage::Gradient(gradient) => Self::Gradient(gradient),
            BackgroundImage::Color(color) => Self::Color(color.used()),
            BackgroundImage::Invalid => Self::Color(Color::TRANSPARENT),
        }))
    }
    /// Natural dimensions in points; generated images have none.
    pub(crate) fn natural(self) -> NaturalDimensions {
        match self {
            Self::Image(image, resolution, scale) => natural_points(image, resolution, scale),
            Self::Gradient(_) | Self::Color(_) => NaturalDimensions::default(),
        }
    }
    /// The image drawn into a tile of `size`, `None` when a gradient has no area.
    pub(crate) fn tile(self, size: [f32; 2]) -> Option<TileImage<'a, I>> {
        Some(match self {
            Self::Image(image, _, scale) => TileImage::Image(image, scale),
            Self::Gradient(gradient) => TileImage::Gradient(gradient.resolve(size)?),
            Self::Color(color) => TileImage::Color(color),
        })
    }
}

impl<I> Copy for Source<'_, I> {}
impl<I> Clone for Source<'_, I> {
    fn clone(&self) -> Self {
        *self
    }
}

/// Content of one tile.
#[derive(Clone, Debug)]
pub enum TileImage<'a, I = Infallible> {
    Image(&'a I, f32),
    Gradient(ResolvedGradient),
    /// A solid-color image.
    Color(Color),
}
/// One background layer ready to paint.
#[derive(Clone, Debug)]
pub struct LayerPaint<'a, I = Infallible> {
    pub image: TileImage<'a, I>,
    /// Background painting area.
    pub clip: Clip,
    /// The tile placed by `background-position`; repeated axes tile from it both ways.
    pub tile: Rect,
    /// Distance between tile origins, including `space` gaps.
    pub step: [f32; 2],
    pub repeat: [bool; 2],
    /// How the layer blends with the layers and colour below it (Compositing 1 §3.4.3).
    pub blend: crate::paint::BlendMode,
}

/// Natural dimensions in points; a raster image's pixels span `1 / resolution` CSS pixels.
pub(crate) fn natural_points<I: ImageInfo>(
    image: &I,
    resolution: f32,
    zoom: f32,
) -> NaturalDimensions {
    let natural = image.natural_size();
    let scale = if image.is_raster() {
        POINTS_PER_PIXEL / resolution
    } else {
        POINTS_PER_PIXEL
    } * zoom;
    NaturalDimensions {
        width: natural.width.map(|width| width * scale),
        height: natural.height.map(|height| height * scale),
        ratio: natural.ratio,
    }
}

fn contain(area: [f32; 2], ratio: f32, cover: bool) -> [f32; 2] {
    let width_limited = area[0] / ratio <= area[1];
    if width_limited == cover {
        [area[1] * ratio, area[1]]
    } else {
        [area[0], area[0] / ratio]
    }
}

/// `background-size` and the CSS Images 3 default sizing algorithm, before `round`;
/// natural dimensions are in points.
pub(crate) fn image_size(
    size: &BackgroundSize,
    natural: NaturalDimensions,
    area: [f32; 2],
) -> [f32; 2] {
    let NaturalDimensions {
        width,
        height,
        ratio,
    } = natural;
    let resolve = |component: &SizeComponent, basis: f32| match component {
        SizeComponent::Auto => None,
        SizeComponent::Value(value) => Some(value.resolve(basis)),
    };
    match size {
        BackgroundSize::Cover | BackgroundSize::Contain => ratio.map_or(area, |ratio| {
            contain(area, ratio, *size == BackgroundSize::Cover)
        }),
        BackgroundSize::Explicit([x, y]) => match (resolve(x, area[0]), resolve(y, area[1])) {
            (Some(x), Some(y)) => [x, y],
            (Some(x), None) => [x, ratio.map(|r| x / r).or(height).unwrap_or(area[1])],
            (None, Some(y)) => [ratio.map(|r| y * r).or(width).unwrap_or(area[0]), y],
            (None, None) => match (width, height, ratio) {
                (Some(x), Some(y), _) => [x, y],
                (Some(x), None, Some(r)) => [x, x / r],
                (None, Some(y), Some(r)) => [y * r, y],
                (Some(x), None, None) => [x, area[1]],
                (None, Some(y), None) => [area[0], y],
                (None, None, Some(r)) => contain(area, r, false),
                (None, None, None) => area,
            },
        },
    }
}

/// Second and third `round` steps of CSS Backgrounds 3 §3.9.
pub(crate) fn rounded_size(
    mut tile: [f32; 2],
    size: &BackgroundSize,
    repeat: [Repeat; 2],
    area: [f32; 2],
) -> [f32; 2] {
    let original = tile;
    for axis in 0..2 {
        if repeat[axis] == Repeat::Round && tile[axis] > 0.0 {
            tile[axis] = area[axis] / (area[axis] / tile[axis]).round().max(1.0);
        }
    }
    if let BackgroundSize::Explicit(components) = size {
        for axis in 0..2 {
            let other = 1 - axis;
            if repeat[axis] == Repeat::Round
                && repeat[other] != Repeat::Round
                && components[other] == SizeComponent::Auto
                && original[axis] > 0.0
            {
                tile[other] = original[other] * tile[axis] / original[axis];
            }
        }
    }
    tile
}

/// Tile origin, pitch and repetition on one axis.
fn place(
    repeat: Repeat,
    position: &PositionComponent,
    start: f32,
    area: f32,
    tile: f32,
) -> (f32, f32, bool) {
    if repeat == Repeat::Space {
        let count = (area / tile + SPACE_FIT_TOLERANCE).floor();
        if count >= 2.0 {
            return (start, tile + (area - count * tile) / (count - 1.0), true);
        }
    }
    (
        start + position.resolve(area - tile),
        tile,
        matches!(repeat, Repeat::Repeat | Repeat::Round),
    )
}

impl<I> BoxPaint<I> {
    /// Padding without the sliced edges of a fragment.
    pub(super) fn fragment_padding(&self) -> [f32; 4] {
        let mut padding = self.padding;
        if self.assembled_box().is_some() {
            return padding;
        }
        for (side, sliced) in padding.iter_mut().zip(self.sliced()) {
            if sliced {
                *side = 0.0;
            }
        }
        padding
    }
    /// The border, padding or content box of this fragment with its curves.
    pub fn box_area(&self, area: BoxArea) -> Clip {
        match area {
            BoxArea::BorderBox => self.box_clip([0.0; 4], [true; 2]),
            BoxArea::PaddingBox => self.inner_clip([0.0; 4], [true; 2]),
            BoxArea::ContentBox => self.inner_clip(self.fragment_padding(), [true; 2]),
        }
    }
    pub fn background_clips(&self) -> crate::paint::BoxAreas<Clip> {
        crate::paint::BoxAreas {
            border: self.box_area(BoxArea::BorderBox),
            padding: self.box_area(BoxArea::PaddingBox),
            content: self.box_area(BoxArea::ContentBox),
        }
    }
    pub fn background_origins(&self) -> crate::paint::BoxAreas<Rect> {
        crate::paint::BoxAreas {
            border: self.positioning_area(BoxArea::BorderBox),
            padding: self.positioning_area(BoxArea::PaddingBox),
            content: self.positioning_area(BoxArea::ContentBox),
        }
    }
    pub(crate) fn background_clip(&self, area: BoxArea) -> Clip {
        self.background_geometry
            .as_ref()
            .map_or_else(|| self.box_area(area), |geometry| geometry.clips.area(area))
    }
    /// Positioning area, taken from `background_box` when another box positions the layers.
    fn positioning_area(&self, area: BoxArea) -> Rect {
        if let Some(origins) = self
            .background_geometry
            .as_ref()
            .and_then(|geometry| geometry.origins)
        {
            return origins.area(area);
        }
        let Some(outer) = self.background_box else {
            return self.box_area(area).rect;
        };
        let borders = self.decoration.borders.map(Border::used_width);
        let insets: [f32; 4] = match area {
            BoxArea::BorderBox => [0.0; 4],
            BoxArea::PaddingBox => borders,
            BoxArea::ContentBox => std::array::from_fn(|side| borders[side] + self.padding[side]),
        };
        Rect {
            x: outer.x + insets[3],
            y: outer.y + insets[0],
            width: outer.width - insets[1] - insets[3],
            height: outer.height - insets[0] - insets[2],
        }
    }
}

impl<I: ImageInfo> BoxPaint<I> {
    /// Image layers bottom-most first; `page_area` positions `fixed` layers.
    pub fn background_layers(
        &self,
        page_area: Rect,
    ) -> Result<Vec<LayerPaint<'_, I>>, UnresolvedImage> {
        if !self.draw_background {
            return Ok(Vec::new());
        }
        Ok(self
            .layers(&self.decoration.backgrounds, page_area)?
            .into_iter()
            .map(|(_, layer)| layer)
            .collect())
    }
    /// Mask image layers bottom-most first with their list indices, placed and clipped like
    /// background layers (CSS Masking 1 §7).
    pub fn mask_layers(
        &self,
        page_area: Rect,
    ) -> Result<Vec<(usize, LayerPaint<'_, I>)>, UnresolvedImage> {
        self.layers(&self.decoration.masks.layers, page_area)
    }
    fn layers<'a>(
        &'a self,
        backgrounds: &'a Backgrounds<I>,
        page_area: Rect,
    ) -> Result<Vec<(usize, LayerPaint<'a, I>)>, UnresolvedImage> {
        let mut layers = Vec::new();
        let page_area = self
            .background_geometry
            .as_ref()
            .and_then(|geometry| geometry.viewport)
            .unwrap_or(page_area);
        for (index, image) in backgrounds.images.iter().enumerate().rev() {
            let Some(source) = Source::of(image)? else {
                continue;
            };
            let natural = source.natural();
            let area = if *cycle(&backgrounds.attachments, index) == Attachment::Fixed {
                page_area
            } else {
                self.positioning_area(*cycle(&backgrounds.origins, index))
            };
            let extent = [area.width, area.height];
            let size = cycle(&backgrounds.sizes, index);
            let repeat = *cycle(&backgrounds.repeats, index);
            let tile = rounded_size(image_size(size, natural, extent), size, repeat, extent);
            if !(tile[0] > 0.0 && tile[1] > 0.0 && tile.iter().all(|v| v.is_finite())) {
                continue;
            }
            let [position_x, position_y] = &backgrounds.positions;
            let (x, step_x, repeat_x) = place(
                repeat[0],
                cycle(position_x, index),
                area.x,
                extent[0],
                tile[0],
            );
            let (y, step_y, repeat_y) = place(
                repeat[1],
                cycle(position_y, index),
                area.y,
                extent[1],
                tile[1],
            );
            let Some(image) = source.tile(tile) else {
                continue;
            };
            layers.push((
                index,
                LayerPaint {
                    image,
                    clip: self.background_clip(*cycle(&backgrounds.clips, index)),
                    tile: Rect {
                        x,
                        y,
                        width: tile[0],
                        height: tile[1],
                    },
                    step: [step_x, step_y],
                    repeat: [repeat_x, repeat_y],
                    blend: *cycle(&backgrounds.blend_modes, index),
                },
            ));
        }
        Ok(layers)
    }
}
