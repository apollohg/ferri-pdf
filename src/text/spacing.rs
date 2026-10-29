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
type GlyphIndex = (usize, usize);

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

/// Word separators, which `auto` and `inter-word` justification expand and compress.
fn word_spaces(paragraph: &Paragraph, runs: &[GlyphRun]) -> Vec<GlyphIndex> {
    runs.iter()
        .enumerate()
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

/// CSS Text 3 §7.4 `inter-character`: the gap after each typographic character unit, in
/// visual order, except after the line's last and between two cursive letters. A gap between
/// two spans justifies by the paragraph's value. Spaces `text-autospace` removed are not units.
fn character_gaps(paragraph: &Paragraph, line: &Line) -> Vec<GlyphIndex> {
    let mut gaps = Vec::new();
    let mut previous: Option<(GlyphIndex, usize, bool)> = None;
    for slot in &line.order {
        let Slot::Run(run_index) = *slot else {
            continue;
        };
        let run = &line.runs[run_index];
        if run.leader {
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
            let cursive = cursive_letter(&run.text[glyph.text_range.clone()]);
            if let Some((end, span, previous_cursive)) = previous {
                let value = if span == run.source_span {
                    justify(paragraph, run)
                } else {
                    paragraph.style().line_breaking.justify
                };
                if value == TextJustify::InterCharacter && !(cursive && previous_cursive) {
                    gaps.push(end);
                }
            }
            previous = Some(((run_index, index), run.source_span, cursive));
        }
    }
    gaps
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
    for &(run_index, index) in glyphs {
        let run = &mut line.runs[run_index];
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
        line.width += extra;
        if !run.width.is_finite() || !line.width.is_finite() {
            return Err(TextError::NonFiniteGeometry);
        }
    }
    Ok(())
}

pub(super) fn align(
    line: &mut Line,
    paragraph: &Paragraph,
    available: f32,
    last: bool,
) -> Result<(), TextError> {
    let style = paragraph.style();
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
    let mut opportunities = word_spaces(paragraph, &line.runs);
    opportunities.extend(character_gaps(paragraph, line));
    opportunities.sort_unstable();
    opportunities.dedup();
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
