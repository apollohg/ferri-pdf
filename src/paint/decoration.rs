//! Text decoration lines (CSS Text Decoration 3 §2) as fills.
use crate::paint::boxes::{DASH_LENGTH_WIDTHS, dash_gap, dot_gap};
use crate::paint::{Color, Contour, Fill, Point, Rect, Segment, Shape};
use serde::Serialize;

/// Offset of a double line's second line, or of a wavy line's centre, from a single line, in thicknesses.
const SECOND_LINE_OFFSET_THICKNESSES: f32 = 1.5;
/// A wave repeats every eight thicknesses.
const WAVE_PERIOD_THICKNESSES: f32 = 8.0;
/// Cubic control-point displacement of a wave, 3√3 thicknesses.
const WAVE_CONTROL_THICKNESSES: f32 = 5.196_152;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum DecorationLine {
    Underline,
    Overline,
    LineThrough,
}
impl DecorationLine {
    /// Direction away from the text along y (positive down); zero for a line through it.
    fn outwards(self) -> f32 {
        match self {
            Self::Underline => 1.0,
            Self::Overline => -1.0,
            Self::LineThrough => 0.0,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub enum DecorationStyle {
    #[default]
    Solid,
    Double,
    Dotted,
    Dashed,
    Wavy,
}

/// One decoration line over a contiguous stretch of decorated text.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct DecorationPaint {
    pub line: DecorationLine,
    pub style: DecorationStyle,
    pub color: Color,
    /// The line's extent; its height is the thickness and its top the single line's top edge.
    pub rect: Rect,
    /// The decorating inline box; `None` is the paragraph's root inline box.
    pub owner: Option<usize>,
}
impl DecorationPaint {
    /// Double and wavy lines move away from the text: underlines down, overlines up, and
    /// line-throughs stay centred.
    pub fn fills(&self) -> Vec<Fill> {
        let Rect {
            x,
            y,
            width,
            height: thickness,
        } = self.rect;
        if width <= 0.0 || thickness <= 0.0 || self.color.alpha <= 0.0 {
            return Vec::new();
        }
        let outwards = self.line.outwards();
        let centre = y + thickness / 2.0;
        let shapes = match self.style {
            DecorationStyle::Solid => vec![Shape::Rect(self.rect)],
            DecorationStyle::Double => {
                let offset = SECOND_LINE_OFFSET_THICKNESSES * thickness;
                let first = if outwards == 0.0 { -offset / 2.0 } else { 0.0 };
                let second = if outwards == 0.0 {
                    offset / 2.0
                } else {
                    offset * outwards
                };
                [first, second]
                    .map(|shift| {
                        Shape::Rect(Rect {
                            y: y + shift,
                            ..self.rect
                        })
                    })
                    .to_vec()
            }
            DecorationStyle::Dotted => vec![Shape::Pattern {
                contours: vec![horizontal(x, centre, width)],
                width: thickness,
                // Round caps centre the first dot on the start; the last ends at the end.
                dash: [0.0, dot_gap(width - thickness / 2.0, thickness)],
                round: true,
                clip: None,
            }],
            DecorationStyle::Dashed => vec![Shape::Pattern {
                contours: vec![horizontal(x, centre, width)],
                width: thickness,
                dash: [DASH_LENGTH_WIDTHS * thickness, dash_gap(width, thickness)],
                round: false,
                clip: None,
            }],
            DecorationStyle::Wavy => vec![Shape::Stroke {
                contours: vec![wave(
                    x,
                    centre + SECOND_LINE_OFFSET_THICKNESSES * thickness * outwards,
                    width,
                    thickness,
                )],
                width: thickness,
            }],
        };
        shapes
            .into_iter()
            .map(|shape| Fill {
                shape,
                color: self.color,
            })
            .collect()
    }
}

fn horizontal(x: f32, y: f32, width: f32) -> Contour {
    Contour {
        start: [x, y],
        segments: vec![Segment::Line([x + width, y])],
        closed: false,
    }
}

/// Whole periods of an S-shaped cubic rising first, then the leading part of one more.
fn wave(x: f32, y: f32, width: f32, thickness: f32) -> Contour {
    let period = WAVE_PERIOD_THICKNESSES * thickness;
    let control = WAVE_CONTROL_THICKNESSES * thickness;
    let mut segments = Vec::new();
    let mut start = x;
    let end = x + width;
    while start < end {
        let curve = [
            [start, y],
            [start + period / 2.0, y - control],
            [start + period / 2.0, y + control],
            [start + period, y],
        ];
        let fraction = ((end - start) / period).min(1.0);
        segments.push(Segment::Cubic(leading_part(curve, fraction)));
        start += period;
    }
    Contour {
        start: [x, y],
        segments,
        closed: false,
    }
}

/// Control points and end of a cubic's part up to parameter `t` (de Casteljau).
fn leading_part([p0, p1, p2, p3]: [Point; 4], t: f32) -> [Point; 3] {
    let lerp = |a: Point, b: Point| [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
    let (p01, p12, p23) = (lerp(p0, p1), lerp(p1, p2), lerp(p2, p3));
    let (p012, p123) = (lerp(p01, p12), lerp(p12, p23));
    [p01, p012, lerp(p012, p123)]
}

#[cfg(test)]
mod tests {
    use super::*;

    const THICKNESS: f32 = 2.0;
    const RED: Color = Color {
        rgb: [255, 0, 0],
        alpha: 1.0,
    };

    fn paint(line: DecorationLine, style: DecorationStyle, width: f32) -> DecorationPaint {
        DecorationPaint {
            line,
            style,
            color: RED,
            rect: Rect {
                x: 10.0,
                y: 20.0,
                width,
                height: THICKNESS,
            },
            owner: None,
        }
    }

    fn rects(fills: &[Fill]) -> Vec<Rect> {
        fills
            .iter()
            .map(|fill| match fill.shape {
                Shape::Rect(rect) => rect,
                ref shape => panic!("double lines are two rectangles, got {shape:?}"),
            })
            .collect()
    }

    #[test]
    fn double_lines_add_a_second_line_away_from_the_text() {
        let under = rects(&paint(DecorationLine::Underline, DecorationStyle::Double, 30.0).fills());
        let over = rects(&paint(DecorationLine::Overline, DecorationStyle::Double, 30.0).fills());
        let through =
            rects(&paint(DecorationLine::LineThrough, DecorationStyle::Double, 30.0).fills());
        let tops = |rects: &[Rect]| rects.iter().map(|rect| rect.y).collect::<Vec<_>>();
        assert_eq!(
            tops(&under),
            [20.0, 23.0],
            "an underline's second line sits 1.5 thicknesses below the first: {under:?}"
        );
        assert_eq!(
            tops(&over),
            [20.0, 17.0],
            "an overline's second line sits 1.5 thicknesses above the first: {over:?}"
        );
        assert_eq!(
            tops(&through),
            [18.5, 21.5],
            "a double line-through straddles the single line's position: {through:?}"
        );
    }

    #[test]
    fn dotted_lines_fit_whole_dot_intervals_ending_at_the_line_end() {
        // A 2pt dotted underline under 17.6699pt of text spaces dots 4.1675pt apart.
        let fills = paint(DecorationLine::Underline, DecorationStyle::Dotted, 17.6699).fills();
        let Shape::Pattern {
            dash, round, width, ..
        } = &fills[0].shape
        else {
            panic!("a dotted line is a stroked pattern: {fills:?}");
        };
        assert!(*round, "dots are round caps of zero-length dashes");
        assert_eq!(*width, THICKNESS, "dots are as wide as the line is thick");
        assert!(
            (dash[1] - 4.1675).abs() < 1e-3 && dash[0] == 0.0,
            "four intervals fit between the first dot centre and the last dot's far edge: {dash:?}"
        );
    }

    #[test]
    fn wavy_lines_end_exactly_at_the_decorated_width_and_move_outwards() {
        let width = 39.5508;
        let fills = paint(DecorationLine::Underline, DecorationStyle::Wavy, width).fills();
        let Shape::Stroke { contours, .. } = &fills[0].shape else {
            panic!("a wavy line is a stroked path: {fills:?}");
        };
        let contour = &contours[0];
        let centre = 20.0 + THICKNESS / 2.0 + 1.5 * THICKNESS;
        assert_eq!(
            contour.start,
            [10.0, centre],
            "an underline wave is centred 1.5 thicknesses below the single line"
        );
        let Segment::Cubic([first, second, _]) = contour.segments[0] else {
            panic!("waves are cubic segments: {contour:?}");
        };
        assert!(
            first[1] < centre && second[1] > centre,
            "each period rises towards the text first: {first:?} {second:?}"
        );
        assert_eq!(
            contour.segments.len(),
            3,
            "39.55pt holds two 16pt periods and part of a third: {contour:?}"
        );
        let Segment::Cubic([.., end]) = contour.segments[2] else {
            unreachable!()
        };
        // The last period is split at the linear fraction of its length.
        assert!(
            (end[0] - 75.6627 + 36.0 - 10.0).abs() < 1e-3,
            "the last partial period ends at its linear fraction: {end:?}"
        );
    }
}
