use crate::document::Paragraph;
use crate::style::{TextAlign, TextJustify};
use crate::text::line_boxes::Slot;
use crate::text::{GlyphRun, Line, TextError};
use icu_properties::{
    CodePointMapData, CodePointSetData,
    props::{DefaultIgnorableCodePoint, GeneralCategory, GeneralCategoryGroup},
};
use unicode_script::{Script, UnicodeScript};

const WRAP_SPACE_COMPRESSION: f32 = 1.0 / 6.0;
const MAX_SPACE_COMPRESSION: f32 = 1.0 / 3.0;
/// CSS Text 3 §7.3.4: scripts whose letters join, so justification must not separate them.
const CURSIVE_SCRIPTS: [Script; 12] = [
    Script::Arabic,
    Script::Syriac,
    Script::Nko,
    Script::Mongolian,
    Script::Mandaic,
    Script::Adlam,
    Script::Hanifi_Rohingya,
    Script::Sogdian,
    Script::Old_Uyghur,
    Script::Manichaean,
    Script::Psalter_Pahlavi,
    Script::Phags_Pa,
];

/// A glyph by its run and its index within the run.
pub(super) type GlyphIndex = (usize, usize);

pub(super) fn word_space_text(text: &str) -> bool {
    text == " "
        || (!text.is_ascii()
            && text.trim_matches(|ch| {
                CodePointSetData::new::<DefaultIgnorableCodePoint>().contains(ch)
            }) == " ")
}

fn justify(paragraph: &Paragraph, run: &GlyphRun) -> TextJustify {
    paragraph.spans()[run.source_span]
        .style
        .line_breaking
        .justify
}

fn word_space(paragraph: &Paragraph, run: &GlyphRun, index: usize) -> bool {
    let glyph = &run.glyphs[index];
    !run.leader
        && run.white_space.collapses_spaces()
        && matches!(
            justify(paragraph, run),
            TextJustify::Auto | TextJustify::InterWord
        )
        && word_space_text(&run.text[glyph.text_range.clone()])
        && !run.removed_spaces.contains(&glyph.text_range.start)
        && run
            .glyphs
            .get(index + 1)
            .is_none_or(|next| next.text_range != glyph.text_range)
}

/// Justification after the final tab preserves all tab stops (CSS Text 3 §6.1).
fn justification_range(paragraph: &Paragraph, runs: &[GlyphRun]) -> std::ops::Range<usize> {
    if paragraph.style().used_rtl() {
        0..runs
            .iter()
            .position(super::tabs::is_tab)
            .unwrap_or(runs.len())
    } else {
        runs.iter()
            .rposition(super::tabs::is_tab)
            .map_or(0, |index| index + 1)..runs.len()
    }
}

/// Word separators, which `auto` and `inter-word` justification expand and compress.
fn word_spaces(paragraph: &Paragraph, runs: &[GlyphRun]) -> Vec<GlyphIndex> {
    let eligible = justification_range(paragraph, runs);
    runs.iter()
        .enumerate()
        .filter(|(index, _)| eligible.contains(index))
        .flat_map(|(run_index, run)| {
            (0..run.glyphs.len())
                .filter(move |&index| word_space(paragraph, run, index))
                .map(move |index| (run_index, index))
        })
        .collect()
}

/// A cursive letter, which joins its neighbours.
fn cursive_letter(text: &str) -> bool {
    text.chars().next().is_some_and(|ch| {
        GeneralCategoryGroup::Letter.contains(CodePointMapData::<GeneralCategory>::new().get(ch))
            && CURSIVE_SCRIPTS.contains(&ch.script())
    })
}

/// A CJK character other than Bopomofo, between which `text-justify: ruby` justifies (CSS
/// Text 4 §7.5, CSS Ruby 1 §4.3).
fn ruby_justified(text: &str) -> bool {
    use icu_properties::props::EastAsianWidth;
    text.chars().next().is_some_and(|ch| match ch.script() {
        Script::Han | Script::Hiragana | Script::Katakana | Script::Hangul => true,
        Script::Common | Script::Inherited => matches!(
            CodePointMapData::<EastAsianWidth>::new().get(ch),
            EastAsianWidth::Wide | EastAsianWidth::Fullwidth
        ),
        _ => false,
    })
}

/// Whether the gap between typographic character units `before` and `after` justifies under
/// `value`: `inter-character` everywhere but between cursive letters (CSS Text 3 §7.4), `ruby`
/// between CJK characters.
pub(super) fn justifies_between(value: TextJustify, before: &str, after: &str) -> bool {
    match value {
        TextJustify::InterCharacter => !(cursive_letter(before) && cursive_letter(after)),
        TextJustify::Ruby => ruby_justified(before) && ruby_justified(after),
        TextJustify::Auto | TextJustify::None | TextJustify::InterWord => false,
    }
}

/// The gap after each typographic character unit of `slots`, in visual order, where its
/// `text-justify` justifies it, except after the last. A gap between two spans justifies by
/// the paragraph's value. Spaces `text-autospace` removed are not units.
fn character_gaps(paragraph: &Paragraph, runs: &[GlyphRun], slots: &[Slot]) -> Vec<GlyphIndex> {
    let mut gaps = Vec::new();
    let mut previous: Option<(GlyphIndex, usize, &str)> = None;
    for slot in slots {
        let Slot::Run(run_index) = *slot else {
            continue;
        };
        let run = &runs[run_index];
        if run.leader || super::tabs::is_tab(run) {
            previous = None;
            continue;
        }
        for (index, glyph) in run.glyphs.iter().enumerate() {
            let ends_cluster = run
                .glyphs
                .get(index + 1)
                .is_none_or(|next| next.text_range != glyph.text_range);
            if !ends_cluster || run.removed_spaces.contains(&glyph.text_range.start) {
                continue;
            }
            let unit = &run.text[glyph.text_range.clone()];
            if let Some((end, span, before)) = previous {
                let value = if span == run.source_span {
                    justify(paragraph, run)
                } else {
                    paragraph.style().line_breaking.justify
                };
                if justifies_between(value, before, unit) {
                    gaps.push(end);
                }
            }
            previous = Some(((run_index, index), run.source_span, unit));
        }
    }
    gaps
}

