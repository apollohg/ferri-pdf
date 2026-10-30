//! Emphasis marks (CSS Text Decoration 3 §3): one per typographic character unit, centred
//! over or under it as a ruby annotation would be, in the element's font at half size with
//! the `ruby` feature.
use crate::document::Paragraph;
use crate::graphics::{Contour, Point, Rect, Segment};
use crate::style::Style;
use crate::style::{EmphasisShape, EmphasisSkip};
use crate::text::line_boxes::BoxMetrics;
use crate::text::shaping::MissingGlyphs;
use crate::text::{GlyphRun, Line, MarkShape, TextContext, TextError};
use icu_properties::{
    CodePointMapData,
    props::{EastAsianWidth, GeneralCategory, GeneralCategoryGroup},
};
use unicode_segmentation::UnicodeSegmentation;

/// Marks are drawn at half the element's font size (§3.1).
const MARK_SCALE: f32 = 0.5;
/// Punctuation classified as symbols, including NFKD equivalents (§3.5).
const PUNCTUATION_SYMBOLS: [char; 15] = [
    '#', '%', '\u{2030}', '\u{2031}', '\u{066A}', '\u{0609}', '\u{060A}', '&', '\u{204A}', '@',
    '\u{00A7}', '\u{00B6}', '\u{204B}', '\u{2053}', '\u{303D}',
];

/// Eligibility of one typographic character unit (CSS Text Decoration 4 §3.5).
pub(super) fn marked(cluster: &str, skip: EmphasisSkip) -> bool {
    let categories = CodePointMapData::<GeneralCategory>::new();
    let mut chars = cluster.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    let category = categories.get(first);
    if [
        GeneralCategory::Control,
        GeneralCategory::Format,
        GeneralCategory::Unassigned,
    ]
    .contains(&category)
        || (skip.narrow
            && !matches!(
                CodePointMapData::<EastAsianWidth>::new().get(first),
                EastAsianWidth::Wide | EastAsianWidth::Fullwidth
            ))
    {
        return false;
    }
    if GeneralCategoryGroup::Separator.contains(category) {
        return !skip.spaces
            || chars.any(|ch| GeneralCategoryGroup::Mark.contains(categories.get(ch)));
    }
    let punctuation = GeneralCategoryGroup::Punctuation.contains(category);
    let symbol = GeneralCategoryGroup::Symbol.contains(category)
        || (punctuation && {
            let normalizer = icu_normalizer::DecomposingNormalizerBorrowed::new_nfkd();
            let mut buffer = [0; 4];
            let decomposed = normalizer.normalize(first.encode_utf8(&mut buffer));
            let mut decomposed = decomposed.chars();
            decomposed
                .next()
                .is_some_and(|symbol| PUNCTUATION_SYMBOLS.contains(&symbol))
                && decomposed.next().is_none()
        });
    !(symbol && skip.symbols || punctuation && !symbol && skip.punctuation)
}

/// The style marks are drawn in: the element's font at half size with the `ruby` feature,
/// in the emphasis colour, without spacing, strokes or shadows of their own.
fn mark_style(style: &Style) -> Style {
    let mut mark = style.clone();
    mark.size *= MARK_SCALE;
    let mut east_asian = mark.font_features.east_asian.to_vec();
    east_asian.push(crate::style::RUBY_FEATURE);
    mark.font_features.east_asian = east_asian.into();
    mark.letter_spacing = 0.0;
    mark.word_spacing = crate::style::LengthPercentage::ZERO;
    mark.text_paint.fill = mark.text_paint.emphasis_color.clone();
    mark.text_paint.stroke_width = 0.0;
    mark.text_paint.shadows = std::sync::Arc::from([]);
    mark
}

/// Whether any typographic character unit of `run` takes a mark.
fn has_marks(run: &GlyphRun, skip: EmphasisSkip) -> bool {
    if run.combined {
        let mut buffer = [0; 4];
        return run.width != 0.0
            && marked(
                crate::document::OBJECT_REPLACEMENT_CHARACTER.encode_utf8(&mut buffer),
                skip,
            );
    }
    run.text
        .graphemes(true)
        .any(|cluster| marked(cluster, skip))
}

