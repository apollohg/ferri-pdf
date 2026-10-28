//! Link annotations (PDF 1.7 §12.5.6.5), the document outline (§12.3.3) and the document
//! information dictionary with its XMP counterpart (§14.3).
use crate::document::{Bookmark, DocumentMetadata, LinkTarget};
use crate::layout::{Destination, LinkArea, LinkShape};
use crate::writer::PdfError;
use krilla::{
    action::LinkAction,
    annotation::{LinkAnnotation, Target},
    destination::XyzDestination,
    geom::{Point, Quadrilateral, Rect},
    metadata::Metadata,
    outline::{Outline, OutlineNode},
    page::Page,
};

const PRODUCER: &str = concat!("ferri-pdf ", env!("CARGO_PKG_VERSION"));
/// Destinations show the page's full width.
const DESTINATION_LEFT: f32 = 0.0;

pub(super) fn destinations(
    located: &[Option<Destination>],
    page_offset: usize,
) -> Vec<Option<XyzDestination>> {
    located
        .iter()
        .map(|destination| {
            destination.map(|destination| {
                XyzDestination::new(
                    destination.page + page_offset,
                    Point::from_xy(DESTINATION_LEFT, destination.top),
                )
            })
        })
        .collect()
}

pub(super) enum ResolvedLink {
    Uri(String),
    Destination(XyzDestination),
}

pub(super) fn resolve_links(
    parts: &[crate::writer::PdfPart<'_>],
    destinations: &[Vec<Option<XyzDestination>>],
) -> Vec<Vec<Option<ResolvedLink>>> {
    let target = |part: usize, index: usize| {
        destinations[part][index]
            .clone()
            .map(ResolvedLink::Destination)
    };
    parts
        .iter()
        .enumerate()
        .map(|(current, part)| {
            part.navigation
                .links
                .iter()
                .map(|link| match link {
                    LinkTarget::Destination(index) => target(current, *index),
                    LinkTarget::Fragment(fragment) => parts
                        .iter()
                        .enumerate()
                        .find_map(|(index, candidate)| {
                            candidate
                                .navigation
                                .fragment_targets
                                .as_ref()?
                                .resolve(Some(fragment))
                                .map(|destination| (index, destination))
                        })
                        .and_then(|(index, destination)| target(index, destination)),
                    LinkTarget::Uri(uri) => {
                        match parts.iter().enumerate().find_map(|(index, candidate)| {
                            candidate
                                .navigation
                                .fragment_targets
                                .as_ref()?
                                .resolve_url(uri)
                                .map(|destination| (index, destination))
                        }) {
                            Some((index, destination)) => {
                                destination.and_then(|destination| target(index, destination))
                            }
                            None => Some(ResolvedLink::Uri(uri.clone())),
                        }
                    }
                })
                .collect()
        })
        .collect()
}

/// Adds a page's link annotations; links to destinations without a box are inert.
pub(super) fn annotate(
    page: &mut Page<'_>,
    areas: &[LinkArea],
    links: &[Option<ResolvedLink>],
) -> Result<(), PdfError> {
    for area in areas {
        let target = match &links[area.link] {
            Some(ResolvedLink::Uri(uri)) => Target::Action(LinkAction::new(uri.clone()).into()),
            Some(ResolvedLink::Destination(destination)) => {
                Target::Destination(destination.clone().into())
            }
            None => continue,
        };
        let annotation = match &area.shape {
            LinkShape::Rect(rect) => LinkAnnotation::new(
                Rect::from_xywh(rect.x, rect.y, rect.width, rect.height)
                    .ok_or(PdfError::InvalidGeometry)?,
                target,
            ),
            LinkShape::Quads(quads) => LinkAnnotation::new_with_quad_points(
                quads
                    .iter()
                    .map(|quad| Quadrilateral(quad.map(|[x, y]| Point::from_xy(x, y))))
                    .collect(),
                target,
            ),
        };
        page.add_annotation(annotation.into());
    }
    Ok(())
}

/// Bookmarks whose elements have boxes, each nested under the closest preceding bookmark of
/// a lower level (CSS GCPM 3 §4.1).
pub(super) fn append_outline(
    outline: &mut Outline,
    bookmarks: &[Bookmark],
    destinations: &[Option<XyzDestination>],
) {
    let entries = bookmarks.iter().filter_map(|bookmark| {
        destinations[bookmark.destination]
            .clone()
            .map(|destination| {
                (
                    bookmark.level,
                    OutlineNode::new(bookmark.label.clone(), destination).with_open(bookmark.open),
                )
            })
    });
    for node in nest(entries, OutlineNode::push_child) {
        outline.push_child(node);
    }
}

/// Builds a forest from `(level, node)` entries in document order.
fn nest<T>(entries: impl IntoIterator<Item = (usize, T)>, attach: impl Fn(&mut T, T)) -> Vec<T> {
    let mut roots = Vec::new();
    let mut open: Vec<(usize, T)> = Vec::new();
    let close = |open: &mut Vec<(usize, T)>, roots: &mut Vec<T>| {
        let (_, node) = open.pop().expect("closing an open entry");
        match open.last_mut() {
            Some((_, parent)) => attach(parent, node),
            None => roots.push(node),
        }
    };
    for (level, node) in entries {
        while open
            .last()
            .is_some_and(|(open_level, _)| *open_level >= level)
        {
            close(&mut open, &mut roots);
        }
        open.push((level, node));
    }
    while !open.is_empty() {
        close(&mut open, &mut roots);
    }
    roots
}

pub(super) fn metadata(document: &DocumentMetadata) -> Metadata {
    let mut metadata = Metadata::new()
        .authors(document.authors.clone())
        .keywords(document.keywords.clone())
        .producer(PRODUCER.to_owned());
    if let Some(title) = &document.title {
        metadata = metadata.title(title.clone());
    }
    if let Some(subject) = &document.subject {
        metadata = metadata.description(subject.clone());
    }
    if let Some(generator) = &document.generator {
        metadata = metadata.creator(generator.clone());
    }
    if let Some(language) = &document.language {
        metadata = metadata.language(language.clone());
    }
    metadata
}

#[cfg(test)]
mod tests {
    use super::nest;

    #[derive(Debug, PartialEq)]
    struct Node(&'static str, Vec<Node>);

    #[test]
    fn bookmarks_nest_under_the_closest_preceding_lower_level() {
        let entries = [
            (1, "h1"),
            (3, "skipped h3"),
            (2, "h2"),
            (4, "deep"),
            (1, "second h1"),
            (2, "under second"),
            (1, "third h1"),
        ]
        .map(|(level, label)| (level, Node(label, Vec::new())));
        let leaf = |label| Node(label, Vec::new());
        assert_eq!(
            nest(entries, |parent: &mut Node, child| parent.1.push(child)),
            vec![
                Node(
                    "h1",
                    vec![leaf("skipped h3"), Node("h2", vec![leaf("deep")])]
                ),
                Node("second h1", vec![leaf("under second")]),
                leaf("third h1"),
            ]
        );
    }

    #[test]
    fn a_document_starting_below_level_one_keeps_later_lower_levels_at_the_top() {
        let entries = [(3, "h3"), (2, "h2"), (3, "h3 child")]
            .map(|(level, label)| (level, Node(label, vec![])));
        assert_eq!(
            nest(entries, |parent: &mut Node, child| parent.1.push(child)),
            vec![
                Node("h3", vec![]),
                Node("h2", vec![Node("h3 child", vec![])])
            ]
        );
    }
}
