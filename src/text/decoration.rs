//! Text decoration lines of a line box (CSS Text Decoration 3 §2). Each decorating box
//! draws its lines under the text it contains, skipping what `text-decoration-skip` gives
//! (CSS Text Decoration 4 §2.10), at a position and thickness from its own font.
use crate::document::Paragraph;
use crate::graphics::{DecorationLine, DecorationPaint, Rect};
use crate::style::{DecorationInset, Style, TextDecoration};
use crate::text::line_boxes::{BoxMetrics, Slot, slot_width};
use crate::text::{Line, TextContext, TextError};

/// Line thickness, and underline depth, of a font without `post` or `OS/2` metrics.
const FALLBACK_THICKNESS_EM: f32 = 0.05;
const FALLBACK_UNDERLINE_EM: f32 = 0.1;
/// Text runs this close together are decorated by one line.
const JOIN_TOLERANCE_PT: f32 = 1.0e-3;
/// The user agent inset of `text-decoration-inset: auto` and `text-decoration-skip: edges`,
/// in line thicknesses.
const AUTO_INSET_THICKNESSES: f32 = 0.5;
/// The clearance skipped glyph ink keeps to either side, in line thicknesses.
const SKIP_INK_CLEARANCE_THICKNESSES: f32 = 1.0;
const NARROW_NO_BREAK_SPACE: char = '\u{202F}';
/// CSS Text 3 §4.1.1 word separators without the `White_Space` property.
const WORD_SEPARATORS: [char; 5] = [
    '\u{1361}',
    '\u{10100}',
    '\u{10101}',
    '\u{1039F}',
    '\u{1091F}',
];
const LINES: [DecorationLine; 3] = [
    DecorationLine::Underline,
    DecorationLine::Overline,
    DecorationLine::LineThrough,
];

/// Decoration metrics of a decorating box's first available font, in points below its baseline.
pub(crate) struct DecoratingFont {
    /// `[top, thickness]` of the font's underline and strikeout.
    pub(crate) underline: [f32; 2],
    pub(crate) strikeout: [f32; 2],
    pub(crate) ascent: f32,
    pub(crate) descent: f32,
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
    pub(crate) fn place(
        &self,
        line: DecorationLine,
        decoration: &TextDecoration,
        style: &Style,
    ) -> [f32; 2] {
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
        let position = style.text_decorations.underline_position;
        // CSS Text Decoration 4 §2.7: in vertical typographic modes `left` and `right` put
        // underlines under the text on that side; the right is the line-over side, where an
        // overline then switches to the line-under side.
        let side = position.right.filter(|_| style.vertical_typographic_mode());
        let top = match line {
            DecorationLine::Underline => match side {
                Some(true) => -self.ascent - thickness - offset.unwrap_or(0.0),
                Some(false) => self.descent + offset.unwrap_or(0.0),
                None if position.under => self.descent + offset.unwrap_or(0.0),
                None => offset.unwrap_or(font_top),
            },
            DecorationLine::Overline if side == Some(true) => self.descent,
            DecorationLine::Overline => -self.ascent - thickness,
            DecorationLine::LineThrough => font_top + (font_thickness - thickness) / 2.0,
        };
        [top, thickness]
    }
}

pub(crate) fn draws(decoration: &TextDecoration, line: DecorationLine) -> bool {
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
    let stretches = stretches(paragraph, line);
    let mut placed = Vec::with_capacity(decorating.len());
    for decorating in &decorating {
        placed.push((
            DecoratingFont::of(context, decorating.style)?,
            decorated(paragraph, line, &stretches, decorating.owner),
        ));
    }
    let mut paints = Vec::new();
    for kind in LINES {
        for (decorating, (font, decorated)) in decorating.iter().zip(&placed) {
            for decoration in decorating
                .decorations
                .iter()
                .filter(|decoration| draws(decoration, kind))
            {
                let [top, thickness] = font.place(kind, decoration, decorating.style);
                let (style, color) = decoration.paint();
                let inset = inset(paragraph, line, decorating, decoration, thickness);
                let lines = inset_segments(&decorated.segments, inset).into_iter().map(
                    |([left, right], positioned)| DecorationPaint {
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
                    },
                );
                if decoration.skip_ink && kind != DecorationLine::LineThrough {
                    let runs: Vec<_> = decorated.runs.iter().map(|&run| &line.runs[run]).collect();
                    for paint in lines {
                        paints.extend(skip_ink(context, paragraph, &runs, paint)?);
                    }
                } else {
                    paints.extend(lines);
                }
            }
        }
    }
    Ok(paints)
}

