//! Inlined SVG libraries, each kept beside its upstream licences and provenance.
pub(crate) mod krilla_svg;
pub(crate) mod resvg;
pub(crate) mod usvg;

/// A tree in which a white rectangle spanning `bounds` (points), or no area without them,
/// references SVG resource `reference` of document `xml` through presentation attribute
/// `property`, so the SVG parser resolves the resource's units against that box. Tree units
/// are CSS pixels.
pub(crate) fn reference_tree(
    xml: &str,
    property: &str,
    reference: &str,
    bounds: Option<crate::graphics::Rect>,
) -> Option<usvg::Tree> {
    use std::fmt::Write;
    let [x, y, width, height] = bounds.map_or([0.0; 4], |rect| {
        [rect.x, rect.y, rect.width, rect.height]
            .map(|value| value / crate::graphics::POINTS_PER_PIXEL)
    });
    let mut document = String::new();
    let _ = write!(
        document,
        "<svg xmlns=\"http://www.w3.org/2000/svg\"><defs>{xml}</defs>\
         <rect x=\"{x}\" y=\"{y}\" width=\"{width}\" height=\"{height}\" fill=\"white\" {property}=\"url({reference})\"/></svg>"
    );
    usvg::Tree::from_str(&document, &usvg::Options::default()).ok()
}

/// The `filter` element `reference` names, resolved for an element with border box
/// `bounds`, through the SVG parser's filter conversion (§7, §8).
pub(crate) fn referenced_filter(
    reference: &str,
    bounds: Option<crate::graphics::Rect>,
    definitions: &crate::document::svg::SvgDefinitions,
) -> Option<std::sync::Arc<usvg::filter::Filter>> {
    let definition = definitions.get(reference)?;
    if definition.element != "filter" {
        return None;
    }
    let tree = reference_tree(&definition.xml, "filter", reference, bounds)?;
    first_filter(tree.root())
}

pub(crate) fn first_filter(group: &usvg::Group) -> Option<std::sync::Arc<usvg::filter::Filter>> {
    group.filters().first().cloned().or_else(|| {
        group.children().iter().find_map(|node| match node {
            usvg::Node::Group(group) => first_filter(group),
            _ => None,
        })
    })
}
