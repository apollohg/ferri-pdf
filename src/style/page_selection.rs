use crate::dom::ElementId;
use crate::style::{
    CaptionSide, Display, GeneratedContent, Page, Position, PseudoElement, Quotes, StyleError,
    WhiteSpaceCollapse,
};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct PageStyleTarget {
    pub element: ElementId,
    pub pseudo: Option<PseudoElement>,
}

/// On-demand computed values used by generated-box first-page selection.
pub(crate) trait PageSelection {
    fn display(&self, target: PageStyleTarget) -> Result<Display, StyleError>;
    fn position(&self, target: PageStyleTarget) -> Result<Position, StyleError>;
    fn page(&self, target: PageStyleTarget) -> Result<Page, StyleError>;
    fn break_before(
        &self,
        target: PageStyleTarget,
    ) -> Result<crate::style::BreakBetween, StyleError>;
    fn progression_rtl(&self) -> Result<bool, StyleError>;
    fn order(&self, target: PageStyleTarget) -> Result<i32, StyleError>;
    fn caption_side(&self, target: PageStyleTarget) -> Result<CaptionSide, StyleError>;
    fn white_space_collapse(
        &self,
        target: PageStyleTarget,
    ) -> Result<WhiteSpaceCollapse, StyleError>;
    fn content(&self, target: PageStyleTarget) -> Result<Option<GeneratedContent>, StyleError>;
    fn bookmark_label(
        &self,
        target: PageStyleTarget,
    ) -> Result<Option<GeneratedContent>, StyleError>;
    fn string_set(
        &self,
        target: PageStyleTarget,
    ) -> Result<std::sync::Arc<[crate::style::StringAssignment]>, StyleError>;
    fn counter_scopes(
        &self,
        target: PageStyleTarget,
    ) -> Result<crate::style::CounterScopes, StyleError>;
    fn counters(
        &self,
        target: PageStyleTarget,
    ) -> Result<crate::style::CounterProperties, StyleError>;
    fn quotes(&self, target: PageStyleTarget) -> Result<Quotes, StyleError>;
    fn inline_edges(
        &self,
        target: PageStyleTarget,
        continued: [bool; 2],
    ) -> Result<bool, StyleError>;
    fn tab_size(&self, target: PageStyleTarget) -> Result<usize, StyleError>;
    fn inside_marker(&self, target: PageStyleTarget) -> Result<bool, StyleError>;
}

pub(crate) type SelectFirstPageName<'a> =
    dyn Fn(&dyn PageSelection) -> Result<Option<std::sync::Arc<str>>, StyleError> + 'a;
pub(crate) type SelectFirstPageSide<'a> =
    dyn Fn(&dyn PageSelection) -> Result<crate::style::PageSide, StyleError> + 'a;