/// `paint` interrupted where the glyphs of `runs` put ink in the band it paints, with a
/// clearance to either side (CSS Text Decoration 4 §2.10.5).
fn skip_ink(
    context: &TextContext<'_>,
    paragraph: &Paragraph,
    runs: &[&crate::text::GlyphRun],
    paint: DecorationPaint,
) -> Result<Vec<DecorationPaint>, TextError> {
    let Some(band) = paint
        .fills()
        .iter()
        .filter_map(|fill| crate::paint::shape_bounds(&fill.shape))
        .reduce(Rect::union)
    else {
        return Ok(Vec::new());
    };
    let ink = crate::text::skip_ink::ink_extents(
        context,
        paragraph,
        runs,
        [band.y, band.y + band.height],
    )?;
    let clearance = paint.rect.height * SKIP_INK_CLEARANCE_THICKNESSES;
    Ok(crate::text::skip_ink::subtract(
        [paint.rect.x, paint.rect.x + paint.rect.width],
        &ink,
        clearance,
    )
    .into_iter()
    .map(|[left, right]| DecorationPaint {
        rect: Rect {
            x: left,
            width: right - left,
            ..paint.rect
        },
        ..paint
    })
    .collect())
}

/// `[left, right]` amounts by which the lines `decoration` of `decorating` start inside its
/// content edges on this line (CSS Text Decoration 4 `text-decoration-inset`): only where an
/// edge of the box lies on the line, which every fragment of a cloned box has.
fn inset(
    paragraph: &Paragraph,
    line: &Line,
    decorating: &Decorating<'_>,
    decoration: &TextDecoration,
    thickness: f32,
) -> [f32; 2] {
    let automatic = thickness * AUTO_INSET_THICKNESSES;
    let [start, end] = match decoration.inset {
        DecorationInset::Auto => [automatic; 2],
        DecorationInset::Lengths(lengths) if decoration.edges => {
            lengths.map(|length| length + automatic)
        }
        DecorationInset::Lengths(lengths) => lengths,
    };
    let rtl = decorating.style.used_rtl();
    let [left_edge, right_edge] = match decorating.owner {
        None => {
            let clone = decorating.style.clone_decoration;
            let first = clone || line.range.start == 0;
            let last = clone || line.range.end >= paragraph.text().len();
            if rtl { [last, first] } else { [first, last] }
        }
        Some(owner) => {
            let mut fragments = line.boxes.iter().filter(|fragment| fragment.index == owner);
            let first = fragments.next();
            let last = fragments.next_back().or(first);
            [
                first.is_some_and(|fragment| fragment.edges[0]),
                last.is_some_and(|fragment| fragment.edges[1]),
            ]
        }
    };
    let [left, right] = if rtl { [end, start] } else { [start, end] };
    [
        if left_edge { left } else { 0.0 },
        if right_edge { right } else { 0.0 },
    ]
}

/// `segments` with the first moved right by `left` and the last left by `right`; a segment
/// that this empties is dropped.
fn inset_segments(
    segments: &[([f32; 2], Option<usize>)],
    [left, right]: [f32; 2],
) -> Vec<([f32; 2], Option<usize>)> {
    let mut inset = segments.to_vec();
    if let Some((first, _)) = inset.first_mut() {
        first[0] += left;
    }
    if let Some((last, _)) = inset.last_mut() {
        last[1] -= right;
    }
    inset.retain(|([left, right], _)| right > left);
    inset
}

/// What a stretch of a line holds, for deciding which decorating boxes' lines cross it.
#[derive(Clone, Copy)]
enum Content {
    /// Text of the line's run `run`; `skipped` marks a spacer that the `text-decoration-skip`
    /// of its element skips.
    Text {
        run: usize,
        owner: Option<usize>,
        spacer: bool,
        skipped: bool,
    },
    /// `text-autospace` spacing.
    Space { owner: Option<usize> },
    /// An atomic inline, `skipped` under its `text-decoration-skip: objects`.
    Atomic { owner: Option<usize>, skipped: bool },
    /// The margin, border and padding of inline box `owner`, `skipped` under its
    /// `text-decoration-skip: box-decoration`.
    Edge { owner: usize, skipped: bool },
}

/// One piece of a line's content, `[left, right]` from its left edge, in visual order.
struct Stretch {
    extent: [f32; 2],
    content: Content,
}

