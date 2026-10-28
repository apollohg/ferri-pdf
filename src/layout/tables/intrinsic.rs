use super::{captions, model::Data};
use crate::layout::LayoutError;
use crate::layout::measure::Measurer;
use crate::text::IntrinsicWidths;
use ferri_pdf_document::BlockBox;
use ferri_pdf_style::{BorderCollapse, LengthPercentage, Size, SizeConstraints};
use ferri_pdf_table::WidthInput;

pub(crate) fn measure(
    block: &BlockBox,
    measurer: &Measurer<'_>,
    available: f32,
) -> Result<IntrinsicWidths, LayoutError> {
    measure_inner(block, measurer, available, false)
}

pub(crate) fn content(
    block: &BlockBox,
    measurer: &Measurer<'_>,
    available: f32,
) -> Result<IntrinsicWidths, LayoutError> {
    measure_inner(block, measurer, available, true)
}

fn measure_inner(
    block: &BlockBox,
    measurer: &Measurer<'_>,
    available: f32,
    content_only: bool,
) -> Result<IntrinsicWidths, LayoutError> {
    let data = Data::new(block)?;
    let cells = data.widths(measurer, 0.0, available)?;
    let columns = data.column_widths();
    let style = block.style();
    let edges = data.table_edges(style, 0.0).horizontal();
    let caption_min = captions::minimum(&data.captions, measurer, available)?;
    let spacing = if style.table.collapse == BorderCollapse::Collapse {
        0.0
    } else {
        style.table.spacing[0]
    };
    let maximum = cells
        .iter()
        .map(|cell| cell.max + cell.specified.resolve(0.0).unwrap_or(0.0))
        .sum::<f32>()
        + columns
            .iter()
            .map(|column| column.specified.resolve(0.0).unwrap_or(0.0))
            .sum::<f32>()
        + spacing * (data.grid.columns + 1) as f32
        + caption_min;
    // Cyclic minima resolve percentages to zero; preferred and maximum sizes stay auto.
    let minimum = style.min_width.resolve(0.0).map_or(Size::Auto, |value| {
        Size::Value(LengthPercentage::Length(value))
    });
    let sizes = if content_only {
        [&Size::Auto; 3]
    } else {
        [&style.width, &minimum, &style.max_width]
    };
    let constraints = SizeConstraints::new(sizes, None, style.box_sizing, edges);
    let constraints = if !content_only
        && [&style.width, &style.min_width, &style.max_width]
            .iter()
            .any(|size| matches!(size, Size::Intrinsic(_)))
    {
        constraints.with_intrinsic(
            [&style.width, &style.min_width, &style.max_width],
            content(block, measurer, available)?,
            None,
        )
    } else {
        constraints
    };
    let mut input = WidthInput {
        cells: &cells,
        columns: &columns,
        layout: style.table.layout,
        table: constraints,
        available: 0.0,
        spacing,
        caption_min: (caption_min - edges).max(0.0),
    };
    let slots = data.grid.rows.saturating_mul(data.grid.columns).max(1);
    measurer.work.spend(slots)?;
    let min = data.grid.column_widths(&input)?.table;
    input.available = maximum;
    measurer.work.spend(slots)?;
    let max = data.grid.column_widths(&input)?.table.max(min);
    if content_only {
        return Ok(IntrinsicWidths { min, max });
    }
    let margins =
        style.margin[1].resolve(0.0).unwrap_or(0.0) + style.margin[3].resolve(0.0).unwrap_or(0.0);
    Ok(IntrinsicWidths {
        min: (min + edges + margins).max(0.0),
        max: (max + edges + margins).max(0.0),
    })
}
