//! CSS syntax, cascade, computed values, and page/font rules.
pub(crate) mod anchor_values;
mod angles;
mod background_values;
mod baseline_values;
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
mod container_queries;
mod containment;
pub(crate) mod counter_style;
mod counter_values;
mod declaration;
mod decoration_values;
mod diagnostics;
mod easing;
mod feature_values;
mod filter_values;
mod flex_values;
mod float_values;
mod font_values;
mod generated;
mod grammar;
mod grid_values;
mod inert;
mod initial_letter_values;
mod layers;
mod list_values;
mod loader;
pub(crate) mod logical;
mod mask_values;
mod matcher;
mod math_values;
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
mod registration;
mod zoom;
pub(crate) use colors::{parse_color_value, widget_palette};
pub(crate) use object::{ImageOrientation, ImageResolution, ObjectFit};
pub(crate) use overflow::{
    BlockEllipsis, ClipEdge, Continue, LineClamp, Overflow, OverflowClipMargin, TextOverflow,
};
pub(crate) use transform::{Parent3d, Rotation, TransformBox, TransformFunction, TransformStyle};
pub(crate) use user_agent::legacy_color;
pub(crate) use widget_values::{Appearance, FieldSizing, WidgetStyle};
pub(crate) use zoom::{INITIAL_ZOOM, accumulate_zoom};
mod attribute;
pub(crate) mod motion;
mod properties;
mod resolution;
mod ruby_values;
mod scope;
mod selector;
mod shadow_values;
pub(crate) mod shape_values;
mod shorthands;
mod sizing;
mod source_set;
mod string_values;
mod stylesheet;
mod supports;
pub(crate) mod svg_cascade;
pub(crate) mod svg_values;
mod syntax;
mod table_values;
mod text_paint_values;
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
mod widget_values;
pub(crate) mod will_change;

use crate::resources::ResourceError;
pub(crate) use baseline_values::{
    Baseline, BaselineAlignment, BaselineShift, BaselineSource, TextBox, TextEdge,
};
pub(crate) use bookmark_values::BookmarkStyle;
pub(crate) use box_values::{
    BoxSizing, BreakBetween, BreakInside, CalculatedSize, Display, LengthPercentage, MarginTrim,
    Size, Visibility,
};
pub(crate) use computed::{OriginNode, Style, StyleOrigin};
pub(crate) use container_queries::{QueryContainers, QuerySizes};
pub(crate) use containment::{
    Contain, ContainIntrinsicSize, ContainerType, ContainmentStyle, ContentVisibility,
};
pub(crate) use counter_style::Representation;
pub(crate) use counter_values::{
    CounterProperties, CounterScopes, DEFAULT_COUNTER_INCREMENT, FOOTNOTE_COUNTER,
    INITIAL_COUNTER_VALUE, LIST_ITEM_COUNTER,
};
pub(crate) use decoration_values::{
    DecorationInset, DecorationLines, DecorationSkip, TextDecoration, TextDecorations,
    UnderlinePosition,
};
pub(crate) use diagnostics::{SourceLocation, StyleDiagnostic, StyleDiagnosticKind};
pub(crate) use flex_values::{Alignment, FlexBasis, FlexStyle, FlexWrap, Gap, OverflowPosition};
pub(crate) use float_values::{Clear, Float, FloatSide, FootnoteDisplay, FootnotePolicy};
pub(crate) use font_values::{
    FontFeatures, FontKerning, FontVariantAlternates, FontVariantCaps, FontVariantEmoji,
    FontVariantPosition, RUBY_FEATURE, language_system_tag,
};
pub(crate) use generated::{
    CounterRequest, ElementContent, GeneratedContent, GeneratedOutput, NamedStringMode,
    NamedStringRequest, TargetReferenceError, TargetRequest, TargetValue,
};
pub(crate) use grid_values::{
    AutoFlow, COLUMNS, GridAreas, GridLine, GridStyle, MAX_GRID_LINE, NamedArea, ROWS, Subgrid,
    TrackBreadth, TrackList, TrackSize,
};
pub(crate) use initial_letter_values::{InitialLetter, InitialLetterAlign, LetterAlignment};
pub(crate) use list_values::{ListStylePosition, ListStyleType};
pub(crate) use loader::{
    CssDecoder, DecodedStylesheet, SourceDiagnostic, StylesheetInput, StylesheetSources,
};
pub(crate) use multicol_values::{ColumnFill, MulticolStyle};
pub(crate) use page_margins::{MarginLocation, PageMargin};
pub(crate) use page_rules::ResolvedPage;
pub(crate) use page_rules::{PageGroupPosition, PageRequest, PageSequence, PageSide};
pub(crate) use page_selection::{FirstPageStart, PageSelection, PageStyleTarget};
pub(crate) use page_values::{Page, PageMarks, PageProduction};
pub(crate) use position::Position;
pub(crate) use print::Presentation;
pub use print::{
    ColorScheme, ContrastPreference, MotionPreference, PrintPreferences, PrintSettings,
};
pub(crate) use quotes::{QuoteDepth, Quotes};
pub(crate) use resolution::PageGeometry;
pub(crate) use ruby_values::{RubyAlign, RubyMerge, RubyPosition, RubyStyle};
pub(crate) use selector::{GeneratedPseudoElement, PseudoElement};
pub(crate) use sizing::{
    AspectRatio, ConstraintsKey, IntrinsicSizing, IntrinsicWidths, Overconstraint, SizeConstraints,
    UsedAxis,
};
pub(crate) use source_set::ImageSource;
pub(crate) use string_values::StringAssignment;
pub(crate) use stylesheet::Stylesheet;
#[cfg(test)]
pub(crate) use stylesheet::StylesheetSource;
pub(crate) use table_values::{
    BorderCollapse, CaptionSide, TableLayout, TableStyle, VerticalAlign,
};
pub(crate) use text_paint_values::{
    EmphasisPosition, EmphasisShape, EmphasisSkip, EmphasisStyle, InheritedColor, TextPaint,
    TextShadow,
};
pub(crate) use text_values::{
    DEFAULT_TAB_SIZE, HangingEnd, HangingPunctuation, HyphenateLimitChars, Hyphens, LineBreak,
    LineBreaking, LineHeight, OverflowWrap, TabSize, TextAlign, TextAutospace, TextCase,
    TextIndent, TextJustify, TextSpacing, TextSpacingTrim, TextTransform, TextWrapStyle,
    WhiteSpace, WhiteSpaceCollapse, WordBreak,
};
pub(crate) use text_values::{UnicodeBidi, has_primary_language};
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