/// The justification opportunities of `slots`, each the glyph whose advance expands: word
/// separators, then the gaps between characters, in glyph order.
pub(super) fn opportunities(
    paragraph: &Paragraph,
    runs: &[GlyphRun],
    slots: &[Slot],
) -> Vec<GlyphIndex> {
    let mut opportunities: Vec<GlyphIndex> = slots
        .iter()
        .filter_map(|slot| match *slot {
            Slot::Run(index) => Some(index),
            _ => None,
        })
        .flat_map(|run_index| {
            (0..runs[run_index].glyphs.len())
                .filter(move |&index| word_space(paragraph, &runs[run_index], index))
                .map(move |index| (run_index, index))
        })
        .collect();
    opportunities.extend(character_gaps(paragraph, runs, slots));
    let eligible = justification_range(paragraph, runs);
    opportunities.retain(|(run, _)| eligible.contains(run));
    opportunities.sort_unstable();
    opportunities.dedup();
    opportunities
}

fn compressible_width(paragraph: &Paragraph, runs: &[GlyphRun]) -> f32 {
    word_spaces(paragraph, runs)
        .into_iter()
        .map(|(run, index)| runs[run].advance(index).max(0.0))
        .sum()
}

pub(super) fn wrapping_width(paragraph: &Paragraph, runs: &[GlyphRun], width: f32) -> f32 {
    if paragraph.style().align == TextAlign::Justify {
        width - compressible_width(paragraph, runs) * WRAP_SPACE_COMPRESSION
    } else {
        width
    }
}

fn adjust(
    line: &mut Line,
    glyphs: &[GlyphIndex],
    adjustment: impl Fn(f32) -> f32,
) -> Result<(), TextError> {
    let extra = expand(&mut line.runs, glyphs, adjustment)?;
    line.width += extra;
    if !line.width.is_finite() {
        return Err(TextError::NonFiniteGeometry);
    }
    Ok(())
}

/// Adds the spacing `adjustment` gives each glyph's advance to it, returning their sum.
pub(super) fn expand(
    runs: &mut [GlyphRun],
    glyphs: &[GlyphIndex],
    adjustment: impl Fn(f32) -> f32,
) -> Result<f32, TextError> {
    let mut total = 0.0;
    for &(run_index, index) in glyphs {
        let run = &mut runs[run_index];
        let extra = adjustment(run.advance(index));
        run.glyphs[index].x_spacing += extra;
        if !run.glyphs[index].x_spacing.is_finite()
            || (run.font_size != 0.0
                && run.glyphs[index]
                    .normalized_advance(run.font_size)
                    .is_none())
        {
            return Err(TextError::NonFiniteGeometry);
        }
        run.width += extra;
        total += extra;
        if !run.width.is_finite() {
            return Err(TextError::NonFiniteGeometry);
        }
    }
    Ok(total)
}

pub(super) fn align(
    line: &mut Line,
    paragraph: &Paragraph,
    available: f32,
    last: bool,
) -> Result<(), TextError> {
    let style = paragraph.style();
    if let Some(align) = paragraph.annotation_align()
        && line.width <= available
    {
        let opportunities = match align {
            crate::style::RubyAlign::SpaceBetween | crate::style::RubyAlign::SpaceAround => {
                opportunities(paragraph, &line.runs, &line.order)
            }
            crate::style::RubyAlign::Start | crate::style::RubyAlign::Center => Vec::new(),
        };
        let (leading, _, each) =
            super::ruby::distribution(align, available - line.width, opportunities.len());
        adjust(line, &opportunities, |_| each)?;
        line.indent += leading;
        line.align = TextAlign::Start;
        return Ok(());
    }
    if line.width > available {
        line.align = TextAlign::Start;
        if style.align == TextAlign::Justify {
            let spaces = word_spaces(paragraph, &line.runs);
            let capacity = compressible_width(paragraph, &line.runs);
            if capacity > 0.0 {
                let fits = line.width - capacity * WRAP_SPACE_COMPRESSION <= available;
                let ratio = if fits {
                    (line.width - available) / capacity
                } else {
                    MAX_SPACE_COMPRESSION
                };
                adjust(line, &spaces, |width| -width.max(0.0) * ratio)?;
                if fits {
                    line.width = available;
                }
            }
        }
        return Ok(());
    }
    line.align = if last {
        style
            .align_last
            .unwrap_or(if style.align == TextAlign::Justify {
                TextAlign::Start
            } else {
                style.align
            })
    } else {
        style.align
    };
    if line.align != TextAlign::Justify || line.width >= available {
        return Ok(());
    }
    let opportunities = opportunities(paragraph, &line.runs, &line.order);
    if opportunities.is_empty() {
        line.align = match style.align_last {
            Some(TextAlign::Justify) => TextAlign::Center,
            Some(align) => align,
            None => TextAlign::Start,
        };
        return Ok(());
    }
    let expansion = (available - line.width) / opportunities.len() as f32;
    if !expansion.is_finite() {
        return Err(TextError::NonFiniteGeometry);
    }
    adjust(line, &opportunities, |_| expansion)?;
    line.width = available;
    Ok(())
}
