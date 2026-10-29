//! CSS Text 4 §8.5 `text-spacing-trim`: fullwidth punctuation set half-width at the edges of
//! lines, beside other punctuation (§8.5.1) or everywhere, by kerning away the blank half
//! of its glyph. Positions within the line follow the logical order of its characters.
use crate::document::Paragraph;
use crate::fonts::SizeMetric;
use crate::style::TextSpacingTrim;
use crate::text::hanging::IDEOGRAPHIC_SPACE;
use crate::text::line_boxes::Arranged;
use crate::text::{GlyphRun, TextContext, TextError, shaping_language};
use icu_properties::{
    CodePointMapData,
    props::{EastAsianWidth, GeneralCategory},
};
use rustybuzz::{
    Direction, Face, Feature, Language, UnicodeBuffer,
    ttf_parser::{GlyphId, Tag},
};
use std::ops::{Range, RangeInclusive};

const CJK_SYMBOLS_AND_PUNCTUATION: RangeInclusive<char> = '\u{3000}'..='\u{303f}';
const OPENING_QUOTES: [char; 2] = ['\u{2018}', '\u{201c}'];
const CLOSING_QUOTES: [char; 2] = ['\u{2019}', '\u{201d}'];
const MIDDLE_DOTS: [char; 3] = ['\u{b7}', '\u{2027}', '\u{30fb}'];
/// §8.5.2 fullwidth colon and dot punctuation, which is closing punctuation where its glyph
/// is drawn in the left half and middle dot punctuation otherwise.
const COLONS_AND_DOTS: [char; 6] = [
    '\u{ff1a}', '\u{ff1b}', '\u{3001}', '\u{3002}', '\u{ff0c}', '\u{ff0e}',
];
/// The OpenType alternate half widths feature, which trims fullwidth punctuation (§8.5.1).
const HALT: [u8; 4] = *b"halt";
/// A full-width glyph's advance may differ from the ideographic advance by this much, in em.
const FULL_WIDTH_TOLERANCE_EM: f32 = 1.0e-3;
/// The part of a full-width advance trimmed from one side, or from both sides of a middle dot.
const HALF: f32 = 0.5;
const QUARTER: f32 = 0.25;

/// §8.5.2 character classes, and the general categories the collapsing rules also consult.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Class {
    Opening,
    Closing,
    MiddleDot,
    IdeographicSpace,
    Other { opens: bool, closes: bool },
}

/// How closing punctuation that ends a line is set.
#[derive(Clone, Copy, PartialEq, Eq)]
enum EndTrim {
    Never,
    /// Half-width only where the line does not otherwise fit before justification.
    Overflow,
    Always,
}

/// Whether opening punctuation that starts a line is set half-width; `first` is the first
/// line of the block container or one after a forced line break.
fn trims_start(trim: TextSpacingTrim, first: bool) -> bool {
    match trim {
        TextSpacingTrim::TrimStart | TextSpacingTrim::TrimBoth | TextSpacingTrim::TrimAll => true,
        // `auto`, the UA's choice: `trim-both`, keeping the first line's opening spacing as
        // `space-first` does for content that indents with it.
        TextSpacingTrim::SpaceFirst | TextSpacingTrim::Auto => !first,
        TextSpacingTrim::SpaceAll | TextSpacingTrim::Normal => false,
    }
}

fn end_trim(trim: TextSpacingTrim) -> EndTrim {
    match trim {
        TextSpacingTrim::SpaceAll => EndTrim::Never,
        TextSpacingTrim::Normal | TextSpacingTrim::SpaceFirst | TextSpacingTrim::TrimStart => {
            EndTrim::Overflow
        }
        TextSpacingTrim::TrimBoth | TextSpacingTrim::TrimAll | TextSpacingTrim::Auto => {
            EndTrim::Always
        }
    }
}

/// A glyph cluster adjustment in em: added to each glyph offset and to the last advance.
#[derive(Clone, Copy)]
struct Kerning {
    offset: f32,
    advance: f32,
}

