//! CSS Writing Modes 4 §9.1: plain-text combinations, isolated horizontal composition.
use crate::document::Paragraph;
use crate::style::{
    Style,
    logical::{TextCombine, WritingMode},
};
use crate::text::{GlyphOrientation, GlyphRun, TextContext, TextError, shaping::MissingGlyphs};
use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;

const HALF_EM: f32 = 0.5;
const WIDTH_FEATURES: [[u8; 4]; 3] = [*b"hwid", *b"twid", *b"qwid"];

pub(super) struct Group {
    pub range: Range<usize>,
    pub span: usize,
}

fn common_property(
    paragraph: &Paragraph,
    a: Option<usize>,
    mut b: Option<usize>,
    ch: char,
) -> bool {
    let mut ancestors = vec![None];
    let mut a = a;
    while let Some(index) = a {
        ancestors.push(Some(index));
        a = paragraph.boxes()[index].parent;
    }
    loop {
        if ancestors.contains(&b) {
            let style = b.map_or_else(
                || paragraph.style(),
                |index| &paragraph.boxes()[index].style,
            );
            if style.text_combine.accepts(ch) {
                return true;
            }
        }
        match b {
            Some(index) => b = paragraph.boxes()[index].parent,
            None => return false,
        }
    }
}

pub(super) fn groups(paragraph: &Paragraph) -> Vec<Group> {
    let text = paragraph.text();
    let mut groups = Vec::new();
    for (span_index, span) in paragraph.spans().iter().enumerate() {
        let property = span.style.text_combine;
        if !span.style.used_writing_mode().vertical()
            || property == TextCombine::None
            || span.atomic.is_some()
        {
            continue;
        }
        let mut start = None;
        let mut candidates = Vec::new();
        for (offset, ch) in text[span.range.clone()].char_indices() {
            let offset = span.range.start + offset;
            if property.accepts(ch) {
                start.get_or_insert(offset);
            } else if let Some(start) = start.take() {
                candidates.push(start..offset);
            }
        }
        if let Some(start) = start {
            candidates.push(start..span.range.end);
        }
        for candidate in candidates {
            let candidate = crate::text::whitespace::visible(paragraph, candidate);
            if candidate.is_empty() {
                continue;
            }
            let count = text[candidate.clone()].graphemes(true).count();
            if matches!(property, TextCombine::Digits(limit) if count > usize::from(limit)) {
                continue;
            }
            let interrupted = [(candidate.start, true), (candidate.end, false)]
                .into_iter()
                .any(|(offset, before)| {
                    let adjacent = if before {
                        text[..offset].char_indices().next_back()
                    } else {
                        text[offset..]
                            .char_indices()
                            .next()
                            .map(|(index, ch)| (offset + index, ch))
                    };
                    adjacent.is_some_and(|(index, ch)| {
                        property.accepts(ch)
                            && paragraph
                                .spans()
                                .iter()
                                .find(|span| span.range.contains(&index))
                                .is_some_and(|other| {
                                    other.inline_box != span.inline_box
                                        && common_property(
                                            paragraph,
                                            span.inline_box,
                                            other.inline_box,
                                            ch,
                                        )
                                })
                    })
                });
            if !interrupted {
                groups.push(Group {
                    range: candidate,
                    span: span_index,
                });
            }
        }
    }
    groups
}

pub(super) fn constrain<T>(
    paragraph: &Paragraph,
    entries: &mut Vec<T>,
    offset: impl Fn(&T) -> usize,
) {
    let groups = groups(paragraph);
    entries.retain(|entry| {
        !groups
            .iter()
            .any(|group| group.range.start < offset(entry) && offset(entry) < group.range.end)
    });
}

fn horizontal(style: &Style) -> Style {
    let mut style = style.clone();
    style.writing_mode = WritingMode::HorizontalTb;
    style.text_combine = TextCombine::None;
    style.letter_spacing = 0.0;
    style.line_height = crate::style::LineHeight::Number(1.0);
    style
}

