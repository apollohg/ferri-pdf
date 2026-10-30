//! Text decoration lines of a line box (CSS Text Decoration 3 §2). Each decorating box
//! draws its lines under the text it contains, skipping atomic inlines and the margins,
//! borders and padding of nested boxes, at a position and thickness from its own font.
use crate::document::Paragraph;
use crate::graphics::{DecorationLine, DecorationPaint, Rect};
use crate::style::{Style, TextDecoration, UnderlinePosition};
use crate::text::line_boxes::{BoxMetrics, Slot, slot_width};
use crate::text::{Line, TextContext, TextError};

/// Line thickness, and underline depth, of a font without `post` or `OS/2` metrics.
const FALLBACK_THICKNESS_EM: f32 = 0.05;
const FALLBACK_UNDERLINE_EM: f32 = 0.1;
/// Text runs this close together are decorated by one line.
const JOIN_TOLERANCE_PT: f32 = 1.0e-3;
const LINES: [DecorationLine; 3] = [
    DecorationLine::Underline,
    DecorationLine::Overline,
    DecorationLine::LineThrough,
];

/// Decoration metrics of a decorating box's first available font, in points below its baseline.
struct DecoratingFont {
    /// `[top, thickness]` of the font's underline and strikeout.
    underline: [f32; 2],
    strikeout: [f32; 2],
    ascent: f32,
    descent: f32,
}
impl DecoratingFont {
    fn of(context: &TextContext<'_>, style: &Style) -> Result<Self, TextError> {
        let instance = context
            .face(context.primary(style)?, style)?
            .ok_or(TextError::NoFontCandidates)?;
        let face = &instance.face;
        let size = instance.used_size(style);
        let em = size / face.units_per_em() as f32;
        let metrics = BoxMetrics::of(context, style)?;
        let fallback = size * FALLBACK_THICKNESS_EM;
        let line = |metrics: Option<ttf_parser::LineMetrics>, top: f32| {
            metrics
                .filter(|metrics| metrics.thickness > 0)
                .map_or([top, fallback], |metrics| {
                    [
                        -f32::from(metrics.position) * em,
                        f32::from(metrics.thickness) * em,
                    ]
                })
        };
        Ok(Self {
            underline: line(face.underline_metrics(), size * FALLBACK_UNDERLINE_EM),
            strikeout: line(
                face.strikeout_metrics(),
                -(metrics.x_height + fallback) / 2.0,
            ),
            ascent: metrics.ascent,
            descent: metrics.descent,
        })
    }
    /// `[top, thickness]` of `line` below the baseline. Underlines keep their top edge,
    /// overlines their bottom edge and line-throughs their centre when the thickness is given.
    fn place(&self, line: DecorationLine, decoration: &TextDecoration, style: &Style) -> [f32; 2] {
        let [font_top, font_thickness] = match line {
            DecorationLine::LineThrough => self.strikeout,
            _ => self.underline,
        };
        let thickness = decoration.thickness.unwrap_or(font_thickness);
        let offset = style
            .text_decorations
            .underline_offset
            .as_ref()
            .map(|offset| offset.resolve(style.size));
        let top = match line {
            DecorationLine::Underline => match style.text_decorations.underline_position {
                UnderlinePosition::Font => offset.unwrap_or(font_top),
                UnderlinePosition::Under => self.descent + offset.unwrap_or(0.0),
            },
            DecorationLine::Overline => -self.ascent - thickness,
            DecorationLine::LineThrough => font_top + (font_thickness - thickness) / 2.0,
        };
        [top, thickness]
    }
}

fn draws(decoration: &TextDecoration, line: DecorationLine) -> bool {
    match line {
        DecorationLine::Underline => decoration.lines.underline || decoration.lines.error.is_some(),
        DecorationLine::Overline => decoration.lines.overline,
        DecorationLine::LineThrough => decoration.lines.line_through,
    }
}

/// A box decorating text on a line: the paragraph's root inline box or an inline box.
struct Decorating<'a> {
    owner: Option<usize>,
    style: &'a Style,
    baseline: f32,
    decorations: Vec<&'a TextDecoration>,
}

