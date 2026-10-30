//! CSS Backgrounds 3 §6 border images: nine-slice regions, their sizes and tiling.
use crate::paint::background::{Source, TileImage, UnresolvedImage, natural_points};
use crate::paint::{
    BackgroundImage, BackgroundSize, Border, BoxPaint, LengthPercentage, POINTS_PER_PIXEL, Rect,
    image_size,
};
use crate::paint::{ImageInfo, NaturalDimensions};
use serde::Serialize;
use std::convert::Infallible;

/// Sides in top, right, bottom, left order; columns and rows run start, middle, end.
const TOP: usize = 0;
const RIGHT: usize = 1;
const BOTTOM: usize = 2;
const LEFT: usize = 3;
const START: usize = 0;
const MIDDLE: usize = 1;
const END: usize = 2;
/// The middle cell of the nine-slice grid.
const CELLS: usize = 3;

/// `border-image-slice` offset: image pixels (vector coordinates) or a fraction of the image.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub enum ImageSlice {
    Number(f32),
    Percentage(f32),
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub enum BorderImageWidth {
    /// A multiple of the border width.
    Number(f32),
    /// Percentages resolve against the border image area.
    Length(LengthPercentage),
    /// The natural size of the image slice.
    Auto,
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub enum BorderImageOutset {
    /// A multiple of the border width.
    Number(f32),
    Length(f32),
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub enum BorderImageRepeat {
    #[default]
    Stretch,
    Repeat,
    Round,
    Space,
}

/// Computed `border-image-*` longhands; sides are top, right, bottom, left.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(bound(serialize = ""))]
pub struct BorderImage<I = Infallible> {
    pub source: BackgroundImage<I>,
    pub slice: [ImageSlice; 4],
    pub fill: bool,
    pub widths: [BorderImageWidth; 4],
    pub outset: [BorderImageOutset; 4],
    /// Horizontal then vertical.
    pub repeat: [BorderImageRepeat; 2],
}
impl<I> Default for BorderImage<I> {
    fn default() -> Self {
        Self {
            source: BackgroundImage::None,
            slice: [ImageSlice::Percentage(1.0); 4],
            fill: false,
            widths: std::array::from_fn(|_| BorderImageWidth::Number(1.0)),
            outset: [BorderImageOutset::Length(0.0); 4],
            repeat: [BorderImageRepeat::Stretch; 2],
        }
    }
}
impl BorderImage<Infallible> {
    pub fn with_image_type<I>(&self) -> BorderImage<I> {
        BorderImage {
            source: self.source.with_image_type(),
            slice: self.slice,
            fill: self.fill,
            widths: self.widths.clone(),
            outset: self.outset,
            repeat: self.repeat,
        }
    }
}
impl<I> BorderImage<I> {
    /// Whether an image replaces the border styles.
    pub fn is_drawn(&self) -> bool {
        !matches!(self.source, BackgroundImage::None)
    }
}

/// One of the nine regions: the image slice `source`, in image points, drawn into `tiles`
/// and clipped to `area`.
#[derive(Clone, Debug, PartialEq)]
pub struct ImagePiece {
    pub area: Rect,
    pub source: Rect,
    pub tiles: Vec<Rect>,
}

/// A border image ready to paint: the image at `size` points, its pieces, and the clip that
/// cuts a sliced fragment's share out of the unbroken box's image.
#[derive(Clone, Debug)]
pub struct BorderImagePaint<'a, I = Infallible> {
    pub image: TileImage<'a, I>,
    pub size: [f32; 2],
    pub clip: Option<Rect>,
    pub pieces: Vec<ImagePiece>,
}

/// Tile origins and length along one axis of `span` for `repeat` (§6.6): stretched,
/// repeated from the centre, rounded to a whole number, or spaced with equal gaps around.
fn tile_axis(repeat: BorderImageRepeat, span: [f32; 2], tile: f32) -> (Vec<f32>, f32) {
    let [start, length] = span;
    if length <= 0.0 || tile <= 0.0 || !tile.is_finite() {
        return (Vec::new(), 0.0);
    }
    match repeat {
        BorderImageRepeat::Stretch => (vec![start], length),
        BorderImageRepeat::Round => {
            let count = (length / tile).round().max(1.0);
            let size = length / count;
            (
                (0..count as usize)
                    .map(|i| start + i as f32 * size)
                    .collect(),
                size,
            )
        }
        BorderImageRepeat::Space => {
            let count = (length / tile).floor();
            let gap = (length - count * tile) / (count + 1.0);
            (
                (0..count as usize)
                    .map(|i| start + gap + i as f32 * (tile + gap))
                    .collect(),
                tile,
            )
        }
        BorderImageRepeat::Repeat => {
            let first = start + (length - tile) / 2.0;
            let before = ((first - start) / tile).ceil();
            let count = (length / tile).ceil() as usize + 2;
            (
                (0..count)
                    .map(|i| first + (i as f32 - before) * tile)
                    .filter(|origin| *origin < start + length && origin + tile > start)
                    .collect(),
                tile,
            )
        }
    }
}

/// `factor` unless it is zero or not finite, then `fallback`'s, then one (§6.6 middle part).
fn scale_or(factor: f32, fallback: f32) -> f32 {
    [factor, fallback]
        .into_iter()
        .find(|scale| *scale > 0.0 && scale.is_finite())
        .unwrap_or(1.0)
}

impl<I> BoxPaint<I> {
    fn image_outsets(&self) -> [f32; 4] {
        std::array::from_fn(|side| match self.decoration.border_image.outset[side] {
            BorderImageOutset::Number(factor) => {
                factor * self.decoration.borders[side].used_width()
            }
            BorderImageOutset::Length(length) => length,
        })
    }

    /// The border image's ink, with introduced fragment edges cut as in its painting.
    pub(crate) fn border_image_bounds(&self) -> Option<Rect> {
        self.decoration.border_image.is_drawn().then(|| {
            let sliced = self.sliced();
            let outsets = self.image_outsets();
            crate::paint::outset(
                self.rect,
                std::array::from_fn(|side| if sliced[side] { 0.0 } else { outsets[side] }),
            )
        })
    }
}

impl<I: ImageInfo> BoxPaint<I> {
    /// The border image replacing this box's border styles, if it has one to draw.
    pub fn border_image(&self) -> Result<Option<BorderImagePaint<'_, I>>, UnresolvedImage> {
        let border_image = &self.decoration.border_image;
        let source = match &border_image.source {
            BackgroundImage::None => return Ok(None),
            BackgroundImage::Url { url, .. } => return Err(UnresolvedImage(url.clone())),
            BackgroundImage::Image {
                image,
                resolution,
                scale,
            } => Source::Image(image, *resolution, *scale),
            BackgroundImage::Gradient(gradient) => Source::Gradient(gradient),
        };
        // A sliced fragment draws its share of the unbroken box's image.
        let borders = self.decoration.borders.map(Border::used_width);
        let outset = self.image_outsets();
        let area = crate::paint::outset(self.background_box.unwrap_or(self.rect), outset);
        if area.width <= 0.0 || area.height <= 0.0 {
            return Ok(None);
        }
        let natural = match source {
            Source::Image(image, resolution, scale) => natural_points(image, resolution, scale),
            Source::Gradient(_) => NaturalDimensions::default(),
        };
        let size = image_size(
            &BackgroundSize::default(),
            natural,
            [area.width, area.height],
        );
        if !(size[0] > 0.0 && size[1] > 0.0 && size.iter().all(|v| v.is_finite())) {
            return Ok(None);
        }
        let image = match source {
            Source::Image(image, _, scale) => TileImage::Image(image, scale),
            Source::Gradient(gradient) => match gradient.resolve(size) {
                Some(resolved) => TileImage::Gradient(resolved),
                None => return Ok(None),
            },
        };
        let slices: [f32; 4] = std::array::from_fn(|side| {
            let extent = size[usize::from(side == TOP || side == BOTTOM)];
            match border_image.slice[side] {
                ImageSlice::Number(pixels) => pixels * POINTS_PER_PIXEL,
                ImageSlice::Percentage(fraction) => fraction * extent,
            }
            .clamp(0.0, extent)
        });
        let widths = self.image_widths(area, borders, slices, natural);
        let pieces = nine_slice(border_image, area, size, slices, widths);
        let sliced = self.sliced();
        let clip = sliced.contains(&true).then(|| {
            let rect = self.rect;
            let [top, right, bottom, left] =
                std::array::from_fn(|side| if sliced[side] { 0.0 } else { outset[side] });
            Rect {
                x: rect.x - left,
                y: rect.y - top,
                width: rect.width + left + right,
                height: rect.height + top + bottom,
            }
        });
        Ok(Some(BorderImagePaint {
            image,
            size,
            clip,
            pieces,
        }))
    }

    /// `border-image-width` per side, reduced together when opposite widths overlap (§6.4).
    fn image_widths(
        &self,
        area: Rect,
        borders: [f32; 4],
        slices: [f32; 4],
        natural: NaturalDimensions,
    ) -> [f32; 4] {
        let has_natural = natural.width.is_some() || natural.height.is_some();
        let widths: [f32; 4] = std::array::from_fn(|side| {
            let extent = if side == TOP || side == BOTTOM {
                area.height
            } else {
                area.width
            };
            match &self.decoration.border_image.widths[side] {
                BorderImageWidth::Number(factor) => factor * borders[side],
                BorderImageWidth::Length(length) => length.resolve(extent),
                BorderImageWidth::Auto if has_natural => slices[side],
                BorderImageWidth::Auto => borders[side],
            }
        });
        let factor = [
            area.width / (widths[LEFT] + widths[RIGHT]),
            area.height / (widths[TOP] + widths[BOTTOM]),
        ]
        .into_iter()
        .filter(|factor| factor.is_finite())
        .fold(1.0_f32, f32::min);
        widths.map(|width| width * factor)
    }
}

