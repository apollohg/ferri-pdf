//! Inline-level boxes: atomic inline measurement (CSS 2.1 §10.3.9, §10.8.1) and the
//! painted border boxes of inline box fragments.
use crate::document::{BlockBox, InlineBox};
use crate::layout::LayoutError;
use crate::layout::geometry::{Edges, inline_horizontal};
use crate::layout::measure::{MeasuredBlock, Measurer};
use crate::text::{AtomicMetrics, BoxFragment};
use ferri_pdf_graphics::{BoxPaint, DecorationLine, DecorationPaint, DisplayItem, Rect};
use ferri_pdf_style::Display;

/// An atomic inline measured in its containing block, waiting for its line position.
#[derive(Clone)]
pub(super) struct MeasuredAtomic<'a> {
    /// Laid out with its left margin edge at x = 0.
    pub block: MeasuredBlock<'a>,
    pub metrics: AtomicMetrics,
    pub margin_top: f32,
}

/// Measures an atomic inline, which is laid out unbroken.
pub(super) fn measure<'a>(
    measurer: &mut Measurer<'_>,
    block: &'a BlockBox,
    containing_width: f32,
    containing_height: Option<f32>,
    containing_rtl: bool,
) -> Result<MeasuredAtomic<'a>, LayoutError> {
    let unbroken = std::mem::replace(&mut measurer.unbroken, true);
    let measured = measure_atomic(
        measurer,
        block,
        containing_width,
        containing_height,
        containing_rtl,
    );
    measurer.unbroken = unbroken;
    measured
}

fn measure_atomic<'a>(
    measurer: &mut Measurer<'_>,
    block: &'a BlockBox,
    containing_width: f32,
    containing_height: Option<f32>,
    containing_rtl: bool,
) -> Result<MeasuredAtomic<'a>, LayoutError> {
    let (measured, geometry) = measure_shrink_to_fit(
        measurer,
        block,
        containing_width,
        containing_height,
        containing_rtl,
    )?;
    let margin = geometry.margin.map(|margin| margin.unwrap_or(0.0));
    let width = margin[3]
        + geometry.border[3]
        + geometry.padding[3]
        + geometry.width
        + geometry.padding[1]
        + geometry.border[1]
        + margin[1];
    let height = margin[0] + measured.height + margin[2];
    let baseline = baseline(block, &measured).map_or(height, |baseline| margin[0] + baseline);
    Ok(MeasuredAtomic {
        block: measured,
        metrics: AtomicMetrics {
            width,
            height,
            baseline,
        },
        margin_top: margin[0],
    })
}

/// Shared shrink-to-fit geometry for atomic inlines and floating boxes.
pub(super) fn measure_shrink_to_fit<'a>(
    measurer: &mut Measurer<'_>,
    block: &'a BlockBox,
    containing_width: f32,
    containing_height: Option<f32>,
    containing_rtl: bool,
) -> Result<(MeasuredBlock<'a>, crate::layout::geometry::HorizontalBox), LayoutError> {
    let result = if block.style().display.is_table() {
        crate::layout::tables::measure_placed(
            measurer,
            block,
            0.0,
            containing_width,
            containing_rtl,
            containing_height,
            inline_horizontal,
        )?
    } else {
        let geometry = shrink_to_fit_geometry(
            measurer,
            block,
            containing_width,
            containing_height,
            containing_rtl,
        )?;
        let measured =
            measurer.format_block(block, geometry.clone(), false, containing_height, None)?;
        (measured, geometry)
    };
    Ok(result)
}

