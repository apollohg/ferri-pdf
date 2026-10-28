use crate::document::{MAX_INLINE_TEXT_BYTES, inline::is_css_whitespace};
use crate::dom::{Element, TreeRelation};
use crate::style::{Display, ElementContent, Style, StyleError};
use std::borrow::Cow;

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
                let first = crate::document::first_letter::find(
                    &text,
                    crate::style::WhiteSpaceCollapse::Collapse,
                    self.element.language(),
                );
                text[..first.end().unwrap_or(0)].to_owned()
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

#[cfg(test)]
mod tests {
    use super::*;

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
