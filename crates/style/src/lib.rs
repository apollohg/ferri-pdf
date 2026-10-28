//! CSS syntax, cascade, computed values, and page/font rules.
mod angles;
mod background_values;
mod bookmark_values;
mod border_image_values;
mod box_values;
mod calc;
mod cascade;
mod colors;
mod component;
mod computed;
mod conditional;
mod conditions;
#[cfg(test)]
mod conformance;
mod counter_style;
mod counter_values;
mod declaration;
mod decoration_values;
mod diagnostics;
mod easing;
mod flex_values;
mod float_values;
mod font_values;
mod generated;
mod grammar;
mod grid_values;
mod layers;
mod list_values;
mod loader;
mod matcher;
mod media;
mod multicol_values;
mod nesting;
mod object;
mod overflow;
mod page_margins;
mod page_parser;
mod page_rules;
mod page_selection;
mod page_values;
mod parser;
mod position;
mod print;
mod quotes;
pub use object::ObjectFit;
pub use overflow::{Overflow, TextOverflow};
pub use transform::{Matrix, TransformFunction};
mod attribute;
mod properties;
mod resolution;
mod scope;
mod selector;
mod shadow_values;
mod shorthands;
mod sizing;
mod string_values;
mod stylesheet;
mod supports;
mod syntax;
mod table_values;
mod text_values;
mod transform;
mod units;
mod user_agent;
#[cfg(test)]
#[path = "../build/syntax.rs"]
mod value_definition_syntax;
mod values;
mod variables;
mod wide_keywords;

pub use bookmark_values::BookmarkStyle;
pub use box_values::{
    BoxSizing, BreakBetween, BreakInside, Display, LengthPercentage, Size, Visibility,
};
pub use computed::Style;
pub use counter_values::{
    CounterChange, CounterProperties, CounterReset, CounterScopes, DEFAULT_COUNTER_INCREMENT,
    INITIAL_COUNTER_VALUE, LIST_ITEM_COUNTER,
};
pub use decoration_values::{DecorationLines, TextDecoration, TextDecorations, UnderlinePosition};
pub use diagnostics::{SourceLocation, StyleDiagnostic, StyleDiagnosticKind};
use ferri_pdf_resources::ResourceError;
pub use flex_values::{Alignment, FlexBasis, FlexDirection, FlexStyle, FlexWrap, Gap};
pub use float_values::{Clear, Float, FloatSide};
pub use font_values::FontVariantCaps;
pub use generated::{
    CounterRequest, CounterText, ElementContent, ElementContentText, GeneratedContent,
    GeneratedOutput, NamedStringMode, NamedStringRequest, NamedStringText, TargetReferenceError,
    TargetRequest, TargetText, TargetValue,
};
pub use grid_values::{
    AutoFlow, AutoRepeat, COLUMNS, GridAreas, GridLine, GridStyle, LineNames, MAX_GRID_LINE,
    NameRepeat, NamedArea, ROWS, Subgrid, TrackBreadth, TrackList, TrackSize,
};
pub use list_values::{ListStylePosition, ListStyleType};
pub use loader::{
    CssDecoder, DecodedStylesheet, SourceDiagnostic, StylesheetInput, StylesheetSources,
};
pub use multicol_values::{ColumnFill, MulticolStyle};
pub use page_margins::{MarginLocation, PageMargin};
pub use page_rules::ResolvedPage;
pub use page_rules::{PageRequest, PageSide};
pub use page_selection::{PageSelection, PageStyleTarget};
pub use page_values::{Page, PageMarks, PageProduction};
pub use position::Position;
pub use print::{
    ColorScheme, ContrastPreference, MotionPreference, PrintPreferences, PrintSettings,
};
pub use quotes::{QuoteDepth, Quotes};
pub use resolution::PageGeometry;
pub use selector::PseudoElement;
pub use sizing::{IntrinsicSizing, IntrinsicWidths, Overconstraint, SizeConstraints, UsedAxis};
pub use string_values::StringAssignment;
pub use stylesheet::{Stylesheet, StylesheetSource};
pub use table_values::{BorderCollapse, CaptionSide, TableLayout, TableStyle, VerticalAlign};
pub use text_values::UnicodeBidi;
pub use text_values::{
    DEFAULT_TAB_SIZE, Hyphens, LineHeight, OverflowWrap, TextAlign, TextCase, TextIndent,
    TextTransform, WhiteSpace,
};

#[derive(Debug, thiserror::Error)]
pub enum StyleError {
    #[error("invalid target reference {reference:?}: {reason}")]
    TargetReference {
        reference: String,
        reason: TargetReferenceError,
    },
    #[error("CSS {resource} exceeds {limit} at {location:?}")]
    StylesheetLimit {
        resource: &'static str,
        limit: usize,
        location: Box<SourceLocation>,
    },
    #[error(
        "CSS computed-value dependency cycle {dependencies:?} at {sources:?}; normative cycle resolution is unverified"
    )]
    DependencyCycle {
        sources: Vec<SourceLocation>,
        dependencies: Vec<String>,
    },
    #[error("invalid print paper dimensions in points: {paper:?}")]
    InvalidPrintSettings { paper: [f32; 2] },
    #[error("CSS variable {resource} exceeds {limit}")]
    VariableLimit {
        resource: &'static str,
        limit: usize,
    },
    #[error("generated content exceeds {limit} bytes")]
    GeneratedContentLimit { limit: usize },
    #[error("CSS value exceeds {limit} {resource}")]
    GrammarLimit {
        resource: &'static str,
        limit: usize,
    },
    /// Valid CSS the engine does not implement.
    #[error("unsupported CSS: {0}")]
    Unsupported(String),
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    Resource(#[from] ResourceError),
    #[error("{0}")]
    Font(#[from] ferri_pdf_fonts::FontError),
}
impl From<String> for StyleError {
    fn from(message: String) -> Self {
        Self::Invalid(message)
    }
}
impl From<&str> for StyleError {
    fn from(message: &str) -> Self {
        Self::Invalid(message.into())
    }
}

pub use ferri_pdf_paint::POINTS_PER_PIXEL;
const POINTS_PER_INCH: f32 = 72.0;
const MILLIMETRES_PER_INCH: f32 = 25.4;
const MILLIMETRES_PER_CENTIMETRE: f32 = 10.0;
const DEFAULT_FONT_SIZE: f32 = 12.0;
/// The `widows` and `orphans` initial value (CSS Fragmentation 3 §4.4).
const DEFAULT_MIN_FRAGMENT_LINES: usize = 2;
const DEFAULT_PAGE: [f32; 2] = [595.2756, 841.8898];
/// Page margins before `@page` sets any.
const DEFAULT_PAGE_MARGIN: f32 = POINTS_PER_INCH;
const DEFAULT_PAGE_AREA: [f32; 2] = [
    DEFAULT_PAGE[0] - 2.0 * DEFAULT_PAGE_MARGIN,
    DEFAULT_PAGE[1] - 2.0 * DEFAULT_PAGE_MARGIN,
];
