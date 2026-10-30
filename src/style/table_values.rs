use crate::dom::Element;
use crate::style::{
    LengthPercentage, Style, StyleError,
    box_values::{length_percentage, unsupported},
    component::Component,
    grammar::Keyword,
    units::Relative,
};
use serde::Serialize;

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
/// How a table cell's content aligns within its rows: first-baseline cells share their
/// startmost row and last-baseline cells their endmost row (CSS Box Alignment 3 §5.4).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CellAlignment {
    Top,
    Middle,
    Bottom,
    Baseline,
    LastBaseline,
}
impl VerticalAlign {
    /// Table cells honour only `top`, `middle` and `bottom`; other values align at the baseline.
    fn cell(&self) -> CellAlignment {
        match self {
            Self::Top => CellAlignment::Top,
            Self::Middle => CellAlignment::Middle,
            Self::Bottom => CellAlignment::Bottom,
            _ => CellAlignment::Baseline,
        }
    }
}
impl Style {
    /// How a table cell aligns its content: by `align-content` other than `normal`, which
    /// otherwise takes its `vertical-align` (CSS Box Alignment 3 §5.1.1).
    pub(crate) fn cell_alignment(&self) -> CellAlignment {
        use crate::style::Alignment;
        match self.flex.align_content {
            Alignment::Normal | Alignment::Auto => self.table.vertical_align.cell(),
            Alignment::Center | Alignment::SpaceAround | Alignment::SpaceEvenly => {
                CellAlignment::Middle
            }
            Alignment::End | Alignment::FlexEnd => CellAlignment::Bottom,
            Alignment::Baseline => CellAlignment::Baseline,
            Alignment::LastBaseline => CellAlignment::LastBaseline,
            _ => CellAlignment::Top,
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
    match (element.namespace(), element.name()) {
        (crate::dom::HTML_NAMESPACE, "td" | "th") | (crate::dom::MATHML_NAMESPACE, "mtd") => {
            style.table.column_span = crate::dom::column_span(element);
            style.table.row_span = crate::dom::row_span(element);
        }
        (crate::dom::HTML_NAMESPACE, "col" | "colgroup") => {
            style.table.column_span = crate::dom::column_span(element);
        }
        _ => {}
    }
}
