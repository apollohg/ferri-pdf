use crate::paint::rounded::Radii;
use crate::paint::{
    Backgrounds, BorderImage, BoxShadow, Clip, Color, Contour, Fill, LengthPercentage, Point, Rect,
    Segment, Shape,
};
use serde::Serialize;
use std::{
    convert::Infallible,
    sync::{Arc, LazyLock},
};

const DOT_SPACING_WIDTHS: f32 = 2.0;
/// Half-extent of a clip along an axis that does not clip.
pub(crate) const DASH_LENGTH_WIDTHS: f32 = 3.0;
pub(crate) const DASH_GAP_WIDTHS: f32 = 2.0;
static INITIAL_RADII: LazyLock<Arc<[[LengthPercentage; 2]; 4]>> =
    LazyLock::new(|| Arc::new(std::array::from_fn(|_| [LengthPercentage::ZERO; 2])));

/// Dot spacing fitted to a whole number of nominal intervals along `length`.
pub(crate) fn dot_gap(length: f32, width: f32) -> f32 {
    let nominal = DOT_SPACING_WIDTHS * width;
    let intervals = (length / nominal).floor();
    if intervals < 1.0 {
        nominal
    } else {
        (length / intervals).max(nominal)
    }
}
/// Gap that fits one more dash than gap along `length`, or the nominal gap when none fit.
pub(crate) fn dash_gap(length: f32, width: f32) -> f32 {
    let dash = DASH_LENGTH_WIDTHS * width;
    let nominal = DASH_GAP_WIDTHS * width;
    let gaps = ((length - dash) / (dash + nominal)).floor();
    if gaps < 1.0 {
        nominal
    } else {
        (length - (gaps + 1.0) * dash) / gaps
    }
}

pub(crate) const DEFAULT_BORDER_WIDTH: f32 = 1.6875;

/// Clockwise from the left side; painting starts after the first absent side.
const SIDE_ORDER: [usize; 4] = [3, 0, 1, 2];

/// Paint runs of present sides. Without an absent side, painting starts at the run holding the left side.
pub(crate) fn side_runs(
    widths: [f32; 4],
    joined: impl Fn(usize, usize) -> bool,
) -> Vec<Vec<usize>> {
    let start = if let Some(absent) = SIDE_ORDER.iter().find(|side| widths[**side] <= 0.0) {
        (absent + 1) % 4
    } else {
        let mut start = SIDE_ORDER[0];
        for _ in 1..SIDE_ORDER.len() {
            let previous = (start + 3) % 4;
            if !joined(previous, start) {
                break;
            }
            start = previous;
        }
        start
    };
    let mut runs: Vec<Vec<usize>> = Vec::new();
    for side in (0..4).map(|offset| (start + offset) % 4) {
        if widths[side] <= 0.0 {
            continue;
        }
        match runs.last_mut() {
            Some(run)
                if (run[run.len() - 1] + 1) % 4 == side && joined(run[run.len() - 1], side) =>
            {
                run.push(side);
            }
            _ => runs.push(vec![side]),
        }
    }
    runs
}
/// Band boundaries, as fractions of the side widths from the outer edge, of a border painted
/// as one even-odd ring: all four sides present with one colour and one solid or double style.
pub(crate) fn ring<I>(decoration: &BoxDecoration<I>, widths: [f32; 4]) -> Option<Vec<f32>> {
    let first = decoration.borders[0];
    let uniform = widths.iter().all(|width| *width > 0.0)
        && decoration
            .borders
            .iter()
            .all(|border| border.style == first.style && border.color == first.color);
    if !uniform {
        return None;
    }
    match first.style {
        BorderStyle::Solid => Some(vec![0.0, 1.0]),
        BorderStyle::Double => Some(vec![0.0, DOUBLE_LINE, 1.0 - DOUBLE_LINE, 1.0]),
        _ => None,
    }
}
/// Corners of `rect` inset by fraction `t` of each side's width.
fn ring_corners(rect: Rect, widths: [f32; 4], t: f32) -> [Point; 4] {
    let [top, right, bottom, left] = widths.map(|width| width * t);
    [
        [rect.x + left, rect.y + top],
        [rect.x + rect.width - right, rect.y + top],
        [rect.x + rect.width - right, rect.y + rect.height - bottom],
        [rect.x + left, rect.y + rect.height - bottom],
    ]
}

