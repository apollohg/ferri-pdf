use crate::{MAX_INLINE_TEXT_BYTES, inline::is_css_whitespace};
use ferri_pdf_dom::{Element, TreeRelation};
use ferri_pdf_style::{Display, ElementContent, Style, StyleError};
use icu_properties::{
    CodePointMapData,
    props::{GeneralCategory, GeneralCategoryGroup},
};
use std::borrow::Cow;
use unicode_segmentation::UnicodeSegmentation;

#[derive(Default)]
pub(crate) struct GeneratedText {
    text: String,
    in_flow: bool,
}
impl GeneratedText {
    pub(crate) fn capture(text: &str, style: &Style) -> Self {
        Self {
            text: text.to_owned(),
            in_flow: style.display == Display::Contents
                || (!style.position.is_out_of_flow() && style.floating().is_none()),
        }
    }
    fn first_letter_text(&self) -> &str {
        if self.in_flow { &self.text } else { "" }
    }
}

pub(crate) struct ElementText<'a, 'd> {
    element: Element<'d>,
    text: Option<Cow<'a, str>>,
    before: &'a GeneratedText,
    after: &'a GeneratedText,
    marker: Option<&'a str>,
}
impl<'a, 'd> ElementText<'a, 'd> {
    pub(crate) fn new(
        element: Element<'d>,
        text: Option<&'a str>,
        before: &'a GeneratedText,
        after: &'a GeneratedText,
        marker: Option<&'a str>,
    ) -> Self {
        Self {
            element,
            text: text.map(Cow::Borrowed),
            before,
            after,
            marker,
        }
    }
    pub(crate) fn resolve(&mut self, source: ElementContent) -> Result<String, StyleError> {
        Ok(match source {
            ElementContent::Text => {
                if self.text.is_none() {
                    self.text = Some(Cow::Owned(element_text(self.element)?));
                }
                self.text
                    .as_deref()
                    .expect("element text was initialized")
                    .to_owned()
            }
            ElementContent::FirstLetter => {
                let text = text_with_generated(
                    self.element,
                    self.before.first_letter_text(),
                    self.after.first_letter_text(),
                )?;
                first_letter(&text).to_owned()
            }
            ElementContent::Before => collapse(&self.before.text),
            ElementContent::After => collapse(&self.after.text),
            ElementContent::Marker => self.marker.map(collapse).unwrap_or_default(),
        })
    }
}

/// `bookmark-label` text: literal strings verbatim, `content()` values as if `white-space:
/// normal` (CSS GCPM 3 §1.1.1).
pub(crate) fn element_text(element: Element<'_>) -> Result<String, StyleError> {
    text_with_generated(element, "", "")
}

fn text_with_generated(
    element: Element<'_>,
    before: &str,
    after: &str,
) -> Result<String, StyleError> {
    let mut text = String::new();
    for part in std::iter::once(before)
        .chain(element.text_in(TreeRelation::Formatting))
        .chain(std::iter::once(after))
    {
        if text
            .len()
            .checked_add(part.len())
            .is_none_or(|bytes| bytes > MAX_INLINE_TEXT_BYTES)
        {
            return Err(StyleError::GeneratedContentLimit {
                limit: MAX_INLINE_TEXT_BYTES,
            });
        }
        text.push_str(part);
    }
    Ok(collapse(&text))
}

/// Collapses runs of white space and trims it from both ends, as `white-space: normal` lays
/// out a lone line.
pub(crate) fn collapse(text: &str) -> String {
    let mut collapsed = String::new();
    for word in text
        .split(is_css_whitespace)
        .filter(|word| !word.is_empty())
    {
        if !collapsed.is_empty() {
            collapsed.push(' ');
        }
        collapsed.push_str(word);
    }
    collapsed
}

/// The first typographic letter unit of `text` (CSS Pseudo-Elements 4 §2.4): a letter, number
/// or symbol grapheme with the punctuation and spaces before it and the punctuation after it.
fn first_letter(text: &str) -> &str {
    let categories = CodePointMapData::<GeneralCategory>::new();
    let is = |grapheme: &str, group: GeneralCategoryGroup| {
        grapheme
            .chars()
            .next()
            .is_some_and(|first| group.contains(categories.get(first)))
    };
    let separator = |grapheme: &str| is(grapheme, GeneralCategoryGroup::SpaceSeparator);
    let punctuation = |grapheme: &str| is(grapheme, GeneralCategoryGroup::Punctuation);
    let mut graphemes = text.grapheme_indices(true).peekable();
    while graphemes
        .next_if(|(_, grapheme)| punctuation(grapheme) || separator(grapheme))
        .is_some()
    {}
    let Some((start, letter)) = graphemes.next() else {
        return "";
    };
    if ![
        GeneralCategoryGroup::Letter,
        GeneralCategoryGroup::Number,
        GeneralCategoryGroup::Symbol,
    ]
    .into_iter()
    .any(|group| is(letter, group))
    {
        return "";
    }
    let mut end = start + letter.len();
    while let Some((index, grapheme)) = graphemes.next_if(|(_, grapheme)| punctuation(grapheme)) {
        end = index + grapheme.len();
    }
    &text[..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_letter_keeps_surrounding_punctuation_and_whole_graphemes() {
        for (text, expected) in [
            ("\"Quoted\" text", "\"Q"),
            ("« Bonjour", "« B"),
            ("(1) item", "(1)"),
            ("e\u{301}lan", "e\u{301}"),
            ("Ab", "A"),
            ("...", ""),
            ("", ""),
            ("€5", "€"),
        ] {
            assert_eq!(
                first_letter(text),
                expected,
                "first typographic letter unit of {text:?}"
            );
        }
    }

    #[test]
    fn collapse_follows_white_space_normal() {
        assert_eq!(
            collapse("  First \n\t heading\r\ntext  "),
            "First heading text"
        );
        assert_eq!(
            collapse("\u{a0}kept\u{a0}"),
            "\u{a0}kept\u{a0}",
            "NBSP is not white space"
        );
    }
}
