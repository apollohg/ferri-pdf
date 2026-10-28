use crate::{GlyphRun, Line, TextError};
use ferri_pdf_style::{Style, TextAlign};
use icu_properties::{CodePointSetData, props::DefaultIgnorableCodePoint};

const WRAP_SPACE_COMPRESSION: f32 = 1.0 / 6.0;
const MAX_SPACE_COMPRESSION: f32 = 1.0 / 3.0;

pub(super) fn word_space_text(text: &str) -> bool {
    text == " "
        || (!text.is_ascii()
            && text.trim_matches(|ch| {
                CodePointSetData::new::<DefaultIgnorableCodePoint>().contains(ch)
            }) == " ")
}

fn word_space(run: &GlyphRun, index: usize) -> bool {
    let glyph = &run.glyphs[index];
    !run.leader
        && run.white_space.collapses_spaces()
        && word_space_text(&run.text[glyph.text_range.clone()])
        && run
            .glyphs
            .get(index + 1)
            .is_none_or(|next| next.text_range != glyph.text_range)
}

fn compressible_width(runs: &[GlyphRun]) -> f32 {
    runs.iter()
        .map(|run| {
            (0..run.glyphs.len())
                .filter(|&index| word_space(run, index))
                .map(|index| run.advance(index).max(0.0))
                .sum::<f32>()
        })
        .sum()
}

pub(super) fn wrapping_width(runs: &[GlyphRun], width: f32, style: &Style) -> f32 {
    if style.align == TextAlign::Justify {
        width - compressible_width(runs) * WRAP_SPACE_COMPRESSION
    } else {
        width
    }
}

fn adjust(line: &mut Line, adjustment: impl Fn(f32) -> f32) -> Result<(), TextError> {
    for run in &mut line.runs {
        let mut change = 0.0;
        for index in 0..run.glyphs.len() {
            if word_space(run, index) {
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
                change += extra;
            }
        }
        run.width += change;
        line.width += change;
        if !run.width.is_finite() || !line.width.is_finite() {
            return Err(TextError::NonFiniteGeometry);
        }
    }
    Ok(())
}

pub(super) fn align(
    line: &mut Line,
    style: &Style,
    available: f32,
    last: bool,
) -> Result<(), TextError> {
    if line.width > available {
        line.align = TextAlign::Start;
        if style.align == TextAlign::Justify {
            let capacity = compressible_width(&line.runs);
            if capacity > 0.0 {
                let fits = line.width - capacity * WRAP_SPACE_COMPRESSION <= available;
                let ratio = if fits {
                    (line.width - available) / capacity
                } else {
                    MAX_SPACE_COMPRESSION
                };
                adjust(line, |width| -width.max(0.0) * ratio)?;
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
    let count: usize = line
        .runs
        .iter()
        .map(|run| {
            (0..run.glyphs.len())
                .filter(|&index| word_space(run, index))
                .count()
        })
        .sum();
    if count == 0 {
        line.align = match style.align_last {
            Some(TextAlign::Justify) => TextAlign::Center,
            Some(align) => align,
            None => TextAlign::Start,
        };
        return Ok(());
    }
    let expansion = (available - line.width) / count as f32;
    if !expansion.is_finite() {
        return Err(TextError::NonFiniteGeometry);
    }
    adjust(line, |_| expansion)?;
    line.width = available;
    Ok(())
}
