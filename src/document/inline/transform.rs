use super::{MAX_INLINE_TEXT_BYTES, Paragraph};
use crate::document::DocumentError;
use crate::style::{TextCase, TextTransform};
use icu_casemap::{CaseMapper, ContextCase};
use icu_locale_core::LanguageIdentifier;
use std::fmt::{self, Write};
use std::ops::Range;
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

/// Where two transformed copies of one text, a paragraph's `::first-line` copy and the
/// paragraph, place the same offsets: the `[copy, paragraph]` ranges of each source character
/// whose encodings differ in length or character boundaries, in order. Offsets inside those
/// have no counterpart.
#[derive(Clone, Debug, Default)]
pub(crate) struct OffsetMap(Box<[[Range<usize>; 2]]>);

const COPY: usize = 0;
const PARAGRAPH: usize = 1;

impl OffsetMap {
    /// From each source character's output `[copy, paragraph]` ranges in texts `texts`.
    pub(super) fn between(units: [&[Range<usize>]; 2], texts: [&str; 2]) -> Self {
        fn lengths<'t>(text: &'t str, range: &Range<usize>) -> impl Iterator<Item = usize> + 't {
            text[range.clone()].chars().map(char::len_utf8)
        }
        Self(
            units[COPY]
                .iter()
                .zip(units[PARAGRAPH])
                .filter(|(copy, paragraph)| {
                    !lengths(texts[COPY], copy).eq(lengths(texts[PARAGRAPH], paragraph))
                })
                .map(|(copy, paragraph)| [copy.clone(), paragraph.clone()])
                .collect(),
        )
    }
    /// Offset `offset` of side `from` on the other side.
    fn map(&self, offset: usize, from: usize) -> Option<usize> {
        let to = 1 - from;
        let next = self.0.partition_point(|entry| entry[from].end <= offset);
        if self
            .0
            .get(next)
            .is_some_and(|entry| entry[from].start < offset)
        {
            return None;
        }
        Some(next.checked_sub(1).map_or(offset, |previous| {
            let entry = &self.0[previous];
            offset - entry[from].end + entry[to].end
        }))
    }
    /// The paragraph offset of copy offset `offset`.
    pub(crate) fn to_paragraph(&self, offset: usize) -> Option<usize> {
        self.map(offset, COPY)
    }
    /// The copy offset of paragraph offset `offset`.
    pub(crate) fn to_copy(&self, offset: usize) -> Option<usize> {
        self.map(offset, PARAGRAPH)
    }
    /// The copy offset of the first paragraph offset from `offset` on that has one: the end
    /// of the character whose encoding holds `offset`.
    pub(crate) fn to_copy_after(&self, offset: usize) -> usize {
        self.to_copy(offset).unwrap_or_else(|| {
            let entry = self
                .0
                .partition_point(|entry| entry[PARAGRAPH].end <= offset);
            self.0[entry][COPY].end
        })
    }
}

/// Applies the spans' `text-transform`, returning the output range of each source character
/// where `record` asks for it.
pub(super) fn apply(
    paragraph: &mut Paragraph,
    record: bool,
) -> Result<Vec<Range<usize>>, DocumentError> {
    let mut units = Vec::new();
    if paragraph
        .spans
        .iter()
        .all(|span| span.style.text_transform == TextTransform::default())
    {
        if record {
            for span in &paragraph.spans {
                units.extend(paragraph.text[span.range.clone()].char_indices().map(
                    |(relative, ch)| {
                        let start = span.range.start + relative;
                        start..start + ch.len_utf8()
                    },
                ));
            }
        }
        return Ok(units);
    }
    let mapper = CaseMapper::new();
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
                .links
                .iter_mut()
                .flat_map(|link| [&mut link.range.start, &mut link.range.end]),
        )
        .collect();
    offsets.sort_by_key(|offset| **offset);
    let mut offsets = offsets.into_iter().peekable();
    for span in &mut paragraph.spans {
        let language = language(&span.style.language);
        let start = output.0.len();
        for (relative, ch) in original[span.range.clone()].char_indices() {
            let offset = span.range.start + relative;
            let unit = output.0.len();
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
            let mut transformed = Transformed {
                output: &mut output,
                transform: span.style.text_transform,
                preserve_spaces: !span.style.white_space.collapses_spaces(),
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
            if record {
                units.push(unit..output.0.len());
            }
        }
        span.range = start..output.0.len();
    }
    for mapped in offsets {
        *mapped = output.0.len();
    }
    paragraph.spans.retain(|span| !span.range.is_empty());
    paragraph.text = output.0;
    Ok(units)
}

fn language(value: &str) -> LanguageIdentifier {
    value
        .parse::<icu_locale_core::Locale>()
        .map_or(LanguageIdentifier::UNKNOWN, |locale| locale.id)
}