/// Fraction of a `double` border's width taken by each line; the gap takes the rest.
const DOUBLE_LINE: f32 = 1.0 / 3.0;
/// Fraction of a `groove` or `ridge` border's width taken by its outer half.
const GROOVE_SPLIT: f32 = 0.5;
const CHANNEL_MAX: u16 = 255;

/// A solid band of one border side between two fractions of its width from the outer edge.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Band {
    pub from: f32,
    pub to: f32,
    pub color: Color,
}

/// 3D border shading: the dark shade halves each channel and the
/// light shade mixes it halfway to white, keeping the alpha.
pub(crate) fn shade(color: Color, light: bool) -> Color {
    Color {
        rgb: color.rgb.map(|channel| {
            let base = if light { CHANNEL_MAX } else { 0 };
            (u16::from(channel) + base).div_ceil(2) as u8
        }),
        alpha: color.alpha,
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub enum BorderStyle {
    #[default]
    None,
    Hidden,
    Solid,
    Dotted,
    Dashed,
    Double,
    Groove,
    Ridge,
    Inset,
    Outset,
}
impl BorderStyle {
    pub fn suppresses_width(self) -> bool {
        matches!(self, Self::None | Self::Hidden)
    }

    /// Style drawn in the collapsing border model, where `inset` draws as `ridge` and
    /// `outset` as `groove` (CSS 2.1 §17.6.3).
    #[must_use]
    pub fn collapsed(self) -> Self {
        match self {
            Self::Inset => Self::Ridge,
            Self::Outset => Self::Groove,
            style => style,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Border {
    pub width: f32,
    pub style: BorderStyle,
    pub color: Color,
}
impl Default for Border {
    fn default() -> Self {
        Self {
            width: DEFAULT_BORDER_WIDTH,
            style: BorderStyle::None,
            color: Color::BLACK,
        }
    }
}
impl Border {
    pub fn used_width(self) -> f32 {
        if self.style.suppresses_width() {
            0.0
        } else {
            self.width
        }
    }
    /// Solid bands painting this border on `side` (top/right/bottom/left); none for the
    /// dotted, dashed and absent styles. Top and left sides take the dark shade of `inset`
    /// and of the outer half of `groove`.
    pub(crate) fn bands(self, side: usize) -> Vec<Band> {
        let top_left = side == 0 || side == 3;
        let band = |from, to, color| Band { from, to, color };
        let color = self.color;
        match self.style {
            BorderStyle::Solid => vec![band(0.0, 1.0, color)],
            BorderStyle::Double => vec![
                band(0.0, DOUBLE_LINE, color),
                band(1.0 - DOUBLE_LINE, 1.0, color),
            ],
            BorderStyle::Groove | BorderStyle::Ridge => {
                let outer_light = (self.style == BorderStyle::Ridge) == top_left;
                vec![
                    band(0.0, GROOVE_SPLIT, shade(color, outer_light)),
                    band(GROOVE_SPLIT, 1.0, shade(color, !outer_light)),
                ]
            }
            BorderStyle::Inset | BorderStyle::Outset => {
                let light = (self.style == BorderStyle::Outset) == top_left;
                vec![band(0.0, 1.0, shade(color, light))]
            }
            BorderStyle::None | BorderStyle::Hidden | BorderStyle::Dotted | BorderStyle::Dashed => {
                Vec::new()
            }
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(bound(serialize = ""))]
pub struct BoxDecoration<I = Infallible> {
    pub background: Color,
    pub backgrounds: Arc<Backgrounds<I>>,
    pub borders: [Border; 4],
    /// `[horizontal, vertical]` radii in top-left, top-right, bottom-right, bottom-left order.
    pub radii: Arc<[[LengthPercentage; 2]; 4]>,
    /// `box-shadow` layers, the first listed on top.
    pub shadows: Arc<Vec<BoxShadow>>,
    pub border_image: Arc<BorderImage<I>>,
}
impl<I> Default for BoxDecoration<I> {
    fn default() -> Self {
        Self {
            background: Color::TRANSPARENT,
            backgrounds: Arc::default(),
            borders: [Border::default(); 4],
            radii: INITIAL_RADII.clone(),
            shadows: Arc::default(),
            border_image: Arc::default(),
        }
    }
}
impl<I> BoxDecoration<I> {
    pub fn is_visible(&self) -> bool {
        self.background.alpha > 0.0
            || self.backgrounds.has_images()
            || self.has_shadows()
            || self.border_image.is_drawn()
            || self
                .borders
                .iter()
                .any(|b| b.used_width() > 0.0 && b.color.alpha > 0.0)
    }
    fn has_shadows(&self) -> bool {
        self.shadows.iter().any(|shadow| shadow.color.alpha > 0.0)
    }
    /// Replaces the `url()` background layers and border image source with loaded images.
    pub fn resolve_images<E>(&mut self, mut load: impl FnMut(&str) -> Result<I, E>) -> Result<(), E>
    where
        I: Clone,
    {
        Backgrounds::resolve_images(&mut self.backgrounds, &mut load)?;
        if matches!(
            self.border_image.source,
            crate::paint::BackgroundImage::Url { .. }
        ) {
            Arc::make_mut(&mut self.border_image)
                .source
                .load(&mut load)?;
        }
        Ok(())
    }
    /// Removes the background colour and images, as when they propagate to the canvas.
    pub fn clear_background(&mut self) {
        self.background = Color::TRANSPARENT;
        self.backgrounds = Arc::default();
    }
}

impl BoxDecoration<Infallible> {
    /// Copies computed values into an image consumer without acquiring resources.
    pub fn with_image_type<I>(&self) -> BoxDecoration<I> {
        let backgrounds = &self.backgrounds;
        let border_image = &self.border_image;
        BoxDecoration {
            background: self.background,
            backgrounds: Arc::new(Backgrounds {
                images: backgrounds
                    .images
                    .iter()
                    .map(crate::paint::BackgroundImage::with_image_type)
                    .collect(),
                positions: backgrounds.positions.clone(),
                sizes: backgrounds.sizes.clone(),
                repeats: backgrounds.repeats.clone(),
                origins: backgrounds.origins.clone(),
                clips: backgrounds.clips.clone(),
                attachments: backgrounds.attachments.clone(),
                blend_modes: backgrounds.blend_modes.clone(),
            }),
            borders: self.borders,
            radii: self.radii.clone(),
            shadows: self.shadows.clone(),
            border_image: Arc::new(BorderImage {
                source: border_image.source.with_image_type(),
                slice: border_image.slice,
                fill: border_image.fill,
                widths: border_image.widths.clone(),
                outset: border_image.outset,
                repeat: border_image.repeat,
            }),
        }
    }

    pub fn load_images<I: Clone, E>(
        &self,
        load: impl FnMut(&str) -> Result<I, E>,
    ) -> Result<BoxDecoration<I>, E> {
        let mut decoration = self.with_image_type();
        decoration.resolve_images(load)?;
        Ok(decoration)
    }
}

/// Left and right edges of a box fragment that is not sliced along the inline axis.
pub const INLINE_EDGES: [bool; 2] = [true; 2];

#[derive(Clone, Copy, Debug, Serialize)]
pub struct BoxAreas<T> {
    pub border: T,
    pub padding: T,
    pub content: T,
}
impl BoxAreas<Rect> {
    /// A box without borders or padding.
    pub(crate) fn single(rect: Rect) -> Self {
        Self {
            border: rect,
            padding: rect,
            content: rect,
        }
    }
}
impl<T: Copy> BoxAreas<T> {
    pub fn area(&self, area: crate::paint::BoxArea) -> T {
        match area {
            crate::paint::BoxArea::BorderBox => self.border,
            crate::paint::BoxArea::PaddingBox => self.padding,
            crate::paint::BoxArea::ContentBox => self.content,
        }
    }
}

/// Page and canvas backgrounds can use different positioning and clipping boxes.
#[derive(Clone, Debug, Serialize)]
pub struct BackgroundGeometry {
    pub origins: Option<BoxAreas<Rect>>,
    pub clips: BoxAreas<Clip>,
    pub viewport: Option<Rect>,
}

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde skip_serializing_if passes the field by reference"
)]
fn background_enabled(enabled: &bool) -> bool {
    *enabled
}

#[derive(Clone, Debug, Serialize)]
#[serde(bound(serialize = ""))]
pub struct BoxPaint<I = Infallible> {
    pub rect: Rect,
    pub decoration: BoxDecoration<I>,
    /// Used padding widths in top/right/bottom/left order.
    pub padding: [f32; 4],
    /// Assembled decoration box of a sliced fragment, or the table row, column or group
    /// whose background layer a cell paints.
    pub background_box: Option<Rect>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub background_geometry: Option<Arc<BackgroundGeometry>>,
    #[serde(skip_serializing_if = "background_enabled")]
    pub draw_background: bool,
    pub first_fragment: bool,
    pub last_fragment: bool,
    /// Whether the left and right edges belong to this fragment; inline boxes broken
    /// across lines are sliced there (CSS Fragmentation 3 `box-decoration-break`).
    pub inline_edges: [bool; 2],
}
impl<I> BoxPaint<I> {
    /// A box without padding that is not split across fragments.
    pub fn whole(rect: Rect, decoration: BoxDecoration<I>) -> Self {
        Self {
            rect,
            decoration,
            padding: [0.0; 4],
            background_box: None,
            background_geometry: None,
            draw_background: true,
            first_fragment: true,
            last_fragment: true,
            inline_edges: INLINE_EDGES,
        }
    }
    /// Edges sliced off this fragment, top/right/bottom/left.
    pub(crate) fn sliced(&self) -> [bool; 4] {
        [
            !self.first_fragment,
            !self.inline_edges[1],
            !self.last_fragment,
            !self.inline_edges[0],
        ]
    }
    fn widths(&self) -> [f32; 4] {
        let mut widths = self.decoration.borders.map(Border::used_width);
        for (width, sliced) in widths.iter_mut().zip(self.sliced()) {
            if sliced {
                *width = 0.0;
            }
        }
        widths
    }
    pub fn assembled_box(&self) -> Option<Rect> {
        self.background_box
            .filter(|_| self.sliced().contains(&true))
    }
    pub(crate) fn decoration_rect(&self) -> Rect {
        self.assembled_box().unwrap_or(self.rect)
    }
    fn decoration_widths(&self) -> [f32; 4] {
        if self.assembled_box().is_some() {
            self.decoration.borders.map(Border::used_width)
        } else {
            self.widths()
        }
    }
    pub(crate) fn slice_bounds(&self, coverage: Rect) -> Rect {
        let [top, right, bottom, left] = self.sliced();
        let x = if left {
            coverage.x.max(self.rect.x)
        } else {
            coverage.x
        };
        let y = if top {
            coverage.y.max(self.rect.y)
        } else {
            coverage.y
        };
        let end_x = coverage.x + coverage.width;
        let end_y = coverage.y + coverage.height;
        Rect {
            x,
            y,
            width: ((if right {
                end_x.min(self.rect.x + self.rect.width)
            } else {
                end_x
            }) - x)
                .max(0.0),
            height: ((if bottom {
                end_y.min(self.rect.y + self.rect.height)
            } else {
                end_y
            }) - y)
                .max(0.0),
        }
    }
    pub(crate) fn slice_clip(&self, mut clip: Clip) -> Clip {
        if self.assembled_box().is_some() {
            let visible = self.slice_bounds(clip.rect);
            if visible != clip.rect {
                clip.reference_box = Some(clip.rect);
                clip.rect = visible;
            }
        }
        clip
    }
    /// This fragment's border, padding and content boxes; sliced edges have neither borders
    /// nor padding.
    pub(crate) fn fragment_areas(&self) -> BoxAreas<Rect> {
        let sliced = self.sliced();
        let padding = std::array::from_fn(|side| {
            if sliced[side] {
                0.0
            } else {
                self.padding[side]
            }
        });
        let (padding_box, _) =
            crate::paint::rounded::inset(self.rect, Radii::default(), self.widths());
        let (content, _) = crate::paint::rounded::inset(padding_box, Radii::default(), padding);
        BoxAreas {
            border: self.rect,
            padding: padding_box,
            content,
        }
    }
    /// Border widths scaled down when opposite borders exceed the box.
    pub(crate) fn inner_widths(&self) -> [f32; 4] {
        let rect = self.decoration_rect();
        let widths = self.decoration_widths();
        let horizontal = (rect.width / (widths[1] + widths[3])).min(1.0);
        let vertical = (rect.height / (widths[0] + widths[2])).min(1.0);
        [
            widths[0] * vertical,
            widths[1] * horizontal,
            widths[2] * vertical,
            widths[3] * horizontal,
        ]
    }
    /// Clip inside the borders and `padding` (top/right/bottom/left) of this fragment.
    /// An axis that does not clip extends without bound and drops the corner curves.
    pub fn inner_clip(&self, padding: [f32; 4], axes: [bool; 2]) -> Clip {
        let widths = self.inner_widths();
        let (rect, mut radii) = crate::paint::rounded::inset(
            self.decoration_rect(),
            self.radii(),
            std::array::from_fn(|side| widths[side] + padding[side]),
        );
        if axes != [true; 2] {
            radii = [[0.0; 2]; 4];
        }
        self.slice_clip(Clip {
            rect,
            radii,
            reference_box: None,
            axes,
        })
    }
    pub fn has_ink(&self) -> bool {
        self.rect.width > 0.0
            && self.rect.height > 0.0
            && ((self.draw_background
                && (self.decoration.background.alpha > 0.0
                    || self.decoration.backgrounds.has_images()))
                || self.decoration.has_shadows()
                || self.decoration.border_image.is_drawn()
                || self
                    .widths()
                    .iter()
                    .zip(self.decoration.borders)
                    .any(|(width, border)| *width > 0.0 && border.color.alpha > 0.0))
    }
    /// Used corner radii of the assembled box, before cutting its fragments.
    pub fn radii(&self) -> Radii {
        let rect = self.decoration_rect();
        let sliced = self.sliced();
        let mut radii: Radii = std::array::from_fn(|corner| {
            let [horizontal, vertical] = &self.decoration.radii[corner];
            // Corner `c` joins side `c` (clockwise from the top-left corner) and the side before it.
            let open =
                self.assembled_box().is_some() || (!sliced[corner] && !sliced[(corner + 3) % 4]);
            let used = [
                horizontal.resolve(rect.width),
                vertical.resolve(rect.height),
            ];
            if open && used[0] > 0.0 && used[1] > 0.0 {
                used
            } else {
                [0.0; 2]
            }
        });
        let factor = [
            (rect.width, radii[0][0] + radii[1][0]),
            (rect.height, radii[1][1] + radii[2][1]),
            (rect.width, radii[2][0] + radii[3][0]),
            (rect.height, radii[3][1] + radii[0][1]),
        ]
        .into_iter()
        .filter(|(_, sum)| *sum > 0.0)
        .map(|(length, sum)| length / sum)
        .fold(1.0_f32, f32::min);
        if factor < 1.0 {
            for corner in &mut radii {
                *corner = corner.map(|radius| radius * factor);
            }
        }
        radii
    }
    /// Background colour within the bottom layer's `background-clip` area.
    pub fn background_fill(&self) -> Option<Fill> {
        if !self.draw_background || self.decoration.background.alpha <= 0.0 {
            return None;
        }
        let clip = self.background_clip(self.decoration.backgrounds.color_clip());
        if clip.rect.width <= 0.0 || clip.rect.height <= 0.0 {
            return None;
        }
        let mut shape = if clip
            .radii
            .iter()
            .any(|corner| corner[0] > 0.0 && corner[1] > 0.0)
        {
            Shape::Path {
                contours: vec![clip.contour()],
                even_odd: true,
                clip: None,
            }
        } else {
            Shape::Rect(clip.rect)
        };
        if let Some(bounds) = clip.fragment_bounds() {
            shape = Shape::Clipped {
                shape: Box::new(shape),
                bounds,
            };
        }
        Some(Fill {
            shape,
            color: self.decoration.background,
        })
    }
    pub fn border_fills(&self) -> Vec<Fill> {
        let mut fills = self.uncut_border_fills();
        if let Some(whole) = self.assembled_box()
            && let Some(bounds) = (Clip {
                rect: self.slice_bounds(whole),
                radii: Radii::default(),
                reference_box: Some(whole),
                axes: [true; 2],
            })
            .fragment_bounds()
        {
            for fill in &mut fills {
                let shape = std::mem::replace(&mut fill.shape, Shape::Rect(whole));
                fill.shape = Shape::Clipped {
                    shape: Box::new(shape),
                    bounds,
                };
            }
        }
        fills
    }
    fn uncut_border_fills(&self) -> Vec<Fill> {
        let rect = self.decoration_rect();
        let mut fills = Vec::new();
        if rect.width <= 0.0 || rect.height <= 0.0 {
            return fills;
        }
        let widths = self.decoration_widths();
        let inner_widths = self.inner_widths();
        let radii = self.radii();
        if radii.iter().any(|corner| corner[0] > 0.0) {
            return crate::paint::rounded::border_fills(
                rect,
                &self.decoration,
                widths,
                inner_widths,
                radii,
            );
        }
        let outer = ring_corners(rect, inner_widths, 0.0);
        let inner = ring_corners(rect, inner_widths, 1.0);
        let uniform = (self.assembled_box().is_some() || !self.sliced().contains(&true))
            && self.decoration.borders.iter().all(|border| {
                border.style == self.decoration.borders[0].style
                    && border.width == widths[0]
                    && border.color == self.decoration.borders[0].color
            })
            && widths[0] > 0.0
            && rect.width >= widths[0]
            && rect.height >= widths[0];
        if uniform && self.decoration.borders[0].style == BorderStyle::Dashed {
            let width = widths[0];
            let half = width / 2.0;
            let points = vec![
                [rect.x + half, rect.y + rect.height - half],
                [rect.x + rect.width - half, rect.y + rect.height - half],
                [rect.x + rect.width - half, rect.y + half],
                [rect.x + half, rect.y + half],
            ];
            fills.push(Fill {
                shape: Shape::Pattern {
                    contours: vec![Contour::polygon(&points)],
                    width,
                    dash: [DASH_LENGTH_WIDTHS * width, DASH_GAP_WIDTHS * width],
                    round: false,
                    clip: None,
                },
                color: self.decoration.borders[0].color,
            });
            return fills;
        }
        if let Some(boundaries) = ring(&self.decoration, widths) {
            if self.decoration.borders[0].color.alpha > 0.0 {
                fills.push(Fill {
                    shape: Shape::Path {
                        contours: boundaries
                            .into_iter()
                            .map(|t| Contour::polygon(&ring_corners(rect, inner_widths, t)))
                            .collect(),
                        even_odd: true,
                        clip: None,
                    },
                    color: self.decoration.borders[0].color,
                });
            }
            return fills;
        }
        let pattern = |side: usize| {
            let next = (side + 1) % 4;
            let border = self.decoration.borders[side];
            let width = widths[side];
            let half = width / 2.0;
            let start = outer[side];
            let end = outer[next];
            let length = (end[0] - start[0]).abs() + (end[1] - start[1]).abs();
            let tangent = [(end[0] - start[0]) / length, (end[1] - start[1]) / length];
            let normal = [-tangent[1], tangent[0]];
            let dotted = border.style == BorderStyle::Dotted;
            let inset = if dotted { half } else { 0.0 };
            let from = [
                start[0] + normal[0] * half + tangent[0] * inset,
                start[1] + normal[1] * half + tangent[1] * inset,
            ];
            let end_inset = if uniform && dotted { width + half } else { 0.0 };
            let to = [
                end[0] + normal[0] * half - tangent[0] * end_inset,
                end[1] + normal[1] * half - tangent[1] * end_inset,
            ];
            let gap = if dotted {
                dot_gap((length - width).max(0.0), width)
            } else {
                dash_gap(length, width)
            };
            Shape::Pattern {
                contours: vec![Contour {
                    start: from,
                    segments: vec![Segment::Line(to)],
                    closed: false,
                }],
                width,
                dash: [
                    if dotted {
                        0.0
                    } else {
                        DASH_LENGTH_WIDTHS * width
                    },
                    gap,
                ],
                round: dotted,
                clip: (!uniform).then(|| vec![outer[side], outer[next], inner[next], inner[side]]),
            }
        };
        if uniform && self.decoration.borders[0].style == BorderStyle::Dotted {
            if self.decoration.borders[0].color.alpha > 0.0 {
                let mut opposite = [Vec::new(), Vec::new()];
                for side in 0..4 {
                    if let Shape::Pattern { contours, .. } = pattern(side) {
                        opposite[side % 2].extend(contours);
                    }
                }
                for (side, contours) in opposite.into_iter().enumerate() {
                    let Shape::Pattern {
                        width,
                        dash,
                        round,
                        clip,
                        ..
                    } = pattern(side)
                    else {
                        unreachable!("dotted sides are patterns")
                    };
                    fills.push(Fill {
                        shape: Shape::Pattern {
                            contours,
                            width,
                            dash,
                            round,
                            clip,
                        },
                        color: self.decoration.borders[0].color,
                    });
                }
            }
            return fills;
        }
        let borders = self.decoration.borders;
        let joined = |a: usize, b: usize| {
            let bands = borders[a].bands(a);
            borders[a].style == borders[b].style
                && !bands.is_empty()
                && bands == borders[b].bands(b)
        };
        for run in side_runs(widths, joined) {
            let border = borders[run[0]];
            if border.color.alpha <= 0.0 {
                continue;
            }
            let bands = border.bands(run[0]);
            if bands.is_empty() {
                fills.push(Fill {
                    shape: pattern(run[0]),
                    color: border.color,
                });
                continue;
            }
            let last = run[run.len() - 1];
            for band in bands {
                let outer = ring_corners(rect, inner_widths, band.from);
                let inner = ring_corners(rect, inner_widths, band.to);
                let mut points: Vec<_> = std::iter::once(run[0])
                    .chain(run.iter().map(|side| (side + 1) % 4))
                    .map(|corner| outer[corner])
                    .collect();
                points.extend(
                    std::iter::once((last + 1) % 4)
                        .chain(run.iter().rev().copied())
                        .map(|corner| inner[corner]),
                );
                fills.push(Fill {
                    shape: Shape::Path {
                        contours: vec![Contour::polygon(&points)],
                        even_odd: false,
                        clip: None,
                    },
                    color: band.color,
                });
            }
        }
        fills
    }
}