pub(super) fn shape(
    context: &TextContext<'_>,
    paragraph: &Paragraph,
    group: &Group,
) -> Result<Vec<GlyphRun>, TextError> {
    let source_style = &paragraph.spans()[group.span].style;
    let mut style = horizontal(source_style);
    let source = &paragraph.text()[group.range.clone()];
    let compressed = source
        .graphemes(true)
        .filter(|unit| {
            !unit
                .chars()
                .all(|ch| matches!(ch, '\n' | '\r' | '\u{2028}' | '\u{2029}'))
        })
        .take(2)
        .count()
        > 1;
    let mut text = String::new();
    let mut offsets = Vec::new();
    for (index, ch) in source.char_indices() {
        if matches!(ch, '\n' | '\r' | '\u{2028}' | '\u{2029}') {
            continue;
        }
        let ch = if compressed {
            crate::document::narrow_width(ch)
        } else {
            ch
        };
        text.push(ch);
        offsets.extend((0..ch.len_utf8()).map(|_| index));
    }
    offsets.push(source.len());
    let text = text.as_str();
    let count = text.graphemes(true).count();
    let shape = |style: &Style| -> Result<Vec<GlyphRun>, TextError> {
        let bidi = unicode_bidi::BidiInfo::new(
            text,
            Some(if style.rtl {
                unicode_bidi::Level::rtl()
            } else {
                unicode_bidi::Level::ltr()
            }),
        );
        let mut runs = Vec::new();
        if let Some(info) = bidi.paragraphs.first() {
            let (levels, visual) = bidi.visual_runs(info, 0..text.len());
            for range in visual {
                let rtl = levels[range.start].is_rtl();
                let mut scripts = super::script_ranges(text, range);
                if rtl {
                    scripts.reverse();
                }
                for range in scripts {
                    runs.extend(super::tabs::shape(context, text, range, rtl, style)?);
                }
            }
        }
        super::tabs::resolve_composition(context, style, &mut runs)?;
        Ok(runs)
    };
    let mut runs = shape(&style)?;
    if count > 1 {
        let feature = WIDTH_FEATURES[(count - 2).min(WIDTH_FEATURES.len() - 1)];
        let mut settings = style.font_features.settings.to_vec();
        settings.push(crate::fonts::FontFeature::on(feature));
        style.font_features.settings = settings.into();
        // Width-specific forms must cover every typographic unit before the whole composition uses them (§9.1.3).
        let mut complete = true;
        for cluster in text
            .graphemes(true)
            .filter(|cluster| !cluster.chars().all(char::is_whitespace))
        {
            let plain = context.shape(
                cluster,
                0..cluster.len(),
                false,
                &horizontal(source_style),
                MissingGlyphs::Substitute,
            )?;
            let variant = context.shape(
                cluster,
                0..cluster.len(),
                false,
                &style,
                MissingGlyphs::Substitute,
            )?;
            let ids = |runs: &[GlyphRun]| {
                runs.iter()
                    .flat_map(|run| run.glyphs.iter().map(|glyph| (run.font_id, glyph.id)))
                    .collect::<Vec<_>>()
            };
            if ids(&plain) == ids(&variant) {
                complete = false;
                break;
            }
        }
        if complete {
            runs = shape(&style)?;
        }
    }
    let em = source_style.size;
    let width: f32 = runs.iter().map(|run| run.width).sum();
    let scale = if width > em && width > 0.0 {
        em / width
    } else {
        1.0
    };
    let mut x = -(width * scale) * HALF_EM;
    let last = runs.len().saturating_sub(1);
    for (index, run) in runs.iter_mut().enumerate() {
        let baseline =
            em * HALF_EM + (run.metrics.ascent - run.metrics.descent) * run.font_size * HALF_EM;
        let orientation = match source_style.used_writing_mode() {
            WritingMode::VerticalLr => GlyphOrientation::UprightLr,
            WritingMode::SidewaysLr => GlyphOrientation::UprightSidewaysLr,
            _ => GlyphOrientation::UprightRl,
        };
        let [a, b, c, d] = orientation.matrix();
        for glyph in &mut run.glyphs {
            let advance = glyph.advance(run.font_size) * scale;
            let gx = x + glyph.x_offset * run.font_size * scale;
            let gy = baseline + glyph.y_offset * run.font_size;
            glyph.x_offset = if run.font_size == 0.0 {
                0.0
            } else {
                (a * gx + c * gy) / run.font_size
            };
            glyph.y_offset = if run.font_size == 0.0 {
                0.0
            } else {
                (b * gx + d * gy) / run.font_size
            };
            glyph.x_advance = 0.0;
            glyph.y_advance = 0.0;
            glyph.x_spacing = 0.0;
            x += advance;
        }
        run.width = if index == last { em } else { 0.0 };
        if index == last
            && let Some(glyph) = run.glyphs.last_mut()
        {
            glyph.x_spacing = em;
        }
        run.orientation = orientation;
        run.inline_scale = scale;
        run.combined = true;
        run.leading = Some(em);
        let instance = context
            .face(run.font_id, source_style)?
            .ok_or(TextError::NoFontCandidates)?;
        run.metrics = run.metrics.upright(&instance.face);
        run.ink_bounds = Some([-em * HALF_EM, em * HALF_EM]);
        let original = offsets[run.source_range.start]..offsets[run.source_range.end];
        for glyph in &mut run.glyphs {
            glyph.text_range = offsets[run.source_range.start + glyph.text_range.start]
                - original.start
                ..offsets[run.source_range.start + glyph.text_range.end] - original.start;
        }
        source[original.clone()].clone_into(&mut run.text);
        run.source_range = group.range.start + original.start..group.range.start + original.end;
        run.source_span = group.span;
    }
    Ok(runs)
}
