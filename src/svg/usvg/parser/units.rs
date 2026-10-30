// Copyright 2019 the Resvg Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use svgtypes::{Length, LengthUnit as Unit};

use super::converter;
use super::font_units::FontLengths;
use super::svgtree::{AId, SvgNode};
use crate::style::svg_cascade::{FontUnit, Relative};
use crate::svg::usvg::Units;

pub(crate) fn checked_length(value: f32, state: &converter::State) -> Option<f32> {
    if value.is_finite() {
        Some(value)
    } else {
        state.length_error.borrow_mut().get_or_insert_with(|| {
            std::sync::Arc::new(crate::style::StyleError::from("non-finite SVG length"))
        });
        None
    }
}

fn root_element<'a, 'input>(node: SvgNode<'a, 'input>) -> SvgNode<'a, 'input> {
    node.ancestors()
        .filter(|node| node.is_element())
        .last()
        .unwrap_or(node)
}

fn relative<'a>(
    font: &'a FontLengths,
    root: &'a FontLengths,
    state: &converter::State,
) -> Relative<'a> {
    use crate::paint::POINTS_PER_PIXEL;
    let viewport = state
        .unit_viewport
        .unwrap_or([state.view_box.width(), state.view_box.height()]);
    crate::style::svg_cascade::relative(
        font,
        root,
        viewport.map(|value| value * POINTS_PER_PIXEL),
        state.opt.dpi,
    )
}

