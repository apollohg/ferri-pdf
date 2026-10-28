use ferri_pdf_resources::ResourceBase;
use std::fmt;

#[derive(Clone, Debug, Eq, PartialEq)]
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
