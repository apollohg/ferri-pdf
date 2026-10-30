use super::children;
use crate::flex::Item;
use crate::layout::measure::{BlockSource, Failure, Measurer};
use crate::layout::{geometry::SizeConstraints, intrinsic};
use crate::style::{FlexWrap, Size};
use crate::text::IntrinsicWidths;

pub(crate) fn measure<'a>(
    block: impl Into<BlockSource<'a>>,
    measurer: &Measurer<'a>,
    available: f32,
) -> Result<IntrinsicWidths, Failure> {
    let block = block.into();
    let style = block.style();
    let mut result = IntrinsicWidths::default();
    let children = children(block.block)?;
    let mut items = Vec::with_capacity(children.len());
    let mut minima = Vec::with_capacity(children.len());
    let mut maxima = Vec::with_capacity(children.len());
    for child in &children {
        if style.flex.direction.column() {
            let size = intrinsic::measure(*child, measurer, available)?;
            result.min = result.min.max(size.min);
            result.max = result.max.max(size.max);
            continue;
        }
        let child_style = child.style();
        let box_edges = measurer.box_edges(*child, 0.0)?;
        let edges = box_edges.horizontal();
        let margins = [
            child_style.margin[3].resolve(0.0),
            child_style.margin[1].resolve(0.0),
        ];
        let outer = edges + margins.into_iter().flatten().sum::<f32>();
        let natural = intrinsic::content(*child, measurer, available)?;
        let size = intrinsic::outer_widths(child_style, natural, box_edges);
        let constraints = SizeConstraints::new(
            [
                &child_style.width,
                &child_style.min_width,
                &child_style.max_width,
            ],
            None,
            child_style.box_sizing,
            edges,
        );
        let constraints = constraints.with_intrinsic(
            [
                &child_style.width,
                &child_style.min_width,
                &child_style.max_width,
            ],
            natural,
            None,
        );
        let basis = super::base_size(
            &child_style.flex.basis,
            &constraints,
            None,
            child_style.box_sizing,
            edges,
            natural,
            natural.max,
        );
        let minimum = if *child_style.min_width.basis() == Size::Auto {
            constraints.automatic_minimum(
                natural
                    .min
                    .min(constraints.specified.unwrap_or(f32::INFINITY))
                    .min(constraints.maximum.unwrap_or(f32::INFINITY)),
            )
        } else {
            constraints.minimum
        };
        let item = Item {
            collapsed: child_style.visibility == crate::style::Visibility::Collapse,
            basis,
            minimum,
            maximum: constraints.maximum,
            grow: child_style.flex.grow,
            shrink: child_style.flex.shrink,
            edges,
            margins,
        };
        let contribution = |value: f32| {
            let mut content = (value - outer).max(0.0);
            if item.grow == 0.0 {
                content = content.min(basis);
            }
            if item.shrink == 0.0 {
                content = content.max(basis);
            }
            content
                .min(item.maximum.unwrap_or(f32::INFINITY))
                .max(item.minimum)
        };
        let min = contribution(size.min);
        if style.flex.wrap != FlexWrap::NoWrap && !item.collapsed {
            result.min = result.min.max(min + outer);
        }
        minima.push(min);
        maxima.push(contribution(size.max));
        items.push(item);
    }
    if !style.flex.direction.column() {
        let gap = style.flex.gap[1].resolve(0.0);
        if style.flex.wrap == FlexWrap::NoWrap {
            measurer.work.spend(items.len().max(1))?;
            result.min = crate::flex::intrinsic_line(&items, &minima, gap)?;
        }
        measurer.work.spend(items.len().max(1))?;
        result.max = crate::flex::intrinsic_line(&items, &maxima, gap)?.max(result.min);
    }
    Ok(result)
}
