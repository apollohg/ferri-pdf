//! The `details` element's internal shadow tree (HTML §15.5.5): a slot for its first
//! `summary` child, or a default summary, and a slot for the rest whose contents are skipped
//! (`content-visibility: hidden`) while the element is closed.
use crate::dom::{Element, HTML_NAMESPACE, Node, TreeRelation};
use crate::style::{Display, ListStylePosition, ListStyleType, Style};

/// The implementation-defined text of the default summary.
pub(super) const DEFAULT_SUMMARY: &str = "Details";

fn is_details(element: Element<'_>) -> bool {
    element.namespace() == HTML_NAMESPACE && element.name() == "details"
}

/// The `summary` child the first slot takes, which renders before the other children.
pub(super) fn summary(details: Element<'_>) -> Option<Node<'_>> {
    if !is_details(details) {
        return None;
    }
    details
        .children(TreeRelation::Formatting)
        .find(|child| child.namespace() == HTML_NAMESPACE && child.name() == "summary")
        .map(Element::as_node)
}

/// Whether `node`, a child of `parent` other than its leading summary, renders in place: the
/// second slot of a closed `details` element skips its contents.
pub(super) fn renders_child(parent: Element<'_>, node: Node<'_>) -> bool {
    if !is_details(parent) {
        return true;
    }
    parent.attr("open").is_some() && summary(parent) != Some(node)
}

/// The style of the default summary of a `details` element without a `summary` child: a
/// list item with an inside disclosure marker (`:host summary`).
pub(super) fn default_summary_style(details: Element<'_>, style: &Style) -> Option<Style> {
    if !is_details(details) || summary(details).is_some() {
        return None;
    }
    let mut summary = style.anonymous(Display::ListItem);
    summary.list_style_type = ListStyleType::disclosure(details.attr("open").is_some());
    summary.list_style_position = ListStylePosition::Inside;
    Some(summary)
}
