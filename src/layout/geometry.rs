pub(super) use crate::style::SizeConstraints;
use crate::style::Style;

#[derive(Clone, PartialEq)]
pub(super) struct HorizontalBox {
    pub containing_x: f32,
    pub containing_width: f32,
    pub containing_rtl: bool,
    pub x: f32,
    pub width: f32,
    pub padding: [f32; 4],
    pub border: [f32; 4],
    pub margin: [Option<f32>; 4],
}

impl HorizontalBox {
    pub(crate) fn border_box(&self, height: f32) -> crate::graphics::Rect {
        crate::graphics::Rect {
            x: self.x - self.padding[3] - self.border[3],
            y: 0.0,
            width: self.width + self.padding[1] + self.padding[3] + self.border[1] + self.border[3],
            height,
        }
    }
}

/// Used padding and border widths, top/right/bottom/left.
#[derive(Clone, Copy)]
pub(super) struct Edges {
    pub padding: [f32; 4],
    pub border: [f32; 4],
}
impl Edges {
    /// The style's padding, resolved against `containing_width`, and border widths.
    pub(crate) fn of(style: &Style, containing_width: f32) -> Self {
        Self {
            padding: style
                .padding
                .each_ref()
                .map(|p| p.resolve(containing_width)),
            border: style
                .decoration
                .borders
                .map(crate::graphics::Border::used_width),
        }
    }
    pub(crate) fn horizontal(&self) -> f32 {
        self.padding[1] + self.padding[3] + self.border[1] + self.border[3]
    }
}

pub(super) fn horizontal(
    style: &Style,
    containing_x: f32,
    containing_width: f32,
    containing_rtl: bool,
    used_width: Option<f32>,
) -> HorizontalBox {
    horizontal_with_edges(
        style,
        containing_x,
        containing_width,
        containing_rtl,
        used_width,
        Edges::of(style, containing_width),
    )
}

/// As [`horizontal`], with used padding and border widths that differ from the style's.
pub(super) fn horizontal_with_edges(
    style: &Style,
    containing_x: f32,
    containing_width: f32,
    containing_rtl: bool,
    used_width: Option<f32>,
    edges: Edges,
) -> HorizontalBox {
    let Edges { padding, border } = edges;
    let margin = style.margin.each_ref().map(|m| m.resolve(containing_width));
    let left = margin[3];
    let right = margin[1];
    let horizontal_edges = edges.horizontal();
    let constraints = SizeConstraints::new(
        [&style.width, &style.min_width, &style.max_width],
        Some(containing_width),
        style.box_sizing,
        horizontal_edges,
    );
    let used = constraints.normal_flow(
        containing_width,
        horizontal_edges,
        [left, right],
        used_width,
        containing_rtl,
        crate::style::Overconstraint::AdjustEndMargin,
    );
    HorizontalBox {
        containing_x,
        containing_width,
        containing_rtl,
        x: containing_x + used.margins[0] + padding[3] + border[3],
        width: used.size,
        padding,
        margin,
        border,
    }
}

/// Atomic inlines start at their left margin edge and `auto` margins are zero (CSS 2.1 §10.3.9).
pub(super) fn inline_horizontal(
    style: &Style,
    containing_x: f32,
    containing_width: f32,
    containing_rtl: bool,
    used_width: Option<f32>,
    edges: Edges,
) -> HorizontalBox {
    let Edges { padding, border } = edges;
    let margin = style
        .margin
        .each_ref()
        .map(|m| Some(m.resolve(containing_width).unwrap_or(0.0)));
    let [_, right, _, left] = margin.map(|m| m.unwrap_or(0.0));
    let horizontal_edges = edges.horizontal();
    let width = used_width.unwrap_or_else(|| {
        let constraints = SizeConstraints::new(
            [&style.width, &style.min_width, &style.max_width],
            Some(containing_width),
            style.box_sizing,
            horizontal_edges,
        );
        constraints.constrain(
            constraints
                .specified
                .unwrap_or((containing_width - horizontal_edges - left - right).max(0.0)),
        )
    });
    HorizontalBox {
        containing_x,
        containing_width,
        containing_rtl,
        x: containing_x + left + border[3] + padding[3],
        width,
        padding,
        border,
        margin,
    }
}

pub(super) struct VerticalBox {
    constraints: SizeConstraints,
    edges: f32,
    table_cell: bool,
}
impl VerticalBox {
    pub(crate) fn new(
        style: &Style,
        horizontal: &HorizontalBox,
        containing_height: Option<f32>,
    ) -> Self {
        let edges = horizontal.padding[0]
            + horizontal.padding[2]
            + horizontal.border[0]
            + horizontal.border[2];
        Self {
            constraints: if style.display == crate::style::Display::TableCell {
                SizeConstraints {
                    specified: None,
                    minimum: 0.0,
                    maximum: None,
                }
            } else {
                SizeConstraints::new(
                    [&style.height, &style.min_height, &style.max_height],
                    containing_height,
                    style.box_sizing,
                    edges,
                )
            },
            edges,
            table_cell: style.display == crate::style::Display::TableCell,
        }
    }
    pub(crate) fn with_constraints(mut self, constraints: SizeConstraints) -> Self {
        self.constraints = constraints;
        self
    }
    pub(crate) fn with_used_height(mut self, height: Option<f32>) -> Self {
        if let Some(height) = height {
            self.constraints = SizeConstraints {
                specified: Some(height),
                minimum: 0.0,
                maximum: None,
            };
        }
        self
    }
    pub(crate) fn definite(&self) -> Option<f32> {
        self.constraints
            .specified
            .map(|value| self.constraints.constrain(value))
    }
    pub(crate) fn collapse_bottom(&self) -> bool {
        self.constraints.specified.is_none()
    }
    pub(crate) fn collapse_through(&self) -> bool {
        self.constraints.minimum == 0.0
            && self.constraints.specified.is_none_or(|value| value == 0.0)
    }
    pub(crate) fn used_height(&self, natural: f32) -> f32 {
        if self.table_cell {
            return self
                .constraints
                .specified
                .map_or(natural, |height| natural.min(height + self.edges));
        }
        self.constraints.constrain(
            self.constraints
                .specified
                .unwrap_or((natural - self.edges).max(0.0)),
        ) + self.edges
    }
}