/// The line's content split at spacers, with each piece's skipping decided by its element.
fn stretches(paragraph: &Paragraph, line: &Line) -> Vec<Stretch> {
    let mut stretches = Vec::new();
    let mut x = 0.0;
    for &slot in &line.order {
        let width = slot_width(&line.runs, &line.atomics, slot);
        let extent = [x, x + width];
        match slot {
            Slot::Run(index) => {
                let run = &line.runs[index];
                let owner = paragraph.spans()[run.source_span].inline_box;
                stretches.extend(spacer_pieces(run, x).into_iter().map(|(extent, spacer)| {
                    Stretch {
                        extent,
                        content: Content::Text {
                            run: index,
                            owner,
                            spacer,
                            skipped: false,
                        },
                    }
                }));
            }
            Slot::Space { owner, .. } => stretches.push(Stretch {
                extent,
                content: Content::Space { owner },
            }),
            Slot::Atomic(index) => {
                let atomic = &line.atomics[index];
                let skip = paragraph.atomics()[atomic.index]
                    .block
                    .style()
                    .text_decorations
                    .skip;
                stretches.push(Stretch {
                    extent,
                    content: Content::Atomic {
                        owner: atomic.owner,
                        skipped: skip.objects,
                    },
                });
            }
            Slot::Edge { owner, .. } => stretches.push(Stretch {
                extent,
                content: Content::Edge {
                    owner,
                    skipped: paragraph.boxes()[owner]
                        .style
                        .text_decorations
                        .skip
                        .box_decoration,
                },
            }),
        }
        x += width;
    }
    let skip = |run: usize| {
        paragraph.spans()[line.runs[run].source_span]
            .style
            .text_decorations
            .skip
    };
    let ascending: Vec<usize> = (0..stretches.len()).collect();
    let descending: Vec<usize> = ascending.iter().rev().copied().collect();
    let (start, end) = if paragraph.style().used_rtl() {
        (descending, ascending)
    } else {
        (ascending, descending)
    };
    for (order, at_start) in [(start, true), (end, false)] {
        for index in order {
            match &mut stretches[index].content {
                Content::Edge { .. } => {}
                Content::Text {
                    run,
                    spacer: true,
                    skipped,
                    ..
                } => {
                    let skip = skip(*run);
                    *skipped |= if at_start {
                        skip.leading_spaces
                    } else {
                        skip.trailing_spaces
                    };
                }
                _ => break,
            }
        }
    }
    for stretch in &mut stretches {
        if let Content::Text {
            run,
            spacer: true,
            skipped,
            ..
        } = &mut stretch.content
        {
            *skipped |= skip(*run).spaces;
        }
    }
    stretches
}

/// CSS Text Decoration 4 §2.10.4: a spacer is a character with the Unicode `White_Space`
/// property other than U+202F NARROW NO-BREAK SPACE, or a word separator.
fn is_spacer(character: char) -> bool {
    (character.is_whitespace() && character != NARROW_NO_BREAK_SPACE)
        || WORD_SEPARATORS.contains(&character)
}

/// The `[left, right]` extents, from the line's left edge, of the spacers and the other
/// glyphs of a run that starts at `x`. A spacer's piece takes the letter and word spacing
/// after the glyph before it too.
fn spacer_pieces(run: &crate::text::GlyphRun, x: f32) -> Vec<([f32; 2], bool)> {
    let mut pieces: Vec<([f32; 2], bool)> = Vec::new();
    let mut pen = x;
    let mut previous_spacing = 0.0;
    for glyph in run.glyphs() {
        let advance = glyph.advance(run.font_size);
        let spacer = run
            .text
            .get(glyph.text_range.clone())
            .is_some_and(|text| !text.is_empty() && text.chars().all(is_spacer));
        match pieces.last_mut() {
            Some((extent, last)) if *last == spacer => extent[1] = pen + advance,
            Some((extent, _)) => {
                let start = if spacer { pen - previous_spacing } else { pen };
                extent[1] = start;
                pieces.push(([start, pen + advance], spacer));
            }
            None => pieces.push(([pen, pen + advance], spacer)),
        }
        previous_spacing = glyph.x_spacing;
        pen += advance;
    }
    match pieces.last_mut() {
        Some((extent, _)) => extent[1] = x + run.width,
        None => pieces.push(([x, x + run.width], false)),
    }
    pieces
}

/// The extents of a line that the lines of box `owner` (`None` is the root inline box, which
/// also holds the ellipsis) cross, joined where contiguous and under the same positioned box
/// inside `owner`, which moves them; and the runs whose text lies inside the box.
struct Decorated {
    segments: Vec<([f32; 2], Option<usize>)>,
    runs: Vec<usize>,
}

fn decorated(
    paragraph: &Paragraph,
    line: &Line,
    stretches: &[Stretch],
    owner: Option<usize>,
) -> Decorated {
    let content = line.ellipsis.unwrap_or(line.runs.len());
    let inside = |inner: Option<usize>| owner.is_none_or(|owner| paragraph.is_within(inner, owner));
    let mut extents: Vec<([f32; 2], Option<usize>)> = Vec::new();
    let mut runs = Vec::new();
    for stretch in stretches {
        let crossed = match stretch.content {
            Content::Text {
                run,
                owner: inner,
                skipped,
                ..
            } => {
                let inside = (owner.is_none() || run < content) && inside(inner);
                if inside && runs.last() != Some(&run) {
                    runs.push(run);
                }
                (inside && !skipped).then_some(inner)
            }
            Content::Space { owner: inner } => inside(inner).then_some(inner),
            Content::Atomic {
                owner: inner,
                skipped,
            } => (inside(inner) && !skipped).then_some(inner),
            // A decorating box never draws over its own box decoration.
            Content::Edge {
                owner: inner,
                skipped,
            } => (Some(inner) != owner && inside(Some(inner)) && !skipped).then_some(Some(inner)),
        };
        let [left, right] = stretch.extent;
        if let Some(inline_box) = crossed
            && right > left
        {
            extents.push((
                [left, right],
                paragraph.positioned_within(inline_box, owner),
            ));
        }
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
    Decorated {
        segments: joined,
        runs,
    }
}
