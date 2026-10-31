use crate::resources::ResourceBase;
use std::{cell::RefCell, collections::HashSet, fmt};

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct SourceLocation {
    pub base: ResourceBase,
    pub line: u32,
    pub column: u32,
}
impl SourceLocation {
    pub(crate) fn new(base: &ResourceBase, location: cssparser::SourceLocation) -> Self {
        Self {
            base: base.clone(),
            line: location.line.saturating_add(1),
            column: location.column,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StyleDiagnosticKind {
    InvalidRule,
    InvalidDeclaration,
    UnavailableSource,
    ImportCycle,
    /// A valid rule or descriptor the engine does not implement, which it ignores.
    Unsupported,
    /// Input past a resource limit, which the engine ignores.
    LimitExceeded,
}
impl StyleDiagnosticKind {
    /// What ignoring a construct for `error` reports: `invalid` unless the construct is
    /// unsupported or past a limit.
    pub(crate) fn of(error: &crate::style::StyleError, invalid: Self) -> Self {
        use crate::style::StyleError;
        match error {
            StyleError::Unsupported(_) => Self::Unsupported,
            StyleError::StylesheetLimit { .. }
            | StyleError::GrammarLimit { .. }
            | StyleError::VariableLimit { .. }
            | StyleError::GeneratedContentLimit { .. } => Self::LimitExceeded,
            _ => invalid,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StyleDiagnostic {
    pub location: SourceLocation,
    pub kind: StyleDiagnosticKind,
    pub context: String,
}
impl fmt::Display for StyleDiagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "CSS {:?} at {:?}:{}:{}: {}",
            self.kind, self.location.base, self.location.line, self.location.column, self.context
        )
    }
}

/// Input the engine renders without, instead of failing.
#[derive(Clone, Debug, Eq, Hash, PartialEq, thiserror::Error)]
pub enum Degradation {
    /// CSS Values 5 §2.4.1: the declaration's property computes as if `unset`.
    #[error("{declaration} is invalid at computed-value time and computes as unset: {reason}")]
    InvalidAtComputedValueTime { declaration: String, reason: String },
    /// A computed-value dependency cycle makes the declaration invalid at computed-value time.
    #[error(
        "{declaration} is invalid at computed-value time and computes as unset: dependency cycle {dependencies:?} at {sources:?}"
    )]
    DependencyCycle {
        declaration: String,
        sources: Vec<SourceLocation>,
        dependencies: Vec<String>,
    },
    /// A `target-counter()`, `target-counters()` or `target-text()` reference that identifies
    /// no element of the document: the target's value is empty.
    #[error("target reference {reference:?} is unresolved ({reason}) and renders as empty text")]
    UnresolvedTarget { reference: String, reason: String },
    /// Page box lengths past what layout represents leave the page no finite geometry, so the
    /// page box takes the geometry no `@page` rule sizes.
    #[error("the page box geometry is not finite; the page takes the initial page geometry")]
    NonFinitePageGeometry,
    /// An inline `svg` element the renderer cannot process renders nothing (SVG 2 §1.9).
    #[error("inline SVG content is in error and renders nothing: {reason}")]
    SvgInError { reason: String },
}

/// Degradations, each kept once however many elements and layout passes repeat it.
#[derive(Default)]
pub(crate) struct Degradations {
    recorded: RefCell<Vec<Degradation>>,
    seen: RefCell<HashSet<Degradation>>,
}
impl Degradations {
    pub(crate) fn record(&self, degradation: Degradation) {
        if self.seen.borrow_mut().insert(degradation.clone()) {
            self.recorded.borrow_mut().push(degradation);
        }
    }
    /// The degradations recorded since the last call.
    pub(crate) fn take(&self) -> Vec<Degradation> {
        std::mem::take(&mut self.recorded.borrow_mut())
    }
}