pub(super) fn with_relative<T>(
    node: SvgNode,
    state: &converter::State,
    resolve: impl FnOnce(&Relative<'_>) -> T,
) -> T {
    let font = |node: SvgNode| {
        FontLengths::new(
            Some(node),
            resolve_font_size(node, state) / node.effective_zoom(),
            state,
        )
    };
    resolve(&relative(&font(node), &font(root_element(node)), state))
}

pub(crate) fn computed_length(
    source_node: SvgNode,
    node: SvgNode,
    aid: AId,
    object_units: Units,
    state: &converter::State,
) -> Option<crate::paint::LengthPercentage> {
    use crate::paint::{LengthPercentage, POINTS_PER_PIXEL};
    let source = source_node.attribute::<&str>(aid)?;
    if let Some(length) = source_node.try_attribute::<Length>(aid) {
        return Some(if length.unit == Unit::Percent {
            LengthPercentage::Percentage(length.number as f32 / 100.0)
        } else {
            LengthPercentage::Length(checked_length(
                convert_length(length, node, aid, object_units, state),
                state,
            )?)
        });
    }
    match with_relative(node, state, |relative| {
        crate::style::svg_cascade::length(source, aid.to_str(), relative)
    }) {
        Ok(value) => {
            let zoom = if object_units == Units::ObjectBoundingBox {
                crate::style::INITIAL_ZOOM
            } else {
                node.effective_zoom()
            };
            value.and_then(|value| match value.zoomed(zoom / POINTS_PER_PIXEL) {
                LengthPercentage::Length(value) => {
                    checked_length(value, state).map(LengthPercentage::Length)
                }
                value => Some(value),
            })
        }
        Err(error) => {
            state
                .length_error
                .borrow_mut()
                .get_or_insert_with(|| std::sync::Arc::new(error));
            None
        }
    }
}

#[inline(never)]
pub(crate) fn convert_length(
    length: Length,
    node: SvgNode,
    aid: AId,
    object_units: Units,
    state: &converter::State,
) -> f32 {
    let dpi = state.opt.dpi;
    let n = length.number as f32;
    let zoom = if object_units == Units::ObjectBoundingBox {
        crate::style::INITIAL_ZOOM
    } else {
        node.effective_zoom()
    };
    match length.unit {
        Unit::None | Unit::Px => n * zoom,
        Unit::Em => n * (resolve_font_size(node, state) / node.effective_zoom()) * zoom,
        Unit::Ex => {
            let value = FontLengths::new(
                Some(node),
                resolve_font_size(node, state) / node.effective_zoom(),
                state,
            )
            .length(FontUnit::Ex)
                * zoom;
            n * value
        }
        Unit::In => n * zoom * dpi,
        Unit::Cm => n * zoom * dpi / 2.54,
        Unit::Mm => n * zoom * dpi / 25.4,
        Unit::Pt => n * zoom * dpi / 72.0,
        Unit::Pc => n * zoom * dpi / 6.0,
        Unit::Percent => {
            if object_units == Units::ObjectBoundingBox {
                n / 100.0
            } else {
                let [width, height] = state
                    .unit_viewport
                    .unwrap_or([state.view_box.width(), state.view_box.height()]);

                match aid {
                    AId::Cx
                    | AId::Dx
                    | AId::Fx
                    | AId::MarkerWidth
                    | AId::RefX
                    | AId::Rx
                    | AId::Width
                    | AId::X
                    | AId::X1
                    | AId::X2 => convert_percent(length, width),
                    AId::Cy
                    | AId::Dy
                    | AId::Fy
                    | AId::Height
                    | AId::MarkerHeight
                    | AId::RefY
                    | AId::Ry
                    | AId::Y
                    | AId::Y1
                    | AId::Y2 => convert_percent(length, height),
                    _ => {
                        let mut vb_len = width.powi(2) + height.powi(2);
                        vb_len = (vb_len / 2.0).sqrt();
                        convert_percent(length, vb_len)
                    }
                }
            }
        }
    }
}

pub(crate) fn convert_user_length(
    length: Length,
    node: SvgNode,
    aid: AId,
    state: &converter::State,
) -> f32 {
    convert_length(length, node, aid, Units::UserSpaceOnUse, state)
}

#[inline(never)]
pub(crate) fn convert_list(
    source: SvgNode,
    target: SvgNode,
    aid: AId,
    state: &converter::State,
) -> Option<Vec<f32>> {
    use crate::paint::POINTS_PER_PIXEL;
    let text = source.attribute::<&str>(aid)?;
    let values = match with_relative(source, state, |relative| {
        crate::style::svg_cascade::length_list(text, aid.to_str(), relative)
    }) {
        Ok(values) => values?,
        Err(error) => {
            state
                .length_error
                .borrow_mut()
                .get_or_insert_with(|| std::sync::Arc::new(error));
            return None;
        }
    };
    let basis = convert_user_length(Length::new(100.0, Unit::Percent), target, aid, state);
    values
        .into_iter()
        .map(|value| {
            checked_length(
                value
                    .zoomed(target.effective_zoom() / POINTS_PER_PIXEL)
                    .resolve(basis),
                state,
            )
        })
        .collect()
}

fn convert_percent(length: Length, base: f32) -> f32 {
    base * (length.number as f32) / 100.0
}

#[inline(never)]
pub(crate) fn resolve_font_size(node: SvgNode, state: &converter::State) -> f32 {
    let nodes: Vec<_> = node.ancestors().collect();
    let mut font_size = state.opt.font_size;
    let mut root_size = font_size;
    let root = root_element(node);
    for (index, n) in nodes.iter().rev().skip(1).enumerate() {
        // skip Root
        if let Some(length) = n.try_attribute::<Length>(AId::FontSize) {
            let dpi = state.opt.dpi;
            let parent = n.parent_element();
            let n = length.number as f32;
            font_size = match length.unit {
                Unit::None | Unit::Px => n,
                Unit::Em => n * font_size,
                Unit::Ex => n * FontLengths::new(parent, font_size, state).length(FontUnit::Ex),
                Unit::In => n * dpi,
                Unit::Cm => n * dpi / 2.54,
                Unit::Mm => n * dpi / 25.4,
                Unit::Pt => n * dpi / 72.0,
                Unit::Pc => n * dpi / 6.0,
                Unit::Percent => {
                    // If `font-size` has percent units that it's value
                    // is relative to the parent node `font-size`.
                    length.number as f32 * font_size * 0.01
                }
            }
        } else if let Some(source) = n.attribute(AId::FontSize) {
            use crate::paint::POINTS_PER_PIXEL;
            let font = FontLengths::new(n.parent_element(), font_size, state);
            let root_font = FontLengths::new((index != 0).then_some(root), root_size, state);
            match crate::style::svg_cascade::font_size(
                source,
                state.opt.font_size * POINTS_PER_PIXEL,
                &relative(&font, &root_font, state),
            ) {
                Ok(Some(value)) => font_size = value / POINTS_PER_PIXEL,
                Ok(None) => {}
                Err(error) => {
                    state
                        .length_error
                        .borrow_mut()
                        .get_or_insert_with(|| std::sync::Arc::new(error));
                }
            }
        }
        if index == 0 {
            root_size = font_size;
        }
    }

    checked_length(font_size * node.effective_zoom(), state).unwrap_or(state.opt.font_size)
}