pub(super) fn shrink_to_fit_geometry(
    measurer: &Measurer<'_>,
    block: &BlockBox,
    containing_width: f32,
    containing_height: Option<f32>,
    containing_rtl: bool,
) -> Result<crate::layout::geometry::HorizontalBox, LayoutError> {
    let style = block.style();
    let width = if block.image().is_some() {
        crate::layout::images::size(
            block,
            containing_width,
            containing_width,
            containing_height,
            None,
            None,
        )[0]
    } else {
        shrink_to_fit(measurer, block, containing_width)?
    };
    let geometry = inline_horizontal(
        style,
        0.0,
        containing_width,
        containing_rtl,
        Some(width),
        Edges::of(style, containing_width),
    );
    Ok(geometry)
}

/// `width: auto` resolves to min(max(min-content, available), max-content).
fn shrink_to_fit(
    measurer: &Measurer<'_>,
    block: &BlockBox,
    containing_width: f32,
) -> Result<f32, LayoutError> {
    let style = block.style();
    let edges = Edges::of(style, containing_width);
    let provisional = inline_horizontal(style, 0.0, containing_width, false, None, edges);
    let edges = edges.horizontal();
    let available = (containing_width
        - edges
        - provisional.margin[1].unwrap_or(0.0)
        - provisional.margin[3].unwrap_or(0.0))
    .max(0.0);
    let constraints = crate::layout::intrinsic::width_constraints(
        block,
        measurer,
        Some(containing_width),
        edges,
        Some(available),
    )?;
    let width = if let Some(width) = constraints.specified {
        width
    } else {
        let sizes = crate::layout::intrinsic::content(block, measurer, containing_width)?;
        sizes.max.min(sizes.min.max(available))
    };
    Ok(constraints.constrain(width))
}

/// Baseline below the border-box top; `None` uses the bottom margin edge. Inline blocks use
/// their last line box, other atomic containers their first baseline; replaced elements and
/// scroll containers have none (CSS Align 3 §9.1).
fn baseline(block: &BlockBox, measured: &MeasuredBlock<'_>) -> Option<f32> {
    let style = block.style();
    if block.image().is_some() || style.is_scroll_container() {
        None
    } else if style.display == Display::InlineBlock {
        measured.last_baseline()
    } else {
        measured.first_baseline()
    }
}

/// Border box of an inline box fragment relative to the line's left edge and baseline, when
/// it paints or carries navigation. Vertical padding and borders surround the content area
/// without affecting the line height.
pub(super) fn fragment_paint(
    inline_box: &InlineBox,
    fragment: &BoxFragment,
    containing_width: f32,
) -> Option<BoxPaint> {
    let style = &inline_box.style;
    let padding = style
        .padding
        .each_ref()
        .map(|padding| padding.resolve(containing_width));
    let outside = |side: usize| padding[side] + style.decoration.borders[side].used_width();
    let top = fragment.baseline - fragment.ascent - outside(0);
    let bottom = fragment.baseline + fragment.descent + outside(2);
    let rect = Rect {
        x: fragment.x,
        y: top,
        width: fragment.width,
        height: bottom - top,
    };
    let mut decoration = inline_box.decoration.clone();
    if inline_box.canvas_source {
        decoration.clear_background();
    }
    if !decoration.is_visible() && inline_box.navigation.is_empty() {
        return None;
    }
    Some(BoxPaint {
        padding,
        inline_edges: fragment.edges,
        ..BoxPaint::whole(rect, decoration)
    })
}

/// A line's decorations at its left edge `x` and `baseline`: those painted below its text,
/// and the line-throughs painted over it (CSS 2.1 Appendix E.2).
pub(super) fn decoration_items(
    decorations: &[DecorationPaint],
    x: f32,
    baseline: f32,
) -> [Vec<DisplayItem>; 2] {
    let (above, below): (Vec<_>, Vec<_>) = decorations
        .iter()
        .map(|decoration| DecorationPaint {
            rect: Rect {
                x: decoration.rect.x + x,
                y: decoration.rect.y + baseline,
                ..decoration.rect
            },
            ..*decoration
        })
        .partition(|decoration| decoration.line == DecorationLine::LineThrough);
    [below, above].map(|items| items.into_iter().map(DisplayItem::Decoration).collect())
}
