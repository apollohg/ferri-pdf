//! Page production geometry; content stays in the original layout coordinates.
use crate::graphics::{BoxDecoration, BoxPaint, Color, Point, Rect};
use crate::layout::LayoutError;
use crate::style::PageProduction;
use serde::Serialize;

const MARK_MARGIN_PT: f32 = 18.0;
const MARK_GAP_PT: f32 = 3.0;
const CROP_LENGTH_PT: f32 = 12.0;
const CROSS_RADIUS_PT: f32 = 4.0;
const MARK_WIDTH_PT: f32 = 0.5;

#[derive(Clone, Serialize)]
pub struct PageOutput {
    pub media: Rect,
    pub trim: Rect,
    pub bleed: Rect,
    pub marks: Vec<BoxPaint>,
}

#[derive(Clone, Serialize)]
pub struct PageRecord {
    pub request: crate::style::PageRequest,
    pub geometry: crate::style::PageGeometry,
    pub output: PageOutput,
}
impl PageRecord {
    pub(crate) fn new(page: &crate::document::PageBox) -> Result<Self, LayoutError> {
        Ok(Self {
            request: page.request.clone(),
            geometry: page.geometry.clone(),
            output: PageOutput::new(
                page.geometry.paper,
                page.geometry.margin_box(),
                page.production,
            )?,
        })
    }
}

impl PageOutput {
    pub(super) fn new(
        sheet: [f32; 2],
        mut trim: Rect,
        production: PageProduction,
    ) -> Result<Self, LayoutError> {
        if !trim.width.is_finite() || !trim.height.is_finite() {
            return Err(LayoutError::NonFinitePageOutputGeometry);
        }
        // Inverted margin extents have no physical area; CSS layout stays unchanged.
        trim.width = trim.width.max(0.0);
        trim.height = trim.height.max(0.0);
        // A negative bleed can collapse the painting area, but cannot invert it.
        let used_bleed = production.bleed.max(-trim.width.min(trim.height) / 2.0);
        let bleed = Rect {
            x: trim.x - used_bleed,
            y: trim.y - used_bleed,
            width: (trim.width + 2.0 * used_bleed).max(0.0),
            height: (trim.height + 2.0 * used_bleed).max(0.0),
        };
        let marked = production.marks.crop || production.marks.cross;
        let reserve = used_bleed.max(0.0) + if marked { MARK_MARGIN_PT } else { 0.0 };
        let media = Rect {
            x: 0.0,
            y: 0.0,
            width: sheet[0],
            height: sheet[1],
        }
        .union(Rect {
            x: trim.x - reserve,
            y: trim.y - reserve,
            width: trim.width + 2.0 * reserve,
            height: trim.height + 2.0 * reserve,
        });
        let mut output = Self {
            media,
            trim,
            bleed,
            marks: Vec::new(),
        };
        let gap = used_bleed.max(0.0) + MARK_GAP_PT;
        if production.marks.crop {
            for x in [trim.x, trim.x + trim.width] {
                for (edge, direction) in [(trim.y, -1.0), (trim.y + trim.height, 1.0)] {
                    let start = edge + direction * gap;
                    output.line([x, start], [x, start + direction * CROP_LENGTH_PT]);
                }
            }
            for y in [trim.y, trim.y + trim.height] {
                for (edge, direction) in [(trim.x, -1.0), (trim.x + trim.width, 1.0)] {
                    let start = edge + direction * gap;
                    output.line([start, y], [start + direction * CROP_LENGTH_PT, y]);
                }
            }
        }
        if production.marks.cross {
            let x = trim.x + trim.width / 2.0;
            let y = trim.y + trim.height / 2.0;
            let outside = gap + CROSS_RADIUS_PT;
            for center in [
                [x, trim.y - outside],
                [x, trim.y + trim.height + outside],
                [trim.x - outside, y],
                [trim.x + trim.width + outside, y],
            ] {
                output.line(
                    [center[0] - CROSS_RADIUS_PT, center[1]],
                    [center[0] + CROSS_RADIUS_PT, center[1]],
                );
                output.line(
                    [center[0], center[1] - CROSS_RADIUS_PT],
                    [center[0], center[1] + CROSS_RADIUS_PT],
                );
            }
        }
        if std::iter::once(media)
            .chain([trim, bleed])
            .chain(output.marks.iter().map(|mark| mark.rect))
            .any(|rect| {
                [rect.x, rect.y, rect.x + rect.width, rect.y + rect.height]
                    .iter()
                    .any(|value| !value.is_finite())
            })
        {
            return Err(LayoutError::NonFinitePageOutputGeometry);
        }
        Ok(output)
    }

    fn line(&mut self, from: Point, to: Point) {
        let vertical = from[0] == to[0];
        debug_assert!(vertical || from[1] == to[1]);
        let half = MARK_WIDTH_PT / 2.0;
        let rect = Rect {
            x: from[0].min(to[0]) - if vertical { half } else { 0.0 },
            y: from[1].min(to[1]) - if vertical { 0.0 } else { half },
            width: if vertical {
                MARK_WIDTH_PT
            } else {
                (to[0] - from[0]).abs()
            },
            height: if vertical {
                (to[1] - from[1]).abs()
            } else {
                MARK_WIDTH_PT
            },
        };
        self.marks.push(BoxPaint::whole(
            rect,
            BoxDecoration {
                background: Color::BLACK,
                ..Default::default()
            },
        ));
    }
}
