use super::children;
use crate::document::BlockBox;
use crate::flex::Item;
use crate::layout::measure::Measurer;
use crate::layout::{LayoutError, geometry::SizeConstraints, intrinsic};
use crate::style::{BoxSizing, FlexBasis, FlexWrap, Size};
use crate::text::IntrinsicWidths;

pub(crate) fn measure(
    block: &BlockBox,
    measurer: &Measurer<'_>,
    available: f32,
) -> Result<IntrinsicWidths, LayoutError> {
    let style = block.style();
    let mut result = IntrinsicWidths::default();
    let children = children(block)?;
    let mut items = Vec::with_capacity(children.len());
    let mut minima = Vec::with_capacity(children.len());
    let mut maxima = Vec::with_capacity(children.len());
    for child in &children {
        if style.flex.direction.column() {
            let size = intrinsic::measure(child, measurer, available)?;
            result.min = result.min.max(size.min);
            result.max = result.max.max(size.max);
            continue;
        }
        let child_style = child.style();
        let edges = child_style.padding[1].resolve(0.0)
            + child_style.padding[3].resolve(0.0)
            + child_style.decoration.borders[1].used_width()
            + child_style.decoration.borders[3].used_width();
        let margins = [
            child_style.margin[3].resolve(0.0),
            child_style.margin[1].resolve(0.0),
        ];
        let outer = edges + margins.into_iter().flatten().sum::<f32>();
        let natural = intrinsic::content(child, measurer, available)?;
        let size = intrinsic::outer_widths(child_style, natural);
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
        let basis = match &child_style.flex.basis {
            FlexBasis::Auto => constraints.specified.unwrap_or(natural.max),
            FlexBasis::Content => natural.max,
            FlexBasis::Intrinsic(value) => value.resolve(natural, None),
            value @ FlexBasis::Value(_) => value.resolve(None).map_or(natural.max, |v| {
                if child_style.box_sizing == BoxSizing::BorderBox {
                    (v - edges).max(0.0)
                } else {
                    v
                }
            }),
        };
        let minimum = if child_style.min_width == Size::Auto {
            natural
                .min
                .min(constraints.specified.unwrap_or(f32::INFINITY))
                .min(constraints.maximum.unwrap_or(f32::INFINITY))
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