/// Start, middle and end spans `[origin, length]` of an axis cut at `start` and `end` from
/// its edges.
fn spans(origin: f32, length: f32, start: f32, end: f32) -> [[f32; 2]; 3] {
    [
        [origin, start],
        [origin + start, length - start - end],
        [origin + length - end, end],
    ]
}

/// The nine regions of §6.6 with their tiles.
fn nine_slice<I>(
    border_image: &BorderImage<I>,
    area: Rect,
    size: [f32; 2],
    slices: [f32; 4],
    widths: [f32; 4],
) -> Vec<ImagePiece> {
    let columns = spans(area.x, area.width, widths[LEFT], widths[RIGHT]);
    let rows = spans(area.y, area.height, widths[TOP], widths[BOTTOM]);
    let source_columns = spans(0.0, size[0], slices[LEFT], slices[RIGHT]);
    let source_rows = spans(0.0, size[1], slices[TOP], slices[BOTTOM]);
    let rect = |[x, width]: [f32; 2], [y, height]: [f32; 2]| Rect {
        x,
        y,
        width,
        height,
    };
    // Edges scale their slices to the border width across them (§6.6 step 2).
    let scale = |width: f32, slice: f32| width / slice;
    let horizontal_scale = |row: usize| scale(rows[row][1], source_rows[row][1]);
    let vertical_scale = |column: usize| scale(columns[column][1], source_columns[column][1]);
    let mut pieces = Vec::new();
    for row in 0..CELLS {
        for column in 0..CELLS {
            if row == MIDDLE && column == MIDDLE && !border_image.fill {
                continue;
            }
            let area = rect(columns[column], rows[row]);
            let source = rect(source_columns[column], source_rows[row]);
            if area.width <= 0.0
                || area.height <= 0.0
                || source.width <= 0.0
                || source.height <= 0.0
            {
                continue;
            }
            let tiles = if row != MIDDLE && column != MIDDLE {
                vec![area]
            } else {
                // Tile sizes: edges scale proportionally to their cross size, the middle by
                // the top (else bottom) and left (else right) factors.
                let tile = match (row, column) {
                    (MIDDLE, MIDDLE) => [
                        source.width * scale_or(horizontal_scale(START), horizontal_scale(END)),
                        source.height * scale_or(vertical_scale(START), vertical_scale(END)),
                    ],
                    (MIDDLE, _) => [area.width, source.height * vertical_scale(column)],
                    _ => [source.width * horizontal_scale(row), area.height],
                };
                let (xs, width) = if column == MIDDLE {
                    tile_axis(border_image.repeat[0], columns[column], tile[0])
                } else {
                    (vec![area.x], area.width)
                };
                let (ys, height) = if row == MIDDLE {
                    tile_axis(border_image.repeat[1], rows[row], tile[1])
                } else {
                    (vec![area.y], area.height)
                };
                ys.iter()
                    .flat_map(|&y| {
                        xs.iter().map(move |&x| Rect {
                            x,
                            y,
                            width,
                            height,
                        })
                    })
                    .collect()
            };
            pieces.push(ImagePiece {
                area,
                source,
                tiles,
            });
        }
    }
    pieces
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOLERANCE: f32 = 1e-4;

    fn assert_close(label: &str, actual: &[f32], expected: &[f32]) {
        assert_eq!(
            actual.len(),
            expected.len(),
            "{label}: {actual:?} vs {expected:?}"
        );
        for (a, e) in actual.iter().zip(expected) {
            assert!(
                (a - e).abs() < TOLERANCE,
                "{label}: {actual:?}, expected {expected:?}"
            );
        }
    }

    #[test]
    fn repeat_modes_place_tiles_along_an_axis() {
        // A 100pt span from x = 10 with 30pt tiles.
        let span = [10.0, 100.0];
        let (origins, size) = tile_axis(BorderImageRepeat::Stretch, span, 30.0);
        assert_close("stretch fills the span with one tile", &origins, &[10.0]);
        assert_close("stretch size", &[size], &[100.0]);
        let (origins, size) = tile_axis(BorderImageRepeat::Round, span, 30.0);
        assert_close(
            "round: 100 / 30 rounds to 3 tiles of 33.33",
            &origins,
            &[10.0, 43.3333, 76.6667],
        );
        assert_close("round size", &[size], &[100.0 / 3.0]);
        let (origins, size) = tile_axis(BorderImageRepeat::Space, span, 30.0);
        assert_close(
            "space: 3 whole tiles with 2.5pt gaps around each",
            &origins,
            &[12.5, 45.0, 77.5],
        );
        assert_close("space size", &[size], &[30.0]);
        let (origins, _) = tile_axis(BorderImageRepeat::Repeat, span, 30.0);
        assert_close(
            "repeat: centred tile at 45, partial tiles out to both ends",
            &origins,
            &[-15.0, 15.0, 45.0, 75.0, 105.0],
        );
        let (origins, _) = tile_axis(BorderImageRepeat::Space, [0.0, 20.0], 30.0);
        assert!(
            origins.is_empty(),
            "space draws nothing when no whole tile fits: {origins:?}"
        );
        let (origins, size) = tile_axis(BorderImageRepeat::Round, [0.0, 10.0], 30.0);
        assert_close(
            "round keeps at least one tile, squeezed to the span",
            &[origins[0], size],
            &[0.0, 10.0],
        );
    }

    fn rect(x: f32, y: f32, width: f32, height: f32) -> Rect {
        Rect {
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn nine_slice_scales_edges_by_their_widths_and_the_middle_by_the_top_and_left() {
        let image: BorderImage = BorderImage {
            fill: true,
            repeat: [BorderImageRepeat::Repeat, BorderImageRepeat::Round],
            ..BorderImage::default()
        };
        // A 30×30 image sliced 10 each side; a 100×60 area with 20pt top, 10pt right,
        // 20pt bottom and 5pt left widths.
        let pieces = nine_slice(
            &image,
            rect(0.0, 0.0, 100.0, 60.0),
            [30.0, 30.0],
            [10.0; 4],
            [20.0, 10.0, 20.0, 5.0],
        );
        assert_eq!(pieces.len(), 9, "fill keeps the middle: {pieces:#?}");
        let corner = &pieces[0];
        assert_eq!(
            (corner.area, corner.source, corner.tiles.as_slice()),
            (
                rect(0.0, 0.0, 5.0, 20.0),
                rect(0.0, 0.0, 10.0, 10.0),
                &[rect(0.0, 0.0, 5.0, 20.0)][..]
            ),
            "corners stretch their slice into the corner"
        );
        let top = &pieces[1];
        assert_eq!(top.area, rect(5.0, 0.0, 85.0, 20.0));
        let widths: Vec<f32> = top.tiles.iter().map(|tile| tile.width).collect();
        assert!(
            widths.iter().all(|width| (width - 20.0).abs() < TOLERANCE),
            "the top slice scales by 20 / 10, so its tiles are 20pt wide: {widths:?}"
        );
        assert_close(
            "repeat centres the top tiles in the 85pt edge",
            &top.tiles.iter().map(|tile| tile.x).collect::<Vec<_>>(),
            &[-2.5, 17.5, 37.5, 57.5, 77.5],
        );
        let left = &pieces[3];
        assert_eq!(left.area, rect(0.0, 20.0, 5.0, 20.0));
        assert_close(
            "the left slice scales by 5 / 10 to 5pt tiles, rounded to 4 in the 20pt edge",
            &left
                .tiles
                .iter()
                .map(|tile| tile.height)
                .collect::<Vec<_>>(),
            &[5.0; 4],
        );
        let middle = &pieces[4];
        assert_close(
            "the middle takes the top factor across (20pt) and the left factor down (5pt)",
            &[middle.tiles[0].width, middle.tiles[0].height],
            &[20.0, 5.0],
        );
        assert_eq!(middle.tiles.len(), 5 * 4, "5 columns of 4 rows");
    }

    #[test]
    fn middle_scale_falls_back_to_the_opposite_edge_then_to_one() {
        assert_close(
            "a zero top width gives way to the bottom factor",
            &[scale_or(0.0, 3.0)],
            &[3.0],
        );
        assert_close(
            "both infinite: unscaled",
            &[scale_or(f32::INFINITY, f32::INFINITY)],
            &[1.0],
        );
        let image: BorderImage = BorderImage {
            fill: true,
            ..BorderImage::default()
        };
        // Slices that leave no middle in the image draw no edges or middle on that axis.
        let pieces = nine_slice(
            &image,
            rect(0.0, 0.0, 100.0, 60.0),
            [30.0, 30.0],
            [10.0, 15.0, 10.0, 15.0],
            [10.0; 4],
        );
        assert!(
            pieces
                .iter()
                .all(|piece| piece.source.width > 0.0 && piece.source.height > 0.0),
            "empty slices are skipped"
        );
        assert_eq!(
            pieces.len(),
            6,
            "left and right slices meeting leave corners and side edges only: {pieces:#?}"
        );
    }
}
