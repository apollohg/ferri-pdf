use crate::document::Paragraph;
use crate::style::{Style, TabSize};
use crate::text::{
    GlyphRun, MissingGlyphs, TextContext, TextError,
    line_boxes::{Arranged, Slot, slot_width},
};
use std::ops::Range;

const MINIMUM_TAB_CH: f32 = 0.5;

/// CSS Text 3 §4.1.2: tab stops precede alignment and use the block's content origin.
pub(super) fn resolve(
    context: &TextContext<'_>,
    paragraph: &Paragraph,
    arranged: &mut Arranged,
    origin: f32,
) -> Result<(), TextError> {
    if !arranged.runs.iter().any(is_tab) {
        return Ok(());
    }
    let block = paragraph.style();
    let stops = Stops::new(context, block)?;
    let mut position = origin;
    for index in 0..arranged.order.len() {
        let index = if block.used_rtl() {
            arranged.order.len() - 1 - index
        } else {
            index
        };
        let slot = arranged.order[index];
        if let Slot::Run(index) = slot {
            let run = &mut arranged.runs[index];
            if is_tab(run) {
                let style = &paragraph.spans()[run.source_span].style;
                stops.apply(run, style, position)?;
            }
        }
        position += slot_width(&arranged.runs, &arranged.atomics, slot);
    }
    arranged.width = position - origin;
    if !arranged.width.is_finite() {
        return Err(TextError::NonFiniteGeometry);
    }
    Ok(())
}

/// The width of a tab `position` from its line's start: to the next stop `interval` apart, past
/// any stop nearer than half of `ch`, the advance of "0" (CSS Text 3 §4.2).
pub(crate) fn tab_width(interval: f32, position: f32, ch: f32) -> f32 {
    let mut width = interval - position.rem_euclid(interval);
    if width < MINIMUM_TAB_CH * ch {
        width += interval;
    }
    width
}

pub(super) fn is_tab(run: &GlyphRun) -> bool {
    !run.combined && run.text == "\t"
}

struct Stops<'c, 'f> {
    context: &'c TextContext<'f>,
    space: f32,
}
impl<'c, 'f> Stops<'c, 'f> {
    fn new(context: &'c TextContext<'f>, block: &Style) -> Result<Self, TextError> {
        let space: f32 = context
            .shape(
                " ",
                0..1,
                block.used_rtl(),
                block,
                MissingGlyphs::Substitute,
            )?
            .iter()
            .map(|run| run.width)
            .sum();
        Ok(Self { context, space })
    }
    fn apply(&self, run: &mut GlyphRun, style: &Style, position: f32) -> Result<(), TextError> {
        let interval = match style.tab_size {
            TabSize::Number(number) => number * self.space,
            TabSize::Length(length) => length,
        };
        let width = if interval <= 0.0 {
            0.0
        } else {
            let vertical = style.vertical_font_units()[0];
            let ch = self
                .context
                .fonts
                .advance(
                    style.font_candidates(),
                    style.font_selection(),
                    &style.font_adjustments,
                    style.size,
                    (crate::fonts::ZERO, vertical),
                    self.context.resolver,
                )?
                .map_or(
                    if vertical {
                        style.size
                    } else {
                        crate::fonts::FALLBACK_ZERO_ADVANCE_EM * style.size
                    },
                    |(advance, scale)| advance * scale * style.size,
                );
            tab_width(interval, position, ch)
        };
        if !width.is_finite() {
            return Err(TextError::NonFiniteGeometry);
        }
        for glyph in &mut run.glyphs {
            glyph.x_advance = 0.0;
            glyph.x_spacing = 0.0;
        }
        if let Some(glyph) = run.glyphs.last_mut() {
            glyph.x_spacing = width;
            if run.font_size != 0.0 && glyph.normalized_advance(run.font_size).is_none() {
                return Err(TextError::NonFiniteGeometry);
            }
        }
        run.width = width;
        Ok(())
    }
}

pub(super) fn resolve_composition(
    context: &TextContext<'_>,
    style: &Style,
    runs: &mut [GlyphRun],
) -> Result<(), TextError> {
    if !runs.iter().any(is_tab) {
        return Ok(());
    }
    let stops = Stops::new(context, style)?;
    let mut position = 0.0;
    for index in 0..runs.len() {
        let index = if style.used_rtl() {
            runs.len() - 1 - index
        } else {
            index
        };
        let run = &mut runs[index];
        if is_tab(run) {
            stops.apply(run, style, position)?;
        }
        position += run.width;
    }
    if !position.is_finite() {
        return Err(TextError::NonFiniteGeometry);
    }
    Ok(())
}

/// Shape retained tabs with a blank glyph, preserving their source identity for layout.
pub(super) fn shape(
    context: &TextContext<'_>,
    text: &str,
    range: Range<usize>,
    rtl: bool,
    style: &Style,
) -> Result<Vec<GlyphRun>, TextError> {
    if !text[range.clone()].contains('\t') {
        return context.shape(text, range, rtl, style, MissingGlyphs::Substitute);
    }
    let mut boundaries = vec![range.start, range.end];
    for (offset, _) in text[range.clone()].match_indices('\t') {
        boundaries.extend([range.start + offset, range.start + offset + 1]);
    }
    boundaries.sort_unstable();
    boundaries.dedup();
    let mut ranges: Vec<_> = boundaries.windows(2).map(|b| b[0]..b[1]).collect();
    if rtl {
        ranges.reverse();
    }
    let mut result = Vec::new();
    for range in ranges {
        let tab = &text[range.clone()] == "\t";
        let mut runs = if tab {
            context.shape(" ", 0..1, rtl, style, MissingGlyphs::Substitute)?
        } else {
            context.shape(text, range.clone(), rtl, style, MissingGlyphs::Substitute)?
        };
        if tab {
            for run in &mut runs {
                run.text = "\t".into();
                run.source_range.start += range.start;
                run.source_range.end += range.start;
            }
        }
        result.extend(runs);
    }
    Ok(result)
}
