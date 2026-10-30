//! Glyph ink that `text-decoration-skip-ink: all` interrupts underlines and overlines over
//! (CSS Text Decoration 4 §2.10.5): where the filled glyph outlines meet the band a line paints.
use crate::document::Paragraph;
use crate::text::{GlyphRun, TextContext, TextError};
use ttf_parser::{GlyphId, OutlineBuilder};

/// Straight pieces a quadratic or cubic outline segment is flattened into.
const CURVE_STEPS: u16 = 8;
/// Keeps scanlines off the vertices that bound each band slab.
const SCANLINE_INSET: f32 = 1.0e-4;

/// Closed glyph contours in font units, y up, curves flattened.
#[derive(Default)]
pub(crate) struct Polygons {
    pub(crate) contours: Vec<Vec<[f32; 2]>>,
}
impl Polygons {
    fn last(&self) -> [f32; 2] {
        self.contours
            .last()
            .and_then(|contour| contour.last())
            .copied()
            .unwrap_or_default()
    }
    fn curve(&mut self, point: impl Fn(f32) -> [f32; 2]) {
        for step in 1..=CURVE_STEPS {
            let point = point(f32::from(step) / f32::from(CURVE_STEPS));
            self.line_to(point[0], point[1]);
        }
    }
}
impl OutlineBuilder for Polygons {
    fn move_to(&mut self, x: f32, y: f32) {
        self.contours.push(vec![[x, y]]);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        if let Some(contour) = self.contours.last_mut() {
            contour.push([x, y]);
        }
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let [x0, y0] = self.last();
        self.curve(|t| {
            let u = 1.0 - t;
            [
                u * u * x0 + 2.0 * u * t * x1 + t * t * x,
                u * u * y0 + 2.0 * u * t * y1 + t * t * y,
            ]
        });
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let [x0, y0] = self.last();
        self.curve(|t| {
            let u = 1.0 - t;
            let [a, b, c, d] = [u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t];
            [
                a * x0 + b * x1 + c * x2 + d * x,
                a * y0 + b * y1 + c * y2 + d * y,
            ]
        });
    }
    fn close(&mut self) {}
}

/// Horizontal extents, from the line's left edge, between the leftmost and rightmost ink that
/// each glyph of `runs` (drawn from the line's left edge and baseline) puts between `[top,
/// bottom]` below the baseline. Glyphs without an outline count by their bounding box.
pub(crate) fn ink_extents(
    context: &TextContext<'_>,
    paragraph: &Paragraph,
    runs: &[&GlyphRun],
    [top, bottom]: [f32; 2],
) -> Result<Vec<[f32; 2]>, TextError> {
    let mut extents = Vec::new();
    for run in runs {
        if run.font_size == 0.0 {
            continue;
        }
        let style = &paragraph.spans()[run.source_span].style;
        let Some(instance) = context.face(run.font_id, style)? else {
            continue;
        };
        let face = &instance.face;
        let scale = run.font_size / face.units_per_em() as f32;
        for pen in run.copies([0.0, 0.0]) {
            let [a, b, c, d] = run.orientation.matrix();
            let (a, b) = (a * run.inline_scale, b * run.inline_scale);
            let [x, y] = pen;
            let orient =
                tiny_skia::Transform::from_row(a, b, c, d, x - a * x - c * y, y - b * x - d * y);
            let shear = run
                .shear(y)
                .map_or(tiny_skia::Transform::identity(), |[a, b, c, d, e, f]| {
                    tiny_skia::Transform::from_row(a, b, c, d, e, f)
                });
            for (glyph, [x, y]) in run.glyph_origins(pen) {
                let Ok(id) = u16::try_from(glyph.id).map(GlyphId) else {
                    continue;
                };
                let mut polygons = Polygons::default();
                if face.outline_glyph(id, &mut polygons).is_none() {
                    let Some(bounds) = face.glyph_bounding_box(id) else {
                        continue;
                    };
                    let [left, right, low, high] =
                        [bounds.x_min, bounds.x_max, bounds.y_min, bounds.y_max].map(f32::from);
                    polygons.contours =
                        vec![vec![[left, low], [right, low], [right, high], [left, high]]];
                }
                let transform = orient
                    .pre_concat(shear)
                    .pre_translate(x, y)
                    .pre_scale(scale, -scale);
                let contours: Vec<Vec<[f32; 2]>> = polygons
                    .contours
                    .iter()
                    .map(|contour| {
                        contour
                            .iter()
                            .map(|&[x, y]| {
                                let mut point = tiny_skia::Point::from_xy(x, y);
                                transform.map_point(&mut point);
                                [point.x, point.y]
                            })
                            .collect()
                    })
                    .collect();
                extents.extend(glyph_extent(&contours, [top, bottom]));
            }
        }
    }
    Ok(merge(extents))
}

