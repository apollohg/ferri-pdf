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
    let start = match (left, right, self_alignment(style, containing_rtl)) {
        (Some(left), Some(right), Some(factor)) => {
            left + factor * (containing_width - used.size - horizontal_edges - left - right)
        }
        _ => used.margins[0],
    };
    HorizontalBox {
        containing_x,
        containing_width,
        containing_rtl,
        x: containing_x + start + padding[3] + border[3],
        width: used.size,
        padding,
        margin,
        border,
    }
}

/// CSS Box Alignment 3 §6.1: a block-level box with a positional `justify-self` and no
/// `auto` margins sits at this fraction of its containing block's free space; other values
/// leave the over-constrained end margin to absorb it.
fn self_alignment(style: &Style, containing_rtl: bool) -> Option<f32> {
    use crate::style::Alignment;
    let start = if containing_rtl { 1.0 } else { 0.0 };
    match style.grid.justify_self {
        Alignment::Left => Some(0.0),
        Alignment::Right => Some(1.0),
        Alignment::Center => Some(0.5),
        Alignment::Start | Alignment::FlexStart | Alignment::SelfStart => Some(start),
        Alignment::End | Alignment::FlexEnd | Alignment::SelfEnd => Some(1.0 - start),
        _ => None,
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
    /// The height a preferred aspect ratio gives an automatic height (CSS Sizing 4 §4.2),
    /// and whether its automatic minimum is the content height (§4.3).
    ratio_height: Option<(f32, bool)>,
    /// The ratio-derived height grows to the content height.
    content_minimum: bool,
    /// Size containment lays the height out as if the content were this tall (CSS
    /// Containment 2 §3.1, CSS Sizing 4 §5.2).
    contained: Option<f32>,
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
        let table_cell = style.display == crate::style::Display::TableCell;
        let constraints = if table_cell {
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
        };
        let horizontal_edges = horizontal.padding[1]
            + horizontal.padding[3]
            + horizontal.border[1]
            + horizontal.border[3];
        let ratio_height =
            crate::layout::ratio::PreferredRatio::of(style, None, [horizontal_edges, edges]).map(
                |ratio| {
                    (
                        ratio.height(horizontal.width),
                        crate::layout::ratio::content_minimum(style, &style.min_height),
                    )
                },
            );
        Self {
            edges,
            table_cell,
            ratio_height,
            content_minimum: false,
            contained: style.containment().size[1]
                .then(|| style.containment.intrinsic[1].explicit().unwrap_or(0.0)),
            constraints: SizeConstraints {
                specified: None,
                minimum: 0.0,
                maximum: None,
            },
        }
        .with_constraints(constraints)
    }
    /// Replaces the height constraints; an automatic height keeps the ratio-derived one.
    pub(crate) fn with_constraints(mut self, mut constraints: SizeConstraints) -> Self {
        self.content_minimum = false;
        if constraints.specified.is_none()
            && !self.table_cell
            && let Some((height, content_minimum)) = self.ratio_height
        {
            constraints.specified = Some(height);
            self.content_minimum = content_minimum;
        }
        self.constraints = constraints;
        self
    }
    pub(crate) fn with_used_height(mut self, height: Option<f32>) -> Self {
        if let Some(height) = height {
            self.content_minimum = false;
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
        let content = self
            .contained
            .unwrap_or_else(|| (natural - self.edges).max(0.0));
        self.constraints
            .constrain(match self.constraints.specified {
                Some(height) if self.content_minimum => height.max(content),
                Some(height) => height,
                None => content,
            })
            + self.edges
    }
}