/// Extra leading the marks of `run` need, and whether it goes under the text: as for ruby,
/// enough that lines of identical marked text would not overlap (CSS Ruby 1 §3.6).
pub(super) fn leading(
    context: &TextContext<'_>,
    style: &Style,
    run: &GlyphRun,
) -> Result<Option<(f32, bool)>, TextError> {
    if style.text_paint.emphasis.mark().is_none() || !has_marks(run, style.text_paint.emphasis_skip)
    {
        return Ok(None);
    }
    let base = BoxMetrics::of(context, style)?;
    let mark = BoxMetrics::of(context, &mark_style(style))?;
    let extra = mark.ascent + mark.descent + base.ascent + base.descent - base.line_height();
    let under = style.text_paint.emphasis_position.under(style);
    Ok((extra > 0.0).then_some((extra, under)))
}

/// Places the marks of the line's runs once they are positioned: each mark's content area
/// abuts the over or under edge of its element's content area, centred on its character.
pub(super) fn place(
    context: &TextContext<'_>,
    paragraph: &Paragraph,
    line: &mut Line,
) -> Result<(), TextError> {
    for run in &mut line.runs {
        let style = &paragraph.spans()[run.source_span].style;
        let Some(text) = style.text_paint.emphasis.mark() else {
            continue;
        };
        if !has_marks(run, style.text_paint.emphasis_skip) {
            continue;
        }
        let mark_style = mark_style(style);
        let shape = style.text_paint.emphasis.shape();
        // A shape no font has is synthesized; a string's missing glyphs show as such.
        let shaped = match context.shape(
            &text,
            0..text.len(),
            false,
            &mark_style,
            if shape.is_some() {
                MissingGlyphs::Reject
            } else {
                MissingGlyphs::Substitute
            },
        ) {
            Ok(shaped) => Some(shaped),
            Err(TextError::MissingGlyph { .. }) => None,
            Err(error) => return Err(error),
        };
        let base = BoxMetrics::of(context, style)?;
        let metrics = BoxMetrics::of(context, &mark_style)?;
        let baseline = run.baseline_shift
            + if style.text_paint.emphasis_position.under(style) {
                base.descent + metrics.ascent
            } else {
                -(base.ascent + metrics.descent)
            };
        let mut marks = Vec::new();
        let mut shapes = Vec::new();
        let clusters = if run.combined {
            vec![(0, run.text.as_str())]
        } else {
            run.text.grapheme_indices(true).collect()
        };
        for (offset, cluster) in clusters {
            if !run.combined && !marked(cluster, style.text_paint.emphasis_skip) {
                continue;
            }
            let start = run.source_range.start + offset;
            let centre = run.x + f32::midpoint(run.x_at(start), run.x_at(start + cluster.len()));
            match (&shaped, shape) {
                (Some(shaped), _) => {
                    let width: f32 = shaped.iter().map(|mark| mark.width).sum();
                    let mut x = centre - width / 2.0;
                    for mark in shaped {
                        marks.push(GlyphRun {
                            x,
                            baseline_shift: baseline,
                            source_range: start..start + cluster.len(),
                            source_span: run.source_span,
                            ..mark.clone()
                        });
                        x += mark.width;
                    }
                }
                (None, Some((shape, filled))) => shapes.push(MarkShape {
                    contours: synthesized(
                        shape,
                        filled,
                        [
                            centre,
                            baseline - f32::midpoint(metrics.ascent, -metrics.descent),
                        ],
                        mark_style.size,
                    ),
                    color: mark_style.text_paint.fill.used,
                }),
                (None, None) => unreachable!("strings substitute missing glyphs"),
            }
        }
        run.marks = marks;
        run.mark_shapes = shapes;
    }
    Ok(())
}

