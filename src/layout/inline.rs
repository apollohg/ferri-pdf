//! Inline-level boxes: atomic inline measurement (CSS 2.1 §10.3.9, §10.8.1) and the
//! painted border boxes of inline box fragments.
use crate::document::{BlockBox, InlineBox};
use crate::graphics::{
    BoxPaint, DecorationLine, DecorationPaint, DisplayItem, Point, Rect, TextShadowPaint,
};
use crate::layout::LayoutError;
use crate::layout::geometry::{Edges, inline_horizontal};
use crate::layout::measure::{MeasuredBlock, Measurer};
use crate::style::BaselineSource;
use crate::text::{AtomicMetrics, BoxFragment, GlyphRun};

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
    let content = baseline(block, &measured);
    Ok(MeasuredAtomic {
        metrics: AtomicMetrics {
            width,
            height,
            baseline: content.map_or(height, |baseline| margin[0] + baseline),
            synthesized_baseline: content.is_none(),
        },
        block: measured,
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
        shrink_to_fit(measurer, block, containing_width, containing_height)?
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

/// `width: auto` resolves to min(max(min-content, available), max-content), or through a
/// preferred aspect ratio from a definite height (CSS Sizing 4 §4.2).
fn shrink_to_fit(
    measurer: &Measurer<'_>,
    block: &BlockBox,
    containing_width: f32,
    containing_height: Option<f32>,
) -> Result<f32, LayoutError> {
    let style = block.style();
    let edges = Edges::of(style, containing_width);
    if let Some(width) = crate::layout::ratio::width_from_height(
        measurer,
        block,
        containing_width,
        containing_height,
        &edges,
    )? {
        return Ok(width);
    }
    let box_edges = edges;
    let provisional = inline_horizontal(style, 0.0, containing_width, false, None, edges);
    let edges = edges.horizontal();
    let available = (containing_width
        - edges
        - provisional.margin[1].unwrap_or(0.0)
        - provisional.margin[3].unwrap_or(0.0))
    .max(0.0);
    let mut constraints = crate::layout::intrinsic::width_constraints(
        block,
        measurer,
        Some(containing_width),
        edges,
        Some(available),
    )?;
    crate::layout::ratio::transfer_to_width(style, &mut constraints, containing_height, &box_edges);
    let width = if let Some(width) = constraints.specified {
        width
    } else {
        let sizes = crate::layout::intrinsic::content(block, measurer, containing_width)?;
        sizes.max.min(sizes.min.max(available))
    };
    Ok(constraints.constrain(width))
}

/// Baseline below the border-box top; `None` uses the bottom margin edge. `baseline-source`
/// chooses the first or last baseline set; `auto` is the last line box for inline blocks and
/// the first baseline for other atomic containers (CSS Inline 3 §4.2.1); replaced elements,
/// scroll containers (CSS Align 3 §9.1) and layout containment boxes (CSS Containment 2
/// §3.2) have none.
fn baseline(block: &BlockBox, measured: &MeasuredBlock<'_>) -> Option<f32> {
    let style = block.style();
    if block.image().is_some() || style.is_scroll_container() || style.containment().layout {
        return None;
    }
    match style.baseline.source {
        BaselineSource::Last => measured.last_baseline(),
        BaselineSource::First => measured.first_baseline(),
        BaselineSource::Auto if style.display.is_inline_block() => measured.last_baseline(),
        BaselineSource::Auto => measured.first_baseline(),
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

/// Runs this close together share one shadow.
const SHADOW_JOIN_TOLERANCE_PT: f32 = 1.0e-3;

/// `text-shadow` layers of a line at its left edge and baseline `origin`, bottom-most
/// first: one per layer of each stretch of adjacent runs with the same shadows, covering
/// them and the decorations that `decorates` draws over them (CSS Text Decoration 3 §4).
pub(super) fn shadow_items(
    runs: &[GlyphRun],
    decorations: &[DecorationPaint],
    origin: Point,
    decorates: &dyn Fn(&GlyphRun, &DecorationPaint) -> bool,
) -> Vec<DisplayItem> {
    let mut items = Vec::new();
    let mut start = 0;
    while start < runs.len() {
        let shadows = &runs[start].shadows;
        let mut end = start + 1;
        while end < runs.len()
            && runs[end].shadows == *shadows
            && (runs[end].x - runs[end - 1].x - runs[end - 1].width).abs()
                <= SHADOW_JOIN_TOLERANCE_PT
        {
            end += 1;
        }
        let group = &runs[start..end];
        start = end;
        if shadows.is_empty() {
            continue;
        }
        let extent = [
            group[0].x,
            group[group.len() - 1].x + group[group.len() - 1].width,
        ];
        let decorations: Vec<DecorationPaint> = decorations
            .iter()
            .filter(|decoration| {
                decoration.rect.x < extent[1]
                    && decoration.rect.x + decoration.rect.width > extent[0]
                    && group.iter().any(|run| decorates(run, decoration))
            })
            .map(|decoration| DecorationPaint {
                rect: Rect {
                    x: decoration.rect.x + origin[0],
                    y: decoration.rect.y + origin[1],
                    ..decoration.rect
                },
                ..*decoration
            })
            .collect();
        for shadow in shadows.iter().rev() {
            if shadow.color.used.alpha <= 0.0 {
                continue;
            }
            items.push(DisplayItem::TextShadow(TextShadowPaint {
                origin,
                runs: group.to_vec(),
                decorations: decorations.clone(),
                extent: extent.map(|x| x + origin[0]),
                offset: shadow.offset,
                blur: shadow.blur,
                color: shadow.color.used,
            }));
        }
    }
    items
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
