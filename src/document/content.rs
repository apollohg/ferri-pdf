use crate::document::{MAX_INLINE_TEXT_BYTES, inline::is_css_whitespace};
use crate::dom::{Element, TreeRelation};
use crate::style::{Display, ElementContent, Style};
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
    /// The text `content()` takes; text from the element's subtree stops once past
    /// `MAX_INLINE_TEXT_BYTES`, which the content taking it cuts at its limit.
    pub(crate) fn resolve(&mut self, source: ElementContent) -> String {
        match source {
            ElementContent::Text => self
                .text
                .get_or_insert_with(|| Cow::Owned(element_text(self.element)))
                .to_string(),
            ElementContent::FirstLetter => {
                let text = text_with_generated(
                    self.element,
                    self.before.first_letter_text(),
                    self.after.first_letter_text(),
                );
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
        }
    }
}

/// `bookmark-label` text: literal strings verbatim, `content()` values as if `white-space:
/// normal` (CSS GCPM 3 §1.1.1). It stops once past `MAX_INLINE_TEXT_BYTES`.
pub(crate) fn element_text(element: Element<'_>) -> String {
    text_with_generated(element, "", "")
}

fn text_with_generated(element: Element<'_>, before: &str, after: &str) -> String {
    let mut text = Collapsed::default();
    for part in std::iter::once(before)
        .chain(element.text_in(TreeRelation::Formatting))
        .chain(std::iter::once(after))
    {
        if !text.push(part, MAX_INLINE_TEXT_BYTES) {
            break;
        }
    }
    text.text
}

/// Text with its runs of white space collapsed and trimmed from both ends, as
/// `white-space: normal` lays out a lone line.
#[derive(Default)]
struct Collapsed {
    text: String,
    space: bool,
}
impl Collapsed {
    /// Appends `part`; false once the text is past `limit` bytes, where it stops.
    fn push(&mut self, part: &str, limit: usize) -> bool {
        for ch in part.chars() {
            if is_css_whitespace(ch) {
                self.space = !self.text.is_empty();
                continue;
            }
            if std::mem::take(&mut self.space) {
                self.text.push(' ');
            }
            self.text.push(ch);
            if self.text.len() > limit {
                return false;
            }
        }
        true
    }
}

/// Reports generated text that `limited` says was cut at `MAX_INLINE_TEXT_BYTES`.
pub(crate) fn record_text_limit(degradations: &crate::style::Degradations, limited: bool) {
    if limited {
        degradations.record(crate::style::Degradation::GeneratedTextLimit {
            limit: MAX_INLINE_TEXT_BYTES,
        });
    }
}

/// Collapses runs of white space and trims it from both ends, as `white-space: normal` lays
/// out a lone line.
pub(crate) fn collapse(text: &str) -> String {
    let mut collapsed = Collapsed::default();
    collapsed.push(text, usize::MAX);
    collapsed.text
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