/// Proportions of synthesized marks, in mark ems: radii of dots, circles and the inner disc
/// of a double circle, the side of a triangle, the length and width of a sesame, and the
/// line width of open shapes.
const DOT_RADIUS_EM: f32 = 0.15;
const CIRCLE_RADIUS_EM: f32 = 0.4;
const INNER_CIRCLE_RADIUS_EM: f32 = 0.2;
const TRIANGLE_SIDE_EM: f32 = 0.8;
const SESAME_LENGTH_EM: f32 = 0.5;
const SESAME_WIDTH_EM: f32 = 0.24;
/// A sesame leans this far clockwise from upright, in radians.
const SESAME_LEAN: f32 = std::f32::consts::FRAC_PI_6;
const OPEN_LINE_EM: f32 = 0.08;

fn circle([x, y]: Point, radius: f32) -> Contour {
    crate::paint::rounded_rect(
        Rect {
            x: x - radius,
            y: y - radius,
            width: 2.0 * radius,
            height: 2.0 * radius,
        },
        [[radius; 2]; 4],
    )
}

/// `contour` scaled by `factor` about `centre`, then turned by `angle` clockwise.
fn transformed(contour: &Contour, [cx, cy]: Point, factor: f32, angle: f32) -> Contour {
    let (sin, cos) = angle.sin_cos();
    let map = |[x, y]: Point| {
        let [dx, dy] = [(x - cx) * factor, (y - cy) * factor];
        [cx + dx * cos - dy * sin, cy + dx * sin + dy * cos]
    };
    Contour {
        start: map(contour.start),
        segments: contour
            .segments
            .iter()
            .map(|segment| match *segment {
                Segment::Line(point) => Segment::Line(map(point)),
                Segment::Cubic(points) => Segment::Cubic(points.map(map)),
            })
            .collect(),
        closed: contour.closed,
    }
}

/// A mark the UA synthesizes (§3.1), centred on `centre`, filled even-odd: an open shape is
/// its outline less a copy shrunk by the open line width.
fn synthesized(shape: EmphasisShape, filled: bool, centre: Point, em: f32) -> Vec<Contour> {
    let [x, y] = centre;
    let outline = match shape {
        EmphasisShape::Dot => circle(centre, DOT_RADIUS_EM * em),
        EmphasisShape::Circle | EmphasisShape::DoubleCircle => {
            circle(centre, CIRCLE_RADIUS_EM * em)
        }
        EmphasisShape::Triangle => {
            let side = TRIANGLE_SIDE_EM * em;
            let height = side * 3.0_f32.sqrt() / 2.0;
            Contour::polygon(&[
                [x, y - height / 2.0],
                [x + side / 2.0, y + height / 2.0],
                [x - side / 2.0, y + height / 2.0],
            ])
        }
        EmphasisShape::Sesame => {
            let [length, width] = [SESAME_LENGTH_EM * em, SESAME_WIDTH_EM * em];
            let upright = crate::paint::rounded_rect(
                Rect {
                    x: x - width / 2.0,
                    y: y - length / 2.0,
                    width,
                    height: length,
                },
                [[width / 2.0; 2]; 4],
            );
            transformed(&upright, centre, 1.0, SESAME_LEAN)
        }
    };
    let radius = match shape {
        EmphasisShape::Dot => DOT_RADIUS_EM,
        EmphasisShape::Circle | EmphasisShape::DoubleCircle => CIRCLE_RADIUS_EM,
        EmphasisShape::Triangle => TRIANGLE_SIDE_EM / 3.0_f32.sqrt(),
        EmphasisShape::Sesame => SESAME_WIDTH_EM / 2.0,
    };
    let hollow = |contour: &Contour, radius: f32| {
        transformed(contour, centre, 1.0 - OPEN_LINE_EM / radius, 0.0)
    };
    let mut contours = vec![outline.clone()];
    if !filled || shape == EmphasisShape::DoubleCircle {
        contours.push(hollow(&outline, radius));
    }
    if shape == EmphasisShape::DoubleCircle {
        let inner = circle(centre, INNER_CIRCLE_RADIUS_EM * em);
        if !filled {
            contours.push(hollow(&inner, INNER_CIRCLE_RADIUS_EM));
        }
        contours.push(inner);
    }
    contours
}