/// A typographic character unit or atomic inline of a line.
struct Unit {
    /// The run and its glyphs; `None` for an atomic inline.
    glyphs: Option<(usize, Range<usize>)>,
    offset: usize,
    class: Class,
    size: f32,
    trim: TextSpacingTrim,
}

/// The `[left, right]` ink extent of `glyphs` in em from their cluster origin, if any.
fn ink(face: &Face<'_>, run: &GlyphRun, glyphs: Range<usize>) -> Option<[f32; 2]> {
    let upem = face.units_per_em() as f32;
    let mut pen = 0.0;
    let mut extent: Option<[f32; 2]> = None;
    for glyph in &run.glyphs[glyphs] {
        if let Some(bounds) = u16::try_from(glyph.id)
            .ok()
            .and_then(|id| face.glyph_bounding_box(GlyphId(id)))
        {
            let origin = pen + glyph.x_offset;
            let [left, right] = [bounds.x_min, bounds.x_max].map(|x| origin + f32::from(x) / upem);
            extent = Some(extent.map_or([left, right], |[a, b]| [a.min(left), b.max(right)]));
        }
        pen += glyph.x_advance;
    }
    extent
}

fn class(
    context: &TextContext<'_>,
    paragraph: &Paragraph,
    run: &GlyphRun,
    glyphs: Range<usize>,
    base: char,
) -> Result<Class, TextError> {
    if base == IDEOGRAPHIC_SPACE {
        return Ok(Class::IdeographicSpace);
    }
    if MIDDLE_DOTS.contains(&base) {
        return Ok(Class::MiddleDot);
    }
    if COLONS_AND_DOTS.contains(&base) {
        let instance = context
            .face(run.font_id, &paragraph.spans()[run.source_span].style)?
            .ok_or(TextError::NoFontCandidates)?;
        let advance: f32 = run.glyphs[glyphs.clone()]
            .iter()
            .map(|glyph| glyph.x_advance)
            .sum();
        let left_half =
            ink(&instance.face, run, glyphs).is_some_and(|[_, right]| right <= advance * HALF);
        return Ok(if left_half {
            Class::Closing
        } else {
            Class::MiddleDot
        });
    }
    let category = CodePointMapData::<GeneralCategory>::new().get(base);
    let fullwidth = CJK_SYMBOLS_AND_PUNCTUATION.contains(&base)
        || CodePointMapData::<EastAsianWidth>::new().get(base) == EastAsianWidth::Fullwidth;
    Ok(
        if OPENING_QUOTES.contains(&base)
            || (fullwidth && category == GeneralCategory::OpenPunctuation)
        {
            Class::Opening
        } else if CLOSING_QUOTES.contains(&base)
            || (fullwidth && category == GeneralCategory::ClosePunctuation)
        {
            Class::Closing
        } else {
            Class::Other {
                opens: category == GeneralCategory::OpenPunctuation,
                closes: category == GeneralCategory::ClosePunctuation,
            }
        },
    )
}

/// The kerning `halt` gives `cluster`, in em. There is none where `halt` changes nothing, or
/// where the run's `advance` differs from the default one because other features apply.
fn halt(
    face: &Face<'_>,
    cluster: &str,
    rtl: bool,
    language: Option<&Language>,
    advance: f32,
) -> Option<Kerning> {
    let shape = |features: &[Feature]| {
        let mut buffer = UnicodeBuffer::new();
        buffer.push_str(cluster);
        buffer.set_direction(if rtl {
            Direction::RightToLeft
        } else {
            Direction::LeftToRight
        });
        if let Some(language) = language {
            buffer.set_language(language.clone());
        }
        buffer.guess_segment_properties();
        rustybuzz::shape(face, features, buffer)
    };
    let plain = shape(&[]);
    let halved = shape(&[Feature::new(Tag::from_bytes(&HALT), 1, ..)]);
    let upem = face.units_per_em() as f32;
    let total = |shaped: &rustybuzz::GlyphBuffer| {
        shaped
            .glyph_positions()
            .iter()
            .map(|position| position.x_advance as f32)
            .sum::<f32>()
            / upem
    };
    let [plain_offset, halved_offset] = [&plain, &halved].map(|shaped| {
        shaped
            .glyph_positions()
            .first()
            .map_or(0.0, |position| position.x_offset as f32 / upem)
    });
    let kerning = Kerning {
        offset: halved_offset - plain_offset,
        advance: total(&halved) - total(&plain),
    };
    ((advance - total(&plain)).abs() <= FULL_WIDTH_TOLERANCE_EM
        && (kerning.offset != 0.0 || kerning.advance != 0.0))
        .then_some(kerning)
}

