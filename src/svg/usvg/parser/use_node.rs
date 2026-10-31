// Copyright 2019 the Resvg Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use svgtypes::{Length, LengthUnit};

use super::svgtree::{AId, EId, SvgNode};
use super::{converter, style};
use crate::svg::usvg::tree::ContextElement;
use crate::svg::usvg::{Group, IsValidLength, Node, NonZeroRect, Size, Transform, ViewBox};

pub(crate) fn convert(
    node: SvgNode,
    state: &converter::State,
    cache: &mut converter::Cache,
    parent: &mut Group,
) {
    let child = match node.first_child() {
        Some(v) => v,
        None => return,
    };

    if state.parent_clip_path.is_some() && child.tag_name() == Some(EId::Symbol) {
        // Ignore `symbol` referenced by `use` inside a `clipPath`.
        // It will be ignored later anyway, but this will prevent
        // a redundant `clipPath` creation (which is required for `symbol`).
        return;
    }

    let placement = converter::element_transform(node, state, cache, parent, &|cache, probe| {
        convert_instance(node, child, state, cache, probe, Transform::default());
    });
    // The `use` element's own group carries a 3D placement.
    let transform = match placement {
        super::motion::Placement::Affine(transform) => transform,
        super::motion::Placement::Matrix(_) => Transform::default(),
        super::motion::Placement::Hidden => return,
    };
    convert_instance(node, child, state, cache, parent, transform);
}

/// The `use` element's instance of `child` under the element's own `transform`, which is
/// needed to set up the `clipPath` of a referenced `symbol`.
fn convert_instance(
    node: SvgNode,
    child: SvgNode,
    state: &converter::State,
    cache: &mut converter::Cache,
    parent: &mut Group,
    transform: Transform,
) {
    let mut use_state = state.clone();
    use_state.context_element = Some((
        style::resolve_fill(node, true, state, cache).map(|mut f| {
            f.context_element = Some(ContextElement::UseNode);
            f
        }),
        style::resolve_stroke(node, true, state, cache).map(|mut s| {
            s.context_element = Some(ContextElement::UseNode);
            s
        }),
    ));

    let mut orig_ts = transform;
    let mut new_ts = Transform::default();

    {
        let x = node.convert_user_length(AId::X, &use_state, Length::zero());
        let y = node.convert_user_length(AId::Y, &use_state, Length::zero());
        new_ts = new_ts.pre_translate(x, y);
    }

    let linked_to_symbol = child.tag_name() == Some(EId::Symbol);

    if linked_to_symbol {
        // If a `use` element has a width/height attribute and references a symbol
        // then relative units (like percentages) should be resolved relative
        // to the width/height of the `use` element, and not the original SVG.
        // This is why we need to (potentially) adapt the view box here.
        use_state.view_box = {
            let def = Length::new(100.0, LengthUnit::Percent);
            let x = use_state.view_box.x();
            let y = use_state.view_box.y();

            let width = if node.has_attribute(AId::Width) {
                node.convert_user_length(AId::Width, &use_state, def)
            } else {
                use_state.view_box.width()
            };

            let height = if node.has_attribute(AId::Height) {
                node.convert_user_length(AId::Height, &use_state, def)
            } else {
                use_state.view_box.height()
            };

            NonZeroRect::from_xywh(x, y, width, height)
                // Fail silently if the rect is not valid.
                .unwrap_or(use_state.view_box)
        };

        if let Some(ts) = viewbox_transform(node, child, &use_state) {
            new_ts = new_ts.pre_concat(ts);
        }

        let clip_rect = get_clip_rect(node, child, &use_state);
        if let Some(view_box) = child.parse_viewbox(child.effective_zoom()) {
            use_state.view_box = view_box;
        }
        if let Some((clip_rect, axes)) = clip_rect {
            let mut g = clip_element(node, orig_ts, &use_state);
            g.abs_transform = parent.abs_transform;

            // Make group for `use`.
            if let Some(mut g2) =
                converter::convert_group(node, &use_state, true, cache, &mut g, &|cache, g2| {
                    convert_children(child, new_ts, &use_state, cache, false, g2);
                })
            {
                // We must reset transform, because it was already set
                // to the group with clip-path.
                g.is_context_element = true;
                g2.id = String::new(); // Prevent ID duplication.
                g2.transform = Transform::default();
                // The viewport clip lies in the plane a 3D placement projects.
                g.matrix = g2.matrix.take();
                g.children.push(Node::Group(g2));
            }

            if g.children.is_empty() {
                return;
            }

            g.calculate_bounding_boxes();
            g.clip_path = converter::viewport_clip(cache, clip_rect, axes, g.layer_bounding_box);
            g.isolate_stacking_context();
            g.isolate_stacking_context();
            parent.children.push(Node::Group(Box::new(g)));
            return;
        }
    }

    orig_ts = orig_ts.pre_concat(new_ts);

    if linked_to_symbol {
        // Make group for `use`.
        if let Some(mut g) =
            converter::convert_group(node, &use_state, false, cache, parent, &|cache, g| {
                convert_children(child, orig_ts, &use_state, cache, false, g);
            })
        {
            g.is_context_element = true;
            g.transform = Transform::default();
            parent.children.push(Node::Group(g));
        }
    } else {
        let linked_to_svg = child.tag_name() == Some(EId::Svg);
        if linked_to_svg {
            // When a `use` element references a `svg` element,
            // we have to remember `use` element size and use it
            // instead of `svg` element size.

            let def = Length::new(100.0, LengthUnit::Percent);
            // As per usual, the SVG spec doesn't clarify this edge case,
            // but it seems like `use` size has to be reset by each `use`.
            // Meaning if we have two nested `use` elements, where one had set `width` and
            // other set `height`, we have to ignore the first `width`.
            //
            // Example:
            // <use id="use1" xlink:href="#use2" width="100"/>
            // <use id="use2" xlink:href="#svg2" height="100"/>
            // <svg id="svg2" x="40" y="40" width="80" height="80" xmlns="http://www.w3.org/2000/svg"/>
            //
            // In this case `svg2` size is 80x100 and not 100x100.
            use_state.use_size = (None, None);

            // Width and height can be set independently.
            if !node.is_contents() && node.has_attribute(AId::Width) {
                use_state.use_size.0 = Some(node.convert_user_length(AId::Width, &use_state, def));
            }
            if !node.is_contents() && node.has_attribute(AId::Height) {
                use_state.use_size.1 = Some(node.convert_user_length(AId::Height, &use_state, def));
            }

            convert_children(node, orig_ts, &use_state, cache, true, parent);
        } else {
            convert_children(node, orig_ts, &use_state, cache, true, parent);
        }
    }
}

