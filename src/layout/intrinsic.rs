use crate::layout::LayoutError;
use crate::layout::measure::Measurer;
use crate::text::IntrinsicWidths;
use ferri_pdf_document::{BlockBox, BlockContent};
use ferri_pdf_style::{BoxSizing, LengthPercentage, Size};

/// Content-box min- and max-content widths; replaced elements with only an aspect ratio
/// stretch into `available`, the width of the containing block being sized into.
pub(super) fn content(
    block: &BlockBox,
    measurer: &Measurer<'_>,
    available: f32,
) -> Result<IntrinsicWidths, LayoutError> {
    measurer.work.spend(1)?;
    if block.image().is_some() {
        return Ok(crate::layout::images::intrinsic(block, available));
    }
    if block.style().display.is_table() {
        return crate::layout::tables::intrinsic_content(block, measurer, available);
    }
    if block.style().display.is_flex() {
        return crate::layout::flex::intrinsic::measure(block, measurer, available);
    }
    if block.style().display.is_grid() {
        return crate::layout::grid::intrinsic(block, measurer, available);
    }
    let sizes = flow(block, measurer, available)?;
    Ok(if block.style().multicol.is_multicol() {
        crate::layout::multicol::intrinsic(block.style(), sizes)
    } else {
        sizes
    })
}

/// The largest widths of the block's in-flow children.
fn flow(
    block: &BlockBox,
    measurer: &Measurer<'_>,
    available: f32,
) -> Result<IntrinsicWidths, LayoutError> {
    let mut sizes = IntrinsicWidths::default();
    for child in block.children() {
        let child = match child {
            BlockContent::Block(child) => measure(child, measurer, available)?,
            BlockContent::Inline(paragraph) => {
                let atomics = paragraph
                    .atomics()
                    .iter()
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

pub(super) fn measure(
    block: &BlockBox,
    measurer: &Measurer<'_>,
    available: f32,
) -> Result<IntrinsicWidths, LayoutError> {
    if block.style().display.is_table() {
        return crate::layout::tables::intrinsic(block, measurer, available);
    }
    Ok(outer_widths(
        block.style(),
        content(block, measurer, available)?,
    ))
}

pub(super) fn outer_widths(
    style: &ferri_pdf_style::Style,
    mut sizes: IntrinsicWidths,
) -> IntrinsicWidths {
    let edges = style.padding[1].resolve(0.0)
        + style.padding[3].resolve(0.0)
        + style.decoration.borders[1].used_width()
        + style.decoration.borders[3].used_width();
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
        Size::Intrinsic(ferri_pdf_style::IntrinsicSizing::MinContent) => sizes.max = sizes.min,
        Size::Intrinsic(ferri_pdf_style::IntrinsicSizing::MaxContent) => sizes.min = sizes.max,
        _ => {}
    }
    let constraints = crate::layout::geometry::SizeConstraints::new(
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
    sizes.min = constraints.constrain(sizes.min);
    sizes.max = constraints.constrain(sizes.max);
    let margins =
        style.margin[1].resolve(0.0).unwrap_or(0.0) + style.margin[3].resolve(0.0).unwrap_or(0.0);
    sizes.min = (sizes.min + edges + margins).max(0.0);
    sizes.max = (sizes.max + edges + margins).max(sizes.min);
    sizes
}

/// Width constraints with content measurements only when a keyword needs them.
pub(super) fn width_constraints(
    block: &BlockBox,
    measurer: &Measurer<'_>,
    basis: Option<f32>,
    edges: f32,
    available: Option<f32>,
) -> Result<ferri_pdf_style::SizeConstraints, LayoutError> {
    let style = block.style();
    let sizes = [&style.width, &style.min_width, &style.max_width];
    let constraints = ferri_pdf_style::SizeConstraints::new(sizes, basis, style.box_sizing, edges);
    if sizes.iter().any(|size| matches!(size, Size::Intrinsic(_))) {
        let natural = content(block, measurer, basis.unwrap_or(0.0))?;
        Ok(constraints.with_intrinsic(sizes, natural, available))
    } else {
        Ok(constraints)
    }
}
