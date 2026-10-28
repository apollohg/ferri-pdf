//! CSS syntax, cascade, computed values, and page/font rules.
mod angles;
mod background_values;
mod bookmark_values;
mod border_image_values;
mod box_values;
mod calc;
mod cascade;
mod color_scheme;
mod colors;
mod component;
mod compositing_values;
mod computed;
mod conditional;
mod conditions;
#[cfg(test)]
mod conformance;
pub(crate) mod counter_style;
mod counter_values;
mod declaration;
mod decoration_values;
mod diagnostics;
mod easing;
mod feature_values;
mod flex_values;
mod float_values;
mod font_values;
mod generated;
mod grammar;
mod grid_values;
mod inert;
mod layers;
mod list_values;
mod loader;
mod logical;
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
pub(crate) use object::ObjectFit;
pub(crate) use overflow::{BlockEllipsis, Continue, LineClamp, Overflow, TextOverflow};
pub(crate) use transform::{Parent3d, Rotation, TransformBox, TransformFunction, TransformStyle};
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
#[path = "../../build/syntax.rs"]
mod value_definition_syntax;
mod values;
mod variables;
mod wide_keywords;
pub(crate) mod will_change;

use crate::resources::ResourceError;
pub(crate) use bookmark_values::BookmarkStyle;
pub(crate) use box_values::{
    BoxSizing, BreakBetween, BreakInside, Display, LengthPercentage, Size, Visibility,
};
pub(crate) use computed::Style;
pub(crate) use counter_values::{
    CounterProperties, CounterScopes, DEFAULT_COUNTER_INCREMENT, INITIAL_COUNTER_VALUE,
    LIST_ITEM_COUNTER,
};
pub(crate) use decoration_values::{
    DecorationLines, TextDecoration, TextDecorations, UnderlinePosition,
};
pub(crate) use diagnostics::{SourceLocation, StyleDiagnostic, StyleDiagnosticKind};
pub(crate) use flex_values::{Alignment, FlexBasis, FlexStyle, FlexWrap, Gap};
pub(crate) use float_values::{Clear, Float, FloatSide};
pub(crate) use font_values::{
    FontFeatures, FontKerning, FontVariantAlternates, FontVariantCaps, FontVariantEmoji,
    FontVariantPosition, language_system_tag,
};
pub(crate) use generated::{
    CounterRequest, ElementContent, GeneratedContent, GeneratedOutput, NamedStringMode,
    NamedStringRequest, TargetReferenceError, TargetRequest, TargetValue,
};
pub(crate) use grid_values::{
    AutoFlow, COLUMNS, GridAreas, GridLine, GridStyle, MAX_GRID_LINE, NamedArea, ROWS, Subgrid,
    TrackBreadth, TrackList, TrackSize,
};
pub(crate) use list_values::{ListStylePosition, ListStyleType};
pub(crate) use loader::{
    CssDecoder, DecodedStylesheet, SourceDiagnostic, StylesheetInput, StylesheetSources,
};
pub(crate) use multicol_values::{ColumnFill, MulticolStyle};
pub(crate) use page_margins::{MarginLocation, PageMargin};
pub(crate) use page_rules::ResolvedPage;
pub(crate) use page_rules::{PageRequest, PageSide};
pub(crate) use page_selection::{PageSelection, PageStyleTarget};
pub(crate) use page_values::{Page, PageMarks, PageProduction};
pub(crate) use position::Position;
pub use print::{
    ColorScheme, ContrastPreference, MotionPreference, PrintPreferences, PrintSettings,
};
pub(crate) use quotes::{QuoteDepth, Quotes};
pub(crate) use resolution::PageGeometry;
pub(crate) use selector::PseudoElement;
pub(crate) use sizing::{
    IntrinsicSizing, IntrinsicWidths, Overconstraint, SizeConstraints, UsedAxis,
};
pub(crate) use string_values::StringAssignment;
pub(crate) use stylesheet::Stylesheet;
#[cfg(test)]
pub(crate) use stylesheet::StylesheetSource;
pub(crate) use table_values::{
    BorderCollapse, CaptionSide, TableLayout, TableStyle, VerticalAlign,
};
pub(crate) use text_values::UnicodeBidi;
pub(crate) use text_values::{
    DEFAULT_TAB_SIZE, HangingEnd, HangingPunctuation, HyphenateLimitChars, Hyphens, LineBreak,
    LineBreaking, LineHeight, OverflowWrap, TextAlign, TextCase, TextIndent, TextJustify,
    TextTransform, TextWrapStyle, WhiteSpace, WhiteSpaceCollapse, WordBreak,
};
#[cfg(test)]
pub(crate) use {
    counter_values::CounterReset,
    grid_values::{AutoRepeat, NameRepeat},
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
    Font(#[from] crate::fonts::FontError),
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

pub(crate) use crate::paint::POINTS_PER_PIXEL;
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

#[cfg(test)]
mod tests;