/// How setting the punctuation of `glyphs` half-width kerns it: with the font's `halt`
/// feature, else by trimming its blank half (§8.5.1). Proportional glyphs, which are both
/// full-width and half-width, and glyphs whose ink the trim would cut, are left as they are.
fn kerning(
    context: &TextContext<'_>,
    paragraph: &Paragraph,
    run: &GlyphRun,
    glyphs: Range<usize>,
    class: Class,
) -> Result<Option<Kerning>, TextError> {
    let style = &paragraph.spans()[run.source_span].style;
    let instance = context
        .face(run.font_id, style)?
        .ok_or(TextError::NoFontCandidates)?;
    let face = &instance.face;
    let advance: f32 = run.glyphs[glyphs.clone()]
        .iter()
        .map(|glyph| glyph.x_advance)
        .sum();
    if crate::fonts::has_feature(face, HALT) {
        let cluster = &run.text[run.glyphs[glyphs.start].text_range.clone()];
        return Ok(halt(
            face,
            cluster,
            run.rtl,
            shaping_language(face, style).as_ref(),
            advance,
        ));
    }
    let full = run.metrics.size_metric(SizeMetric::IcWidth);
    if (advance - full).abs() > FULL_WIDTH_TOLERANCE_EM {
        return Ok(None);
    }
    // Opening punctuation is blank on its inline-start side and closing on its inline end.
    let [left, right] = match (class, run.rtl) {
        (Class::Opening, false) | (Class::Closing, true) => [HALF, 0.0],
        (Class::Opening, true) | (Class::Closing, false) => [0.0, HALF],
        (Class::MiddleDot, _) => [QUARTER, QUARTER],
        (Class::IdeographicSpace | Class::Other { .. }, _) => return Ok(None),
    }
    .map(|part| part * full);
    let blank = ink(face, run, glyphs)
        .is_none_or(|[ink_left, ink_right]| ink_left >= left && ink_right <= advance - right);
    Ok(blank.then_some(Kerning {
        offset: -left,
        advance: -(left + right),
    }))
}

fn kern(arranged: &mut Arranged, run: usize, glyphs: Range<usize>, kerning: Kerning) {
    let run = &mut arranged.runs[run];
    for glyph in &mut run.glyphs[glyphs.clone()] {
        glyph.x_offset += kerning.offset;
    }
    run.glyphs[glyphs.end - 1].x_advance += kerning.advance;
    let width = kerning.advance * run.font_size;
    run.width += width;
    arranged.width += width;
}

/// Closing punctuation ending a line, kerned only where the line does not otherwise fit.
pub(super) struct LineEndTrim {
    run: usize,
    glyphs: Range<usize>,
    kerning: Kerning,
}
impl LineEndTrim {
    pub(super) fn apply(self, arranged: &mut Arranged) {
        kern(arranged, self.run, self.glyphs, self.kerning);
    }
}

