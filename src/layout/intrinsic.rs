use crate::document::{BlockBox, BlockContent};
use crate::layout::measure::{BlockSource, Failure, Measurer};
use crate::style::{BoxSizing, LengthPercentage, Size};
use crate::text::IntrinsicWidths;

/// Content-box min- and max-content widths; replaced elements with only an aspect ratio
/// stretch into `available`, the width of the containing block being sized into.
pub(super) fn content<'a>(
    block: impl Into<BlockSource<'a>>,
    measurer: &Measurer<'a>,
    available: f32,
) -> Result<IntrinsicWidths, Failure> {
    let block = block.into();
    measurer.work.spend(1)?;
    if let Some(inner) = block.collapsed_table()
        && block.layout_flow().writing_mode.vertical()
            == inner.style().used_writing_mode().vertical()
    {
        return crate::layout::tables::intrinsic_content(inner, measurer, available);
    }
    if block.flow().is_some() {
        let mut probe = measurer.clone();
        let geometry = crate::layout::geometry::horizontal_with_edges(
            block.style(),
            0.0,
            available,
            false,
            None,
            measurer.box_edges(block, available)?,
        );
        let measured = probe.frame(block, &geometry, None, None)?;
        return Ok(IntrinsicWidths {
            min: measured.geometry.width,
            max: measured.geometry.width,
        });
    }
    if block.image().is_some() {
        return Ok(crate::layout::images::intrinsic(block, available));
    }
    let style = block.style();
    if style.display.is_table() {
        return crate::layout::tables::intrinsic_content(block, measurer, available);
    }
    // CSS Containment 2 §3.1 and CSS Containment 3 §3.1: inline-size containment sizes the
    // box as if it had no content, or as if its content were the explicit intrinsic inner
    // width (CSS Sizing 4 §5.2).
    let contained = style.containment().size[0];
    let explicit = style.containment.intrinsic[0]
        .explicit()
        .filter(|_| contained);
    let sizes = if let Some(width) = explicit {
        IntrinsicWidths {
            min: width,
            max: width,
        }
    } else if style.display.is_flex() {
        if contained {
            return Ok(IntrinsicWidths::default());
        }
        return crate::layout::flex::intrinsic::measure(block, measurer, available);
    } else if style.display.is_grid() {
        return crate::layout::grid::intrinsic(block, measurer, available, contained);
    } else if contained {
        IntrinsicWidths::default()
    } else {
        flow(block.block, measurer, available)?
    };
    Ok(if style.multicol.is_multicol() {
        crate::layout::multicol::intrinsic(style, sizes)
    } else {
        sizes
    })
}

/// The largest widths of the block's in-flow children.
pub(super) fn flow<'a>(
    block: &'a BlockBox,
    measurer: &Measurer<'a>,
    available: f32,
) -> Result<IntrinsicWidths, Failure> {
    let mut sizes = IntrinsicWidths::default();
    for child in block.children() {
        let child = match child {
            BlockContent::Block(child) => measure(child, measurer, available)?,
            BlockContent::Inline(paragraph) => {
                let atomics = paragraph
                    .inline_blocks()
                    .map(|atomic| measure(&atomic.block, measurer, available))
                    .collect::<Result<Vec<_>, _>>()?;
                measurer.intrinsic_widths(paragraph, &atomics)?
            }
            BlockContent::Positioned(_) | BlockContent::Events(_) => continue,
        };
        sizes.min = sizes.min.max(child.min);
        sizes.max = sizes.max.max(child.max);
    }
    Ok(sizes)
}

pub(super) fn measure<'a>(
    block: impl Into<BlockSource<'a>>,
    measurer: &Measurer<'a>,
    available: f32,
) -> Result<IntrinsicWidths, Failure> {
    let block = block.into();
    if block.style().display.is_table() {
        return crate::layout::tables::intrinsic(block, measurer, available);
    }
    Ok(outer_widths(
        block.style(),
        content(block, measurer, available)?,
        measurer.box_edges(block, 0.0)?,
    ))
}

pub(super) fn outer_widths(
    style: &crate::style::Style,
    mut sizes: IntrinsicWidths,
    box_edges: crate::layout::geometry::Edges,
) -> IntrinsicWidths {
    let edges = box_edges.horizontal();
    if let Size::Value(LengthPercentage::Length(width)) = style.width {
        let content = if style.box_sizing == BoxSizing::BorderBox {
            (width - edges).max(0.0)
        } else {
            width
        };
        sizes.min = content;
        sizes.max = content;
    }
    let natural = sizes;
    match style.width {
        Size::Intrinsic(crate::style::IntrinsicSizing::MinContent) => sizes.max = sizes.min,
        Size::Intrinsic(crate::style::IntrinsicSizing::MaxContent) => sizes.min = sizes.max,
        _ => {}
    }
    let mut constraints = crate::layout::geometry::SizeConstraints::new(
        [&Size::Auto, &style.min_width, &style.max_width],
        None,
        style.box_sizing,
        edges,
    )
    .with_intrinsic(
        [&Size::Auto, &style.min_width, &style.max_width],
        natural,
        None,
    );
    // CSS Sizing 4 §4.2–4.4: an automatic width contributes a definite height through the
    // preferred aspect ratio, and otherwise the height's limits transfer into the width's.
    if style.width == Size::Auto
        && let Some(ratio) = crate::layout::ratio::PreferredRatio::of_box(style, &box_edges)
    {
        let [_, vertical] = crate::layout::ratio::axis_edges(&box_edges);
        let heights = crate::layout::geometry::SizeConstraints::new(
            [&style.height, &style.min_height, &style.max_height],
            None,
            style.box_sizing,
            vertical,
        );
        if let Some(height) = heights.specified {
            let mut width = ratio.width(heights.constrain(height));
            if crate::layout::ratio::content_minimum(style, &style.min_width) {
                width = width.max(
                    constraints
                        .maximum
                        .map_or(natural.min, |maximum| natural.min.min(maximum)),
                );
            }
            sizes.min = width;
            sizes.max = width;
        } else {
            crate::layout::ratio::transfer_to_width(style, &mut constraints, None, &box_edges);
        }
    }
    sizes.min = constraints.constrain(sizes.min);
    sizes.max = constraints.constrain(sizes.max);
    let margins =
        style.margin[1].resolve(0.0).unwrap_or(0.0) + style.margin[3].resolve(0.0).unwrap_or(0.0);
    sizes.min = (sizes.min + edges + margins).max(0.0);
    sizes.max = (sizes.max + edges + margins).max(sizes.min);
    sizes
}

/// Width constraints with content measurements only when a keyword needs them.
pub(super) fn width_constraints<'a>(
    block: impl Into<BlockSource<'a>>,
    measurer: &Measurer<'a>,
    basis: Option<f32>,
    edges: f32,
    available: Option<f32>,
) -> Result<crate::style::SizeConstraints, Failure> {
    let block = block.into();
    let style = block.style();
    let sizes = [&style.width, &style.min_width, &style.max_width];
    let constraints = crate::style::SizeConstraints::new(sizes, basis, style.box_sizing, edges);
    if sizes.iter().any(|size| matches!(size, Size::Intrinsic(_))) {
        let natural = content(block, measurer, basis.unwrap_or(0.0))?;
        Ok(constraints.with_intrinsic(sizes, natural, available))
    } else {
        Ok(constraints)
    }
}
