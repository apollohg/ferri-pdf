use crate::dom::Element;
use crate::style::{
    LengthPercentage, Style, StyleError,
    box_values::{length_percentage, unsupported},
    component::Component,
    grammar::Keyword,
    units::Relative,
};
use serde::Serialize;

const MAX_HTML_COLUMN_SPAN: usize = 1000;
const MAX_HTML_ROW_SPAN: usize = 65534;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
pub(crate) enum TableLayout {
    #[default]
    Auto,
    Fixed,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
pub(crate) enum BorderCollapse {
    #[default]
    Separate,
    Collapse,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
pub(crate) enum CaptionSide {
    #[default]
    Top,
    Bottom,
}

/// `vertical-align`; lengths raise the box, percentages refer to its own line height.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub(crate) enum VerticalAlign {
    #[default]
    Baseline,
    Top,
    Middle,
    /// HTML `align=center/middle`: the box's centre lies on the parent's baseline.
    BaselineMiddle,
    Bottom,
    Sub,
    Super,
    TextTop,
    TextBottom,
    Length(LengthPercentage),
}
impl VerticalAlign {
    /// Table cells honour only `top`, `middle` and `bottom`; other values align at the baseline.
    #[must_use]
    pub(crate) fn cell(&self) -> Self {
        match self {
            Self::Top | Self::Middle | Self::Bottom => self.clone(),
            _ => Self::Baseline,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct TableStyle {
    pub layout: TableLayout,
    pub collapse: BorderCollapse,
    pub spacing: [f32; 2],
    pub empty_cells: bool,
    pub caption_side: CaptionSide,
    pub vertical_align: VerticalAlign,
    pub column_span: usize,
    pub row_span: usize,
}

impl Default for TableStyle {
    fn default() -> Self {
        Self {
            layout: TableLayout::Auto,
            collapse: BorderCollapse::Separate,
            spacing: [0.0; 2],
            empty_cells: true,
            caption_side: CaptionSide::Top,
            vertical_align: VerticalAlign::Baseline,
            column_span: 1,
            row_span: 1,
        }
    }
}

impl TableLayout {
    pub(super) fn from_component(value: &Component) -> Result<Self, StyleError> {
        match value.keyword() {
            Some(Keyword::Auto) => Ok(Self::Auto),
            Some(Keyword::Fixed) => Ok(Self::Fixed),
            _ => Err(unsupported(value)),
        }
    }
}
impl BorderCollapse {
    pub(super) fn from_component(value: &Component) -> Result<Self, StyleError> {
        match value.keyword() {
            Some(Keyword::Separate) => Ok(Self::Separate),
            Some(Keyword::Collapse) => Ok(Self::Collapse),
            _ => Err(unsupported(value)),
        }
    }
}
impl CaptionSide {
    pub(super) fn from_component(value: &Component) -> Result<Self, StyleError> {
        match value.keyword() {
            Some(Keyword::Top) => Ok(Self::Top),
            Some(Keyword::Bottom) => Ok(Self::Bottom),
            _ => Err(unsupported(value)),
        }
    }
}
impl VerticalAlign {
    pub(super) fn from_component(
        value: &Component,
        relative: &Relative<'_>,
    ) -> Result<Self, StyleError> {
        if matches!(value.unwrap(), Component::HtmlBaselineMiddle) {
            return Ok(Self::BaselineMiddle);
        }
        Ok(match value.keyword() {
            Some(Keyword::Baseline) => Self::Baseline,
            Some(Keyword::Top) => Self::Top,
            Some(Keyword::Middle) => Self::Middle,
            Some(Keyword::Bottom) => Self::Bottom,
            Some(Keyword::Sub) => Self::Sub,
            Some(Keyword::Super) => Self::Super,
            Some(Keyword::TextTop) => Self::TextTop,
            Some(Keyword::TextBottom) => Self::TextBottom,
            Some(_) => return Err(unsupported(value)),
            None => Self::Length(length_percentage(value, relative)?),
        })
    }
}
/// `empty-cells`: whether empty cells paint their borders and backgrounds.
pub(super) fn empty_cells(value: &Component) -> Result<bool, StyleError> {
    match value.keyword() {
        Some(Keyword::Show) => Ok(true),
        Some(Keyword::Hide) => Ok(false),
        _ => Err(unsupported(value)),
    }
}
/// `border-spacing`: one length for both axes or horizontal then vertical.
pub(super) fn spacing(value: &Component, relative: &Relative<'_>) -> Result<[f32; 2], StyleError> {
    let lengths = value
        .items()
        .iter()
        .map(|item| crate::style::values::length(item, relative))
        .collect::<Result<Vec<_>, _>>()?;
    let spacing = match lengths[..] {
        [both] => [both; 2],
        [horizontal, vertical] => [horizontal, vertical],
        _ => return Err(unsupported(value)),
    };
    if spacing.iter().any(|length| *length < 0.0) {
        return Err(format!("negative border-spacing: {value}").into());
    }
    Ok(spacing)
}
/// `table-column-span` and `table-row-span`.
pub(super) fn span(value: &Component, relative: &Relative<'_>) -> Result<usize, StyleError> {
    let span = crate::style::values::integer(value, relative)?;
    usize::try_from(span).map_err(|_| format!("negative table span: {value}").into())
}

/// HTML and `MathML` Core table structure, independently of CSS presentation.
pub(super) fn structure(style: &mut Style, element: Element<'_>) {
    let column_span = match (element.namespace(), element.name()) {
        (crate::dom::HTML_NAMESPACE, "td" | "th") => Some("colspan"),
        (crate::dom::MATHML_NAMESPACE, "mtd") => Some("columnspan"),
        _ => None,
    };
    if let Some(attribute) = column_span {
        style.table.column_span = element
            .attr(attribute)
            .and_then(crate::style::user_agent::non_negative)
            .unwrap_or(1.0)
            .clamp(1.0, MAX_HTML_COLUMN_SPAN as f64) as usize;
        style.table.row_span = element
            .attr("rowspan")
            .and_then(crate::style::user_agent::non_negative)
            .unwrap_or(1.0)
            .min(MAX_HTML_ROW_SPAN as f64) as usize;
    } else if element.namespace() == crate::dom::HTML_NAMESPACE
        && matches!(element.name(), "col" | "colgroup")
    {
        style.table.column_span = element
            .attr("span")
            .and_then(crate::style::user_agent::non_negative)
            .unwrap_or(1.0)
            .clamp(1.0, MAX_HTML_COLUMN_SPAN as f64) as usize;
    }
}