/// The extent the ink of one glyph's `contours` puts in `[top, bottom]`: the line stays hidden
/// inside a glyph's enclosed areas within the band.
pub(crate) fn glyph_extent(contours: &[Vec<[f32; 2]>], band: [f32; 2]) -> Option<[f32; 2]> {
    band_extents(contours, band)
        .into_iter()
        .reduce(|[left, right], [l, r]| [left.min(l), right.max(r)])
}

/// The horizontal projection of the nonzero-filled `contours` within `[top, bottom]`: the
/// parts of edges inside the band, and the filled spans of scanlines just inside each slab
/// between consecutive vertex heights, which edges vary linearly across.
pub(super) fn band_extents(contours: &[Vec<[f32; 2]>], [top, bottom]: [f32; 2]) -> Vec<[f32; 2]> {
    let edges: Vec<[[f32; 2]; 2]> = contours
        .iter()
        .filter(|contour| contour.len() > 1)
        .flat_map(|contour| {
            contour
                .iter()
                .zip(contour.iter().cycle().skip(1))
                .map(|(&start, &end)| [start, end])
        })
        .filter(|[start, end]| start != end)
        .collect();
    let mut extents = Vec::new();
    for &[[x0, y0], [x1, y1]] in &edges {
        let (low, high) = (y0.min(y1), y0.max(y1));
        if high < top || low > bottom {
            continue;
        }
        let at = |y: f32| {
            if y1 == y0 {
                [x0.min(x1), x0.max(x1)]
            } else {
                let x = x0 + (x1 - x0) * (y - y0) / (y1 - y0);
                [x, x]
            }
        };
        let [a, _] = at(low.max(top));
        let [b, c] = at(high.min(bottom));
        extents.push([a.min(b), a.max(c)]);
    }
    let mut heights: Vec<f32> = edges
        .iter()
        .flat_map(|[start, _]| [start[1]])
        .filter(|y| *y > top && *y < bottom)
        .chain([top, bottom])
        .collect();
    heights.sort_by(f32::total_cmp);
    heights.dedup();
    for pair in heights.windows(2) {
        let inset = (pair[1] - pair[0]) * SCANLINE_INSET;
        for y in [pair[0] + inset, pair[1] - inset] {
            extents.extend(filled_spans(&edges, y));
        }
    }
    extents
}

/// The nonzero-filled spans of scanline `y`.
fn filled_spans(edges: &[[[f32; 2]; 2]], y: f32) -> Vec<[f32; 2]> {
    let mut crossings: Vec<(f32, i32)> = edges
        .iter()
        .filter_map(|&[[x0, y0], [x1, y1]]| {
            let crosses = (y0 <= y && y < y1) || (y1 <= y && y < y0);
            crosses.then(|| {
                (
                    x0 + (x1 - x0) * (y - y0) / (y1 - y0),
                    if y1 > y0 { 1 } else { -1 },
                )
            })
        })
        .collect();
    crossings.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut spans = Vec::new();
    let mut winding = 0;
    let mut start = 0.0;
    for (x, direction) in crossings {
        if winding == 0 {
            start = x;
        }
        winding += direction;
        if winding == 0 {
            spans.push([start, x]);
        }
    }
    spans
}

/// Sorted, disjoint extents covering `extents`.
pub(crate) fn merge(mut extents: Vec<[f32; 2]>) -> Vec<[f32; 2]> {
    extents.sort_by(|a, b| a[0].total_cmp(&b[0]));
    let mut merged: Vec<[f32; 2]> = Vec::with_capacity(extents.len());
    for [left, right] in extents {
        match merged.last_mut() {
            Some(last) if left <= last[1] => last[1] = last[1].max(right),
            _ => merged.push([left, right]),
        }
    }
    merged
}

/// `extent` without the parts within `clearance` of `ink`, which is sorted and disjoint.
pub(crate) fn subtract([left, right]: [f32; 2], ink: &[[f32; 2]], clearance: f32) -> Vec<[f32; 2]> {
    let mut pieces = Vec::new();
    let mut start = left;
    for &[ink_left, ink_right] in ink {
        let [gap_left, gap_right] = [ink_left - clearance, ink_right + clearance];
        if gap_right <= start || gap_left >= right {
            continue;
        }
        if gap_left > start {
            pieces.push([start, gap_left]);
        }
        start = start.max(gap_right);
    }
    if start < right {
        pieces.push([start, right]);
    }
    pieces
}
