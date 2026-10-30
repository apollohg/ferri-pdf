//! Inlined SVG libraries, each kept beside its upstream licences and provenance.
pub(crate) mod krilla_svg;
pub(crate) mod resvg;
pub(crate) mod usvg;

/// The `mask` value naming, by index, the CSS mask layers that the document attaches to SVG
/// content it writes; no style sheet can produce it.
const LAYER_MASK_FUNCTION: &str = "-ferri-pdf-mask-layers";

/// The `mask` value naming the `index`th CSS mask layers of an SVG document.
pub(crate) fn layer_mask_value(index: usize) -> String {
    format!("{LAYER_MASK_FUNCTION}({index})")
}

/// The index a `mask` value written by [`layer_mask_value`] names.
pub(crate) fn layer_mask_index(value: &str) -> Option<usize> {
    value
        .strip_prefix(LAYER_MASK_FUNCTION)?
        .strip_prefix('(')?
        .strip_suffix(')')?
        .parse()
        .ok()
}

/// Draws the CSS mask layers and mask border of SVG content (CSS Masking 1 §7, §8) as the
/// document draws those of its boxes, for both SVG renderers.
pub(crate) trait LayerMasks {
    /// Masks what `surface` draws next in the masked group's user space, where its content
    /// lies within `bounds`; returns the number of pushes to pop.
    fn push(
        &self,
        mask: &usvg::LayerMask,
        bounds: usvg::Rect,
        surface: &mut krilla::surface::Surface<'_>,
    ) -> u16;
    /// Multiplies the alpha of `pixmap`, onto which `transform` places the masked group's user
    /// space, by the mask's values.
    fn apply(
        &self,
        mask: &usvg::LayerMask,
        transform: tiny_skia::Transform,
        pixmap: &mut tiny_skia::Pixmap,
    );
}

/// A tree in which a white rectangle spanning `bounds` (points), or no area without them,
/// references SVG resource `reference` of document `xml` through presentation attribute
/// `property`, so the SVG parser resolves the resource's units against that box. Tree units
/// are CSS pixels. `layer_masks` are the CSS mask layers `xml`'s content names, and
/// `documents` supplies the other documents that references name.
pub(crate) fn reference_tree(
    xml: &str,
    layer_masks: crate::images::LayerMaskTable,
    property: &str,
    reference: &str,
    bounds: Option<crate::graphics::Rect>,
    documents: &dyn Fn(&str) -> Option<usvg::LinkedDocument>,
) -> Option<usvg::Tree> {
    use std::fmt::Write;
    let [x, y, width, height] = bounds.map_or([0.0; 4], |rect| {
        [rect.x, rect.y, rect.width, rect.height]
            .map(|value| value / crate::graphics::POINTS_PER_PIXEL)
    });
    let mut url = String::new();
    cssparser::serialize_string(reference, &mut url).expect("writing a String cannot fail");
    let mut attribute = String::new();
    crate::document::svg::escape(&mut attribute, &format!("url({url})"), true);
    let mut document = String::new();
    let _ = write!(
        document,
        "<svg xmlns=\"http://www.w3.org/2000/svg\"><defs>{xml}</defs>\
         <rect x=\"{x}\" y=\"{y}\" width=\"{width}\" height=\"{height}\" fill=\"white\" {property}=\"{attribute}\"/></svg>"
    );
    let options = usvg::Options {
        layer_masks,
        ..usvg::Options::default()
    };
    usvg::Tree::from_str_with_documents(&document, &options, documents).ok()
}

/// The `filter` element `reference` names, resolved for an element with border box
/// `bounds`, through the SVG parser's filter conversion (§7, §8).
pub(crate) fn referenced_filter(
    reference: &str,
    bounds: Option<crate::graphics::Rect>,
    definitions: &crate::document::svg::SvgDefinitions,
) -> Option<std::sync::Arc<usvg::filter::Filter>> {
    let tree = definitions.reference_tree(FILTER_ELEMENT, FILTER_ELEMENT, reference, bounds)?;
    first_filter(tree.root())
}

/// The `filter` element and the property naming it.
const FILTER_ELEMENT: &str = "filter";

pub(crate) fn first_filter(group: &usvg::Group) -> Option<std::sync::Arc<usvg::filter::Filter>> {
    group.filters().first().cloned().or_else(|| {
        group.children().iter().find_map(|node| match node {
            usvg::Node::Group(group) => first_filter(group),
            _ => None,
        })
    })
}