/// The units of an arranged line in logical order.
fn units(
    context: &TextContext<'_>,
    paragraph: &Paragraph,
    arranged: &Arranged,
) -> Result<Vec<Unit>, TextError> {
    let mut units = Vec::new();
    for (index, run) in arranged.runs.iter().enumerate() {
        let style = &paragraph.spans()[run.source_span].style;
        let mut start = 0;
        while start < run.glyphs.len() {
            let range = run.glyphs[start].text_range.clone();
            let end = start
                + run.glyphs[start..]
                    .iter()
                    .take_while(|glyph| glyph.text_range == range)
                    .count();
            if let Some(base) = run.text[range.clone()].chars().next() {
                units.push(Unit {
                    glyphs: Some((index, start..end)),
                    offset: run.source_range.start + range.start,
                    class: class(context, paragraph, run, start..end, base)?,
                    size: style.size,
                    trim: style.text_spacing.trim,
                });
            }
            start = end;
        }
    }
    units.extend(arranged.atomics.iter().map(|atomic| {
        let style = paragraph.style_at(atomic.offset);
        Unit {
            glyphs: None,
            offset: atomic.offset,
            class: Class::Other {
                opens: false,
                closes: false,
            },
            size: style.size,
            trim: style.text_spacing.trim,
        }
    }));
    units.sort_by_key(|unit| unit.offset);
    Ok(units)
}

/// §8.5.1: opening punctuation after these is set half-width.
fn collapses_opening(previous: &Unit, opening: &Unit) -> bool {
    match previous.class {
        Class::Opening | Class::MiddleDot | Class::IdeographicSpace => true,
        Class::Closing => previous.size >= opening.size,
        Class::Other { opens, .. } => opens,
    }
}

/// §8.5.1: closing punctuation before these is set half-width.
fn collapses_closing(closing: &Unit, next: &Unit) -> bool {
    match next.class {
        Class::Closing | Class::MiddleDot | Class::IdeographicSpace => true,
        Class::Opening => next.size > closing.size,
        Class::Other { closes, .. } => closes,
    }
}

/// Sets the fullwidth punctuation of an arranged line half-width where its
/// `text-spacing-trim` requires, returning the line-end trim that applies only on overflow.
/// `first` is the block container's first line or a line after a forced break.
pub(super) fn trim(
    context: &TextContext<'_>,
    paragraph: &Paragraph,
    arranged: &mut Arranged,
    first: bool,
) -> Result<Option<LineEndTrim>, TextError> {
    // Fullwidth punctuation, middle dots and ideographic spaces are all outside ASCII.
    if arranged.runs.iter().all(|run| run.text.is_ascii())
        || paragraph
            .spans()
            .iter()
            .all(|span| span.style.text_spacing.trim == TextSpacingTrim::SpaceAll)
    {
        return Ok(None);
    }
    let units = units(context, paragraph, arranged)?;
    let mut trimmed = Vec::new();
    let mut overflow = None;
    for (index, unit) in units.iter().enumerate() {
        let previous = index.checked_sub(1).map(|previous| &units[previous]);
        let next = units.get(index + 1);
        let half = match (unit.trim, unit.class) {
            (TextSpacingTrim::SpaceAll, _) | (_, Class::IdeographicSpace | Class::Other { .. }) => {
                false
            }
            (TextSpacingTrim::TrimAll, _) => true,
            (_, Class::MiddleDot) => false,
            (trim, Class::Opening) => previous.map_or_else(
                || trims_start(trim, first),
                |previous| collapses_opening(previous, unit),
            ),
            (trim, Class::Closing) => match next {
                Some(next) => collapses_closing(unit, next),
                None => match end_trim(trim) {
                    EndTrim::Always => true,
                    EndTrim::Overflow => {
                        overflow = Some(unit);
                        false
                    }
                    EndTrim::Never => false,
                },
            },
        };
        if half {
            trimmed.push(unit);
        }
    }
    for unit in trimmed {
        let Some((run, glyphs)) = unit.glyphs.clone() else {
            continue;
        };
        if let Some(kerning) = kerning(
            context,
            paragraph,
            &arranged.runs[run],
            glyphs.clone(),
            unit.class,
        )? {
            kern(arranged, run, glyphs, kerning);
        }
    }
    let Some((run, glyphs)) = overflow.and_then(|unit| unit.glyphs.clone()) else {
        return Ok(None);
    };
    Ok(kerning(
        context,
        paragraph,
        &arranged.runs[run],
        glyphs.clone(),
        Class::Closing,
    )?
    .map(|kerning| LineEndTrim {
        run,
        glyphs,
        kerning,
    }))
}