pub(crate) fn convert_svg(
    node: SvgNode,
    state: &converter::State,
    cache: &mut converter::Cache,
    parent: &mut Group,
) {
    let viewport = SvgViewport::of(node, state);
    if let Some((clip_rect, axes)) = viewport.clip {
        let mut g = Box::new(clip_element(node, viewport.transform, state));
        g.abs_transform = parent.abs_transform;
        convert_children(
            node,
            viewport.content,
            &viewport.state,
            cache,
            false,
            &mut g,
        );
        g.calculate_bounding_boxes();
        g.clip_path = converter::viewport_clip(cache, clip_rect, axes, g.layer_bounding_box);
        parent.children.push(Node::Group(g));
    } else {
        let transform = viewport.transform.pre_concat(viewport.content);
        convert_children(node, transform, &viewport.state, cache, false, parent);
    }
}

/// The viewport a nested `svg` element establishes. Its state is boxed: nested `svg` elements
/// convert recursively.
struct SvgViewport<'a> {
    /// The state of the content, whose view box is the element's.
    state: Box<converter::State<'a>>,
    /// The element's own transform.
    transform: Transform,
    /// The transform of the content within the element.
    content: Transform,
    /// The viewport rectangle and the axes it clips.
    clip: Option<(NonZeroRect, [bool; 2])>,
}

impl<'a> SvgViewport<'a> {
    #[inline(never)]
    fn of(node: SvgNode, state: &converter::State<'a>) -> Self {
        // We require original transformation to setup 'clipPath'.
        let transform = node.resolve_transform(AId::Transform, state);
        let mut content = Transform::default();

        let x = node.convert_user_length(AId::X, state, Length::zero());
        let y = node.convert_user_length(AId::Y, state, Length::zero());
        content = content.pre_translate(x, y);

        if let Some(ts) = viewbox_transform(node, node, state) {
            content = content.pre_concat(ts);
        }

        // We have to create a new state which would have its viewBox set to the current SVG
        // element. Note that we're not updating State::size - it's a completely different
        // property.
        let mut new_state = Box::new(state.clone());
        new_state.view_box = {
            if let Some(vb) = node.parse_viewbox(node.effective_zoom()) {
                vb
            } else {
                // No `viewBox` attribute? Then use `x`, `y`, `width` and `height` instead.
                let (mut w, mut h) = use_node_size(node, state);

                // If attributes `width` and/or `height` are provided on the `use` element,
                // then these values will override the corresponding attributes
                // on the `svg` in the generated tree.
                w = state.use_size.0.unwrap_or(w);
                h = state.use_size.1.unwrap_or(h);

                NonZeroRect::from_xywh(x, y, w, h).unwrap_or(state.view_box)
            }
        };

        Self {
            state: new_state,
            transform,
            content,
            clip: get_clip_rect(node, node, state),
        }
    }
}

