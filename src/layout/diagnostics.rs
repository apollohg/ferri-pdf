//! Layout results that depart from the specified rendering because the input exceeded a
//! resource limit, never reached a fixed layout, or had geometry no layout can use.
use std::{cell::RefCell, fmt, rc::Rc};

/// A layout degradation retained with the rendering result.
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum LayoutDiagnostic {
    #[error("{0} did not reach a fixed layout; the last completed layout is kept")]
    Unsettled(Convergence),
    #[error("layout exceeds {limit} pages; content after the last page is omitted")]
    PageLimit { limit: usize },
    #[error("layout exceeds {limit} work units; {kept}")]
    WorkLimit { limit: usize, kept: WorkFallback },
    /// A box whose geometry lies beyond finite coordinates, or whose structure or effects
    /// layout cannot form, is omitted with its content.
    #[error("a box layout cannot form is omitted: {reason}")]
    OmittedBox { reason: String },
    /// Re-laying out the document failed at a box, so the last completed layout is kept.
    #[error("re-laying out failed ({reason}); the last completed layout is kept")]
    FailedRelayout { reason: String },
    #[error(
        "fixed boxes repeated on every page exceed {limit} placements; they repeat on the first \
         {pages} pages"
    )]
    FixedPlacementLimit { limit: usize, pages: usize },
}
impl LayoutDiagnostic {
    /// Whether a limit left author content out of the laid-out pages.
    pub fn loses_content(&self) -> bool {
        match self {
            Self::PageLimit { .. } | Self::OmittedBox { .. } | Self::FixedPlacementLimit { .. } => {
                true
            }
            Self::WorkLimit { kept, .. } => match kept {
                WorkFallback::Truncated => true,
                WorkFallback::LastPass => false,
            },
            Self::Unsettled(_) | Self::FailedRelayout { .. } => false,
        }
    }
}

/// A layout that repeats until what it measured stops changing.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Convergence {
    /// CSS Anchor Positioning 1 §3.
    AnchorPositioning,
    /// CSS Generated Content 3 §1 cross-references.
    GeneratedContent,
    /// CSS Conditional 5 §5 container queries.
    ContainerQueries,
    /// CSS Overflow 5 §3.1.8 active scroll markers.
    ScrollMarkers,
    /// CSS Images 4 §2.7 `element()` images.
    ElementImages,
    /// CSS Page 3 §3 page sizes that depend on their content.
    PageSizes,
    /// CSS GCPM 3 §2.8 footnote calls kept with their footnotes.
    FootnoteDeferrals,
}
impl fmt::Display for Convergence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::AnchorPositioning => "anchor positioning",
            Self::GeneratedContent => "generated content",
            Self::ContainerQueries => "container query sizing",
            Self::ScrollMarkers => "scroll marker activation",
            Self::ElementImages => "element() image rendering",
            Self::PageSizes => "content-sized pages",
            Self::FootnoteDeferrals => "footnote call deferral",
        })
    }
}

/// What a layout whose work budget ran out keeps.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkFallback {
    /// The last completed layout pass, before the re-layout the budget could not finish.
    LastPass,
    /// The content in document order before the formatting box the budget ran out in.
    Truncated,
}
impl fmt::Display for WorkFallback {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::LastPass => "the last completed layout is kept",
            Self::Truncated => "content from where the budget ran out is omitted",
        })
    }
}

/// The diagnostics one layout pass records, shared by its measurers and paginators.
#[derive(Clone, Default)]
pub(crate) struct Diagnostics(Rc<RefCell<Vec<LayoutDiagnostic>>>);
impl Diagnostics {
    pub(crate) fn record(&self, diagnostic: LayoutDiagnostic) {
        let mut recorded = self.0.borrow_mut();
        if !recorded.contains(&diagnostic) {
            recorded.push(diagnostic);
        }
    }
    pub(crate) fn take(&self) -> Vec<LayoutDiagnostic> {
        std::mem::take(&mut self.0.borrow_mut())
    }
}
