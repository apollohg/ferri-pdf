use crate::svg::usvg::{BlendMode, Node, Opacity, Transform};
use krilla::num::NormalizedF32;
use krilla::surface::Surface;

use crate::svg::krilla_svg::composite::{self, Backdrop};
use crate::svg::krilla_svg::util::{KrillaTransformExt, UsvgTransformExt, convert_blend_mode};
use crate::svg::krilla_svg::{ProcessContext, clip_path, filter, image, mask, path, text};

/// Renders `group` as the root of a stacking context: of a document, a referenced image, or
/// the content of a mask or pattern, which paint in their own streams.
pub(crate) fn render_root(
    group: &crate::svg::usvg::Group,
    surface: &mut Surface,
    process_context: &mut ProcessContext,
) {
    let to_page = surface
        .cur_transform()
        .to_usvg()
        .pre_concat(group.transform());
    render(group, Err(to_page), surface, process_context);
}

/// Renders `group`, whose content paints onto `backdrop`, or roots its own stacking context
/// whose content space the error maps onto the page.
fn render(
    group: &crate::svg::usvg::Group,
    backdrop: Result<Backdrop<'_>, Transform>,
    surface: &mut Surface,
    process_context: &mut ProcessContext,
) {
    if !group.filters().is_empty() {
        filter::render(group, surface, process_context);
        return;
    }

    let mut pop_count = 0;

    surface.push_transform(&group.transform().to_krilla());
    pop_count += 1;

    if let Some(clip_path) = group.clip_path() {
        pop_count += clip_path::render(group, clip_path, surface, process_context);
    }

    if let Some(mask) = group.mask() {
        pop_count += mask::render(mask, surface, process_context);
    }

    surface.push_blend_mode(convert_blend_mode(&group.blend_mode()));
    surface.push_opacity(NormalizedF32::new(group.opacity().get()).unwrap());
    pop_count += 2;

    // There is the `group.should_isolate` method which also returns true if
    // there is a clip path, since strictly speaking, just a clip path
    // also requires layer isolation. However, clip paths are very common
    // and it would be wasteful to generate new isolated XObjects for that
    // (especially since it shouldn't make any visual difference), so we
    // don't include this here.
    let content = match backdrop {
        Ok(backdrop) if isolates(group) => backdrop.nested(group, group.transform()),
        Ok(backdrop) => backdrop.within(group.transform()),
        Err(to_page) => Backdrop::new(group, to_page),
    };
    if isolates(group) {
        surface.push_isolated();
        pop_count += 1
    }

    for child in group.children() {
        render_node(child, group, content, surface, process_context);
    }

    for _ in 0..pop_count {
        surface.pop();
    }
}

/// Whether the group draws as an isolated transparency group.
pub(crate) fn isolates(group: &crate::svg::usvg::Group) -> bool {
    group.isolate()
        || group.opacity() != Opacity::ONE
        || group.mask().is_some()
        || group.blend_mode() != BlendMode::Normal
}

/// Renders `node`, a child of `parent` painting onto `backdrop`.
fn render_node(
    node: &Node,
    parent: &crate::svg::usvg::Group,
    backdrop: Backdrop<'_>,
    surface: &mut Surface,
    process_context: &mut ProcessContext,
) {
    if composite::in_pixels(node, parent) {
        composite::render(node, backdrop, surface, process_context);
        return;
    }
    match node {
        Node::Group(g) => render(g, Ok(backdrop), surface, process_context),
        Node::Path(p) => path::render(p, surface, process_context),
        Node::Image(i) => {
            image::render(i, surface, process_context);
        }
        // Glyph runs stroke in their own space; the outlines stroke in the host space.
        Node::Text(t) if t.non_scaling_stroke() => {
            render(t.flattened(), Ok(backdrop), surface, process_context);
        }
        Node::Text(t) => text::render(t, surface, process_context),
    }
}
