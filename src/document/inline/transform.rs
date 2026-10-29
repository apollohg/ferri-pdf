use super::{MAX_INLINE_TEXT_BYTES, Paragraph};
use crate::document::DocumentError;
use crate::style::{TextCase, TextTransform};
use icu_casemap::{CaseMapper, ContextCase};
use icu_locale_core::LanguageIdentifier;
use std::fmt::{self, Write};
use unicode_segmentation::UnicodeSegmentation;

struct BoundedText(String);
impl Write for BoundedText {
    fn write_str(&mut self, value: &str) -> fmt::Result {
        if self
            .0
            .len()
            .checked_add(value.len())
            .is_none_or(|len| len > MAX_INLINE_TEXT_BYTES)
        {
            return Err(fmt::Error);
        }
        self.0.push_str(value);
        Ok(())
    }
}

struct Transformed<'a> {
    output: &'a mut BoundedText,
    transform: TextTransform,
    preserve_spaces: bool,
}
impl Write for Transformed<'_> {
    fn write_str(&mut self, value: &str) -> fmt::Result {
        if !self.transform.full_width && !self.transform.full_size_kana {
            return self.output.write_str(value);
        }
        for mut ch in value.chars() {
            if self.transform.full_width && (ch != ' ' || self.preserve_spaces) {
                ch = super::transform_maps::full_width(ch);
            }
            if self.transform.full_size_kana {
                ch = super::transform_maps::full_size_kana(ch);
            }
            self.output.write_char(ch)?;
        }
        Ok(())
    }
}

pub(super) fn apply(paragraph: &mut Paragraph) -> Result<(), DocumentError> {
    if paragraph
        .spans
        .iter()
        .all(|span| span.style.text_transform == TextTransform::default())
    {
        return Ok(());
    }
    let mapper = CaseMapper::new();
    let expanded_tabs = paragraph.expanded_tabs.clone();
    let original = std::mem::take(&mut paragraph.text);
    let capitalize = paragraph
        .spans
        .iter()
        .any(|span| span.style.text_transform.case == TextCase::Capitalize);
    let mut words = original
        .unicode_word_indices()
        .filter_map(|(start, word)| {
            let (lead, _) = word.char_indices().find(|(_, ch)| ch.is_alphanumeric())?;
            Some(start + lead)
        })
        .peekable();
    let mut prefix = 0..0;
    let mut context = mapper.context(&original);
    let mut output = BoundedText(String::with_capacity(original.len()));
    let mut offsets: Vec<&mut usize> = paragraph
        .anchors
        .iter_mut()
        .chain(&mut paragraph.atomics)
        .chain(&mut paragraph.annotations)
        .map(|anchor| &mut anchor.offset)
        .chain(paragraph.optional_breaks.iter_mut())
        .chain(paragraph.events.iter_mut().map(|event| &mut event.offset))
        .chain(
            paragraph
                .clearances
                .iter_mut()
                .map(|clearance| &mut clearance.0),
        )
        .chain(
            paragraph
                .boxes
                .iter_mut()
                .flat_map(|inline_box| [&mut inline_box.range.start, &mut inline_box.range.end]),
        )
        .chain(
            paragraph
                .expanded_tabs
                .iter_mut()
                .flat_map(|range| [&mut range.start, &mut range.end]),
        )
        .chain(
            paragraph
                .links
                .iter_mut()
                .flat_map(|link| [&mut link.range.start, &mut link.range.end]),
        )
        .collect();
    offsets.sort_by_key(|offset| **offset);
    let mut offsets = offsets.into_iter().peekable();
    let mut tabs = expanded_tabs.iter().peekable();
    for span in &mut paragraph.spans {
        let language = language(&span.style.language);
        let start = output.0.len();
        for (relative, ch) in original[span.range.clone()].char_indices() {
            let offset = span.range.start + relative;
            while let Some(mapped) = offsets.next_if(|mapped| **mapped <= offset) {
                *mapped = output.0.len();
            }
            if capitalize && words.peek().is_some_and(|&start| start == offset) {
                words.next();
                let count = mapper.titlecase_prefix_len(&original[offset..], &language);
                let minimum_end = original[offset..]
                    .char_indices()
                    .nth(count)
                    .map_or(original.len(), |(relative, _)| offset + relative);
                let end = original[offset..]
                    .grapheme_indices(true)
                    .map(|(relative, cluster)| offset + relative + cluster.len())
                    .find(|&end| end >= minimum_end)
                    .unwrap_or(original.len());
                prefix = offset..end;
            }
            let mapping = match span.style.text_transform.case {
                TextCase::None | TextCase::MathAuto => None,
                TextCase::Uppercase => Some(ContextCase::Upper),
                TextCase::Lowercase => Some(ContextCase::Lower),
                TextCase::Capitalize => {
                    (prefix.contains(&offset) && !ch.is_uppercase()).then_some(ContextCase::Title)
                }
            };
            while tabs.peek().is_some_and(|tab| tab.end <= offset) {
                tabs.next();
            }
            let expanded_tab = tabs.peek().is_some_and(|tab| tab.contains(&offset));
            let mut transformed = Transformed {
                output: &mut output,
                transform: span.style.text_transform,
                preserve_spaces: !span.style.white_space.collapses_spaces() && !expanded_tab,
            };
            let result = if let Some(mapping) = mapping {
                context.map_char(offset, &language, mapping, &mut transformed)
            } else {
                transformed.write_char(
                    if span.style.text_transform.case == TextCase::MathAuto
                        && span.single_character_source
                    {
                        super::transform_math::italic(ch)
                    } else {
                        ch
                    },
                )
            };
            result.map_err(|_| DocumentError::TextExpansionLimit {
                limit: MAX_INLINE_TEXT_BYTES,
            })?;
        }
        span.range = start..output.0.len();
    }
    for mapped in offsets {
        *mapped = output.0.len();
    }
    paragraph.spans.retain(|span| !span.range.is_empty());
    paragraph.text = output.0;
    Ok(())
}

fn language(value: &str) -> LanguageIdentifier {
    value
        .parse::<icu_locale_core::Locale>()
        .map_or(LanguageIdentifier::UNKNOWN, |locale| locale.id)
}
