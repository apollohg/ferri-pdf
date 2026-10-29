//! CSS Sizing 4 §4: preferred aspect ratios and the sizes they transfer between axes.
use crate::layout::measure::{BlockSource, Failure};
use crate::layout::{geometry::Edges, measure::Measurer};
use crate::style::{BoxSizing, Size, SizeConstraints, Style};

/// Width over height, converting content-box sizes.
#[derive(Clone, Copy, Debug)]
pub(super) struct PreferredRatio {
    ratio: f32,
    /// Horizontal and vertical padding and borders inside the boxes the ratio relates:
    /// `box-sizing: border-box` with a `<ratio>` that is not `auto && <ratio>` (§4.1).
    edges: [f32; 2],
}
impl PreferredRatio {
    /// A box's preferred aspect ratio, given a replaced element's natural one; internal table
    /// boxes have none (§4.1).
    pub(super) fn of(style: &Style, natural: Option<f32>, edges: [f32; 2]) -> Option<Self> {
        if style.display.is_table_internal() {
            return None;
        }
        let (ratio, sizing_box) = style.aspect_ratio.preferred(natural)?;
        Some(Self {
            ratio,
            edges: if sizing_box && style.box_sizing == BoxSizing::BorderBox {
                edges
            } else {
                [0.0; 2]
            },
        })
    }
    /// The preferred aspect ratio of a non-replaced box with `edges` padding and borders.
    pub(super) fn of_box(style: &Style, edges: &Edges) -> Option<Self> {
        Self::of(style, None, axis_edges(edges))
    }
    pub(super) fn height(self, width: f32) -> f32 {
        ((width + self.edges[0]) / self.ratio - self.edges[1]).max(0.0)
    }
    pub(super) fn width(self, height: f32) -> f32 {
        ((height + self.edges[1]) * self.ratio - self.edges[0]).max(0.0)
    }
}

/// Horizontal and vertical padding plus border widths.
pub(super) fn axis_edges(edges: &Edges) -> [f32; 2] {
    [
        edges.horizontal(),
        edges.padding[0] + edges.padding[2] + edges.border[0] + edges.border[2],
    ]
}

/// §4.3: the automatic minimum size in the ratio-dependent axis of a non-replaced box that
/// is not a scroll container is its min-content size.
pub(super) fn content_minimum(style: &Style, minimum: &Size) -> bool {
    matches!(minimum, Size::Auto) && !style.is_scroll_container()
}

/// The used content width of a non-replaced box whose width is automatic and whose height is
/// definite: the used height converted through the ratio (§4.2), within the width's own
/// limits and its automatic minimum (§4.3).
pub(super) fn width_from_height<'a>(
    measurer: &Measurer<'a>,
    block: impl Into<BlockSource<'a>>,
    containing_width: f32,
    containing_height: Option<f32>,
    edges: &Edges,
) -> Result<Option<f32>, Failure> {
    let block = block.into();
    let style = block.style();
    if block.image().is_some() || style.width != Size::Auto {
        return Ok(None);
    }
    let Some(ratio) = PreferredRatio::of_box(style, edges) else {
        return Ok(None);
    };
    let [horizontal, vertical] = axis_edges(edges);
    let heights = SizeConstraints::new(
        [&style.height, &style.min_height, &style.max_height],
        containing_height,
        style.box_sizing,
        vertical,
    );
    let Some(height) = heights.specified else {
        return Ok(None);
    };
    let widths = crate::layout::intrinsic::width_constraints(
        block,
        measurer,
        Some(containing_width),
        horizontal,
        None,
    )?;
    let mut width = ratio.width(heights.constrain(height));
    if content_minimum(style, &style.min_width) {
        let content = crate::layout::intrinsic::content(block, measurer, containing_width)?.min;
        width = width.max(
            widths
                .maximum
                .map_or(content, |maximum| content.min(maximum)),
        );
    }
    Ok(Some(widths.constrain(width)))
}

/// §4.4: the height's definite minimum and maximum, converted through the ratio into the
/// width's indefinite ones. A definite preferred or maximum width caps the transferred
/// minimum, and a definite preferred or minimum width floors the transferred maximum.
pub(super) fn transfer_to_width(
    style: &Style,
    widths: &mut SizeConstraints,
    containing_height: Option<f32>,
    edges: &Edges,
) {
    let Some(ratio) = PreferredRatio::of_box(style, edges) else {
        return;
    };
    let [_, vertical] = axis_edges(edges);
    let heights = SizeConstraints::new(
        [&style.height, &style.min_height, &style.max_height],
        containing_height,
        style.box_sizing,
        vertical,
    );
    let definite_minimum = style.min_width != Size::Auto;
    let minimum = style
        .min_height
        .resolve_definite(containing_height)
        .map(|_| ratio.width(heights.minimum))
        .map(|minimum| {
            widths
                .specified
                .into_iter()
                .chain(widths.maximum)
                .fold(minimum, f32::min)
        });
    let maximum = heights.maximum.map(|maximum| {
        widths
            .specified
            .into_iter()
            .chain(definite_minimum.then_some(widths.minimum))
            .chain(minimum)
            .fold(ratio.width(maximum), f32::max)
    });
    if !definite_minimum && let Some(minimum) = minimum {
        widths.minimum = widths.minimum.max(minimum);
    }
    if widths.maximum.is_none() {
        widths.maximum = maximum;
    }
}