/// The group emulating a new viewport, which its clip path, set once its content is
/// converted, bounds; the clip cannot go on the element, whose transform would move it.
fn clip_element(node: SvgNode, transform: Transform, state: &converter::State) -> Group {
    // Nodes generated by markers must not have an ID. Otherwise we would have duplicates.
    let id = if state.parent_markers.is_empty() {
        node.element_id().to_string()
    } else {
        String::new()
    };

    Group {
        id,
        transform,
        ..Group::empty()
    }
}

fn convert_children(
    node: SvgNode,
    transform: Transform,
    state: &converter::State,
    cache: &mut converter::Cache,
    is_context_element: bool,
    parent: &mut Group,
) {
    // Temporarily adjust absolute transform so `convert_group` would account for `transform`.
    let old_abs_transform = parent.abs_transform;
    parent.abs_transform = parent.abs_transform.pre_concat(transform);

    let required = !transform.is_identity();
    if let Some(mut g) =
        converter::convert_group(node, state, required, cache, parent, &|cache, g| {
            if state.parent_clip_path.is_some() {
                converter::convert_clip_path_elements(node, state, cache, g);
            } else {
                converter::convert_children(node, state, cache, g);
            }
        })
    {
        g.is_context_element = is_context_element;
        match g.matrix {
            // The 3D placement applies after the element's viewport transform.
            Some(matrix) => {
                g.matrix = Some(crate::style::transform::conjugated(
                    &matrix,
                    crate::style::transform::IDENTITY_MATRIX,
                    [
                        transform.sx,
                        transform.ky,
                        transform.kx,
                        transform.sy,
                        transform.tx,
                        transform.ty,
                    ],
                ));
                g.transform = Transform::default();
            }
            None => g.transform = transform,
        }
        parent.children.push(Node::Group(g));
    }

    parent.abs_transform = old_abs_transform;
}

/// The viewport rectangle and the axes its `overflow` clips, `None` when it clips neither.
fn get_clip_rect(
    use_node: SvgNode,
    symbol_node: SvgNode,
    state: &converter::State,
) -> Option<(NonZeroRect, [bool; 2])> {
    let axes = converter::overflow_axes(symbol_node);
    if axes == [false; 2] {
        return None;
    }

    // A nested `svg` with only the `viewBox` attribute and no "rectangle" (x, y, width, height)
    // should not be clipped.
    if use_node.tag_name() == Some(EId::Svg) {
        // Nested `svg` referenced by `use` still should be clipped, but by `use` bounds.
        if state.use_size.0.is_none() && state.use_size.1.is_none() {
            if !(use_node.has_attribute(AId::Width) && use_node.has_attribute(AId::Height)) {
                return None;
            }
        }
    }

    let (x, y, mut w, mut h) = {
        let x = use_node.convert_user_length(AId::X, state, Length::zero());
        let y = use_node.convert_user_length(AId::Y, state, Length::zero());
        let (w, h) = use_node_size(use_node, state);
        (x, y, w, h)
    };

    if use_node.tag_name() == Some(EId::Svg) {
        // If attributes `width` and/or `height` are provided on the `use` element,
        // then these values will override the corresponding attributes
        // on the `svg` in the generated tree.
        w = state.use_size.0.unwrap_or(w);
        h = state.use_size.1.unwrap_or(h);
    }

    if !w.is_valid_length() || !h.is_valid_length() {
        return None;
    }

    NonZeroRect::from_xywh(x, y, w, h).map(|rect| (rect, axes))
}

fn use_node_size(node: SvgNode, state: &converter::State) -> (f32, f32) {
    let def = Length::new(100.0, LengthUnit::Percent);
    let w = node.convert_user_length(AId::Width, state, def);
    let h = node.convert_user_length(AId::Height, state, def);
    (w, h)
}

fn viewbox_transform(
    node: SvgNode,
    linked: SvgNode,
    state: &converter::State,
) -> Option<Transform> {
    let (mut w, mut h) = use_node_size(node, state);

    if node.tag_name() == Some(EId::Svg) {
        // If attributes `width` and/or `height` are provided on the `use` element,
        // then these values will override the corresponding attributes
        // on the `svg` in the generated tree.
        w = state.use_size.0.unwrap_or(w);
        h = state.use_size.1.unwrap_or(h);
    }

    let size = Size::from_wh(w, h)?;
    let rect = linked.parse_viewbox(linked.effective_zoom())?;
    let aspect = linked
        .attribute(AId::PreserveAspectRatio)
        .unwrap_or_default();
    let view_box = ViewBox { rect, aspect };

    Some(view_box.to_transform(size))
}
