use crate::layout::measure::{BlockSource, Failure, FlowItem, MeasuredBlock, Measurer};
use crate::style::StyleError;

pub(super) fn baseline<'a>(
    measurer: &Measurer<'a>,
    source: BlockSource<'a>,
    measured: &MeasuredBlock<'a>,
) -> Result<Option<f32>, Failure> {
    if source.style().display.is_table()
        && source
            .math()
            .is_some_and(|math| math.name.as_ref() == "mtable")
    {
        let axis = super::Metrics::of(
            measurer.text.fonts(),
            source.style(),
            measurer.text.resolver(),
        )
        .map_err(StyleError::Font)?
        .axis;
        return Ok(Some(measured.height / 2.0 + axis));
    }
    Ok(None)
}

pub(crate) fn align_cell(cell: &mut MeasuredBlock<'_>) {
    if cell
        .source
        .math()
        .is_none_or(|math| math.name.as_ref() != "mtd")
    {
        return;
    }
    for item in &mut cell.items {
        if let FlowItem::Block(row) = item
            && row.source.is_anonymous()
            && row
                .source
                .math()
                .is_some_and(|math| !math.principal && math.name.as_ref() == "mrow")
        {
            let width = row.geometry.border_box(row.height).width;
            row.offset[0] +=
                cell.style
                    .align
                    .offset(cell.geometry.width, width, cell.style.used_rtl());
        }
    }
}