/// Lines of a placed line, relative to its left edge and baseline: all underlines, then
/// overlines, then line-throughs, each in tree order of the decorating boxes (CSS 2.1 E.2).
pub(crate) fn paint(
    context: &TextContext<'_>,
    paragraph: &Paragraph,
    line: &Line,
) -> Result<Vec<DecorationPaint>, TextError> {
    let mut decorating = vec![Decorating {
        owner: None,
        style: paragraph.style(),
        baseline: 0.0,
        decorations: paragraph.style().text_decorations.applied().collect(),
    }];
    for fragment in &line.boxes {
        let style = &paragraph.boxes()[fragment.index].style;
        let declared = &style.text_decorations.declared;
        if declared.lines.is_empty()
            || decorating
                .last()
                .is_some_and(|last| last.owner == Some(fragment.index))
        {
            continue;
        }
        decorating.push(Decorating {
            owner: Some(fragment.index),
            style,
            baseline: fragment.baseline,
            decorations: vec![declared],
        });
    }
    decorating.retain(|decorating| !decorating.decorations.is_empty());
    let mut placed = Vec::with_capacity(decorating.len());
    for decorating in &decorating {
        placed.push((
            DecoratingFont::of(context, decorating.style)?,
            segments(paragraph, line, decorating.owner),
        ));
    }
    let mut paints = Vec::new();
    for kind in LINES {
        for (decorating, (font, segments)) in decorating.iter().zip(&placed) {
            for decoration in decorating
                .decorations
                .iter()
                .filter(|decoration| draws(decoration, kind))
            {
                let [top, thickness] = font.place(kind, decoration, decorating.style);
                let (style, color) = decoration.paint();
                paints.extend(segments.iter().map(|&([left, right], positioned)| {
                    DecorationPaint {
                        line: kind,
                        style,
                        color,
                        rect: Rect {
                            x: left,
                            y: decorating.baseline + top,
                            width: right - left,
                            height: thickness,
                        },
                        owner: decorating.owner,
                        positioned,
                    }
                }));
            }
        }
    }
    Ok(paints)
}

/// Contiguous `[left, right]` extents of the text and `text-autospace` spacing inside box
/// `owner` (`None` is the root inline box, which also holds the ellipsis), from the line's
/// left edge, each with the positioned box inside `owner` that moves it; content under
/// different such boxes is not joined.
fn segments(
    paragraph: &Paragraph,
    line: &Line,
    owner: Option<usize>,
) -> Vec<([f32; 2], Option<usize>)> {
    let content = line.ellipsis.unwrap_or(line.runs.len());
    let inside = |inner: Option<usize>| owner.is_none_or(|owner| paragraph.is_within(inner, owner));
    let mut extents: Vec<([f32; 2], Option<usize>)> = Vec::new();
    let mut x = 0.0;
    for &slot in &line.order {
        let width = slot_width(&line.runs, &line.atomics, slot);
        let decorated = match slot {
            Slot::Run(index) => {
                let inline_box = paragraph.spans()[line.runs[index].source_span].inline_box;
                ((owner.is_none() || index < content) && inside(inline_box)).then_some(inline_box)
            }
            Slot::Space { owner: inner, .. } => inside(inner).then_some(inner),
            Slot::Atomic(_) | Slot::Edge { .. } => None,
        };
        if let Some(inline_box) = decorated
            && width > 0.0
        {
            extents.push((
                [x, x + width],
                paragraph.positioned_within(inline_box, owner),
            ));
        }
        x += width;
    }
    extents.sort_by(|a, b| a.0[0].total_cmp(&b.0[0]));
    let mut joined: Vec<([f32; 2], Option<usize>)> = Vec::with_capacity(extents.len());
    for ([left, right], positioned) in extents {
        match joined.last_mut() {
            Some((last, last_positioned))
                if *last_positioned == positioned && left <= last[1] + JOIN_TOLERANCE_PT =>
            {
                last[1] = last[1].max(right);
            }
            _ => joined.push(([left, right], positioned)),
        }
    }
    joined
}
