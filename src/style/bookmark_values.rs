use crate::style::GeneratedContent;

/// An element's bookmark (CSS GCPM 3 §4).
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub(crate) struct BookmarkStyle {
    /// `bookmark-level`; `None` makes no bookmark.
    pub level: Option<usize>,
    pub label: GeneratedContent,
    /// `bookmark-state: open` shows the bookmark's descendants.
    pub open: bool,
}
impl Default for BookmarkStyle {
    fn default() -> Self {
        Self {
            level: None,
            label: GeneratedContent::element_text_content(),
            open: true,
        }
    }
}
