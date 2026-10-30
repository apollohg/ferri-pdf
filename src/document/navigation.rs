//! Hyperlinks and their same-document targets (HTML §4.6.4, §7.4.6.4) and bookmarks
//! (CSS GCPM 3 §4), located later by the boxes that carry them.
use crate::document::{DocumentError, MAX_INLINE_TEXT_BYTES, content::element_text};
use crate::dom::{DocumentTree, Element, ElementId};
use crate::resources::{ResourceBase, Url};
use crate::style::{GeneratedContent, GeneratedPseudoElement, QuoteDepth, Quotes, Style};

use std::collections::{BTreeMap, HashMap};

const LINK_ELEMENT: &str = "a";
const HREF_ATTRIBUTE: &str = "href";
const ID_ATTRIBUTE: &str = "id";
const NAME_ATTRIBUTE: &str = "name";
/// A fragment naming the top of the document unless an element claims it (HTML §7.4.6.4).
const TOP_FRAGMENT: &str = "top";

/// The hyperlink and destination a generated box belongs to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct BoxNavigation {
    /// The innermost hyperlink containing the box, as an index into [`Navigation::links`].
    pub link: Option<usize>,
    /// The destination of the box's element, located by its first box fragment.
    pub destination: Option<usize>,
}
impl BoxNavigation {
    pub(crate) fn is_empty(&self) -> bool {
        self.link.is_none() && self.destination.is_none()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum LinkTarget {
    /// An absolute URL in its URL Standard serialization.
    Uri(String),
    /// An element of this document.
    Destination(usize),
    /// An unresolved same-document fragment retained for multipart lookup.
    Fragment(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Bookmark {
    pub level: usize,
    pub label: String,
    /// `bookmark-state: open`.
    pub open: bool,
    pub destination: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Heading {
    pub id: Option<String>,
    pub level: u8,
    pub text: String,
    pub destination: usize,
}

/// Fragment targets retained for cross-document navigation.
#[derive(Debug)]
pub(crate) struct FragmentTargets {
    pub document_url: Option<Url>,
    pub root: usize,
    pub ids: BTreeMap<String, usize>,
    pub names: BTreeMap<String, usize>,
}
impl FragmentTargets {
    /// None means an external URL; Some(None) means a missing target in this document.
    #[expect(
        clippy::option_option,
        reason = "external URLs and missing same-document targets resolve differently"
    )]
    pub(crate) fn resolve_url(&self, reference: &str) -> Option<Option<usize>> {
        let url = Url::parse(reference).ok()?;
        let document = without_fragment(self.document_url.clone()?);
        (without_fragment(url.clone()) == document).then(|| self.resolve(url.fragment()))
    }

    pub(crate) fn resolve(&self, fragment: Option<&str>) -> Option<usize> {
        indicated(
            fragment,
            |name| self.ids.get(name).or_else(|| self.names.get(name)).copied(),
            self.root,
        )
    }
}

/// Hyperlinks, and bookmarks in document order.
#[derive(Debug, Default)]
pub(crate) struct Navigation {
    pub links: Vec<LinkTarget>,
    pub bookmarks: Vec<Bookmark>,
    pub headings: Vec<Heading>,
    /// Number of destinations that boxes locate.
    pub destinations: usize,
    pub fragment_targets: Option<FragmentTargets>,
    pub(crate) element_destinations: HashMap<ElementId, usize>,
}

pub(crate) struct FragmentIndex {
    ids: BTreeMap<String, ElementId>,
    owners: std::collections::HashSet<ElementId>,
    names: BTreeMap<String, ElementId>,
    base: ResourceBase,
    document: Url,
}
impl FragmentIndex {
    fn new(
        html: &DocumentTree,
        base: &ResourceBase,
        source_base: &ResourceBase,
    ) -> Result<Self, DocumentError> {
        let mut ids = BTreeMap::new();
        let mut names = BTreeMap::new();
        for element in html.elements() {
            if let Some(id) = element.attr(ID_ATTRIBUTE).filter(|id| !id.is_empty()) {
                ids.entry(id.to_owned()).or_insert(element.id());
            }
            if element.value().name() == LINK_ELEMENT
                && let Some(name) = element.attr(NAME_ATTRIBUTE).filter(|name| !name.is_empty())
            {
                names.entry(name.to_owned()).or_insert(element.id());
            }
        }
        let owners = ids.values().copied().collect();
        Ok(Self {
            owners,
            ids,
            names,
            base: base.clone(),
            document: without_fragment(source_base.resolve("")?),
        })
    }
    pub(crate) fn contains(&self, element: ElementId) -> bool {
        self.owners.contains(&element)
    }
    pub(crate) fn target(&self, reference: &str) -> Result<ElementId, crate::style::StyleError> {
        use crate::style::{StyleError, TargetReferenceError};
        let invalid = |reason| StyleError::TargetReference {
            reference: reference.to_owned(),
            reason,
        };
        let url = self
            .base
            .resolve(reference)
            .map_err(|_| invalid(TargetReferenceError::InvalidUrl))?;
        if without_fragment(url.clone()) != self.document {
            return Err(invalid(TargetReferenceError::ExternalDocument));
        }
        let fragment = url
            .fragment()
            .filter(|fragment| !fragment.is_empty())
            .ok_or_else(|| invalid(TargetReferenceError::MissingFragment))?;
        fragment_match(fragment, |name| self.ids.get(name).copied())
            .ok_or_else(|| invalid(TargetReferenceError::MissingId))
    }
}

/// Resolves hyperlinks before boxes are built, then tracks the innermost one in tree order.
pub(crate) struct NavigationBuilder {
    links: HashMap<ElementId, usize>,
    destinations: HashMap<ElementId, usize>,
    /// Innermost hyperlink of each entered element.
    scopes: Vec<Option<usize>>,
    navigation: Navigation,
    labels: HashMap<ElementId, PendingLabel>,
    pub targets: std::rc::Rc<FragmentIndex>,
}

struct PendingLabel {
    index: Option<usize>,
    heading: Option<usize>,
    content: GeneratedContent,
    counters: crate::document::counters::CounterSnapshot,
    quotes: Quotes,
    marker: Option<String>,
    depth: QuoteDepth,
    before: crate::document::content::GeneratedText,
    after: crate::document::content::GeneratedText,
}
impl NavigationBuilder {
    pub(crate) fn new(
        html: &DocumentTree,
        base: &ResourceBase,
        capture: bool,
        document_url: Option<Url>,
        source_base: &ResourceBase,
    ) -> Result<Self, DocumentError> {
        let targets = std::rc::Rc::new(FragmentIndex::new(html, base, source_base)?);
        let mut builder = Self {
            links: HashMap::new(),
            destinations: HashMap::new(),
            scopes: Vec::new(),
            navigation: Navigation::default(),
            labels: HashMap::new(),
            targets: targets.clone(),
        };
        let root = html.root_element();
        for element in html.shadow_including_elements() {
            let Some(href) = element
                .attr(HREF_ATTRIBUTE)
                .filter(|_| element.value().name() == LINK_ELEMENT)
            else {
                continue;
            };
            // A reference that does not parse makes the hyperlink inert (HTML §4.6.4).
            let Ok(url) = base.resolve(href) else {
                continue;
            };
            let target = if without_fragment(url.clone()) == targets.document {
                // A same-document reference never leaves the document (RFC 3986 §4.4).
                match indicated(
                    url.fragment(),
                    |name| {
                        targets
                            .ids
                            .get(name)
                            .or_else(|| targets.names.get(name))
                            .copied()
                    },
                    root.id(),
                ) {
                    Some(node) => LinkTarget::Destination(builder.destination(node)),
                    None if capture => {
                        LinkTarget::Fragment(url.fragment().unwrap_or_default().to_owned())
                    }
                    None => continue,
                }
            } else {
                LinkTarget::Uri(url.into())
            };
            builder
                .links
                .insert(element.id(), builder.navigation.links.len());
            builder.navigation.links.push(target);
        }
        if capture {
            let mut targets = FragmentTargets {
                document_url,
                root: builder.destination(root.id()),
                ids: BTreeMap::new(),
                names: BTreeMap::new(),
            };
            let index = builder.targets.clone();
            for (source, target) in [
                (&index.ids, &mut targets.ids),
                (&index.names, &mut targets.names),
            ] {
                for (name, element) in source {
                    target.insert(name.clone(), builder.destination(*element));
                }
            }
            builder.navigation.fragment_targets = Some(targets);
        }
        for owner in targets.ids.values() {
            builder.destination(*owner);
        }
        Ok(builder)
    }
    fn destination(&mut self, node: ElementId) -> usize {
        let next = self.navigation.destinations;
        *self.destinations.entry(node).or_insert_with(|| {
            self.navigation.destinations += 1;
            next
        })
    }
    /// Enters `element`, returning the navigation of its boxes; [`Self::leave`] pairs with it.
    pub(crate) fn enter(&mut self, element: Element<'_>) -> BoxNavigation {
        let link = self
            .links
            .get(&element.id())
            .copied()
            .or_else(|| self.current());
        self.scopes.push(link);
        BoxNavigation {
            link,
            destination: self.destinations.get(&element.id()).copied(),
        }
    }
    pub(crate) fn leave(&mut self) {
        self.scopes.pop();
    }
    fn current(&self) -> Option<usize> {
        self.scopes.last().copied().flatten()
    }
    /// Navigation of anonymous and generated boxes inside the current element.
    pub(crate) fn descendant(&self) -> BoxNavigation {
        BoxNavigation {
            link: self.current(),
            destination: None,
        }
    }
    /// Records headings and CSS bookmarks at their shared element destination.
    pub(crate) fn record(
        &mut self,
        element: Element<'_>,
        style: &Style,
        marker: Option<&str>,
        depth: QuoteDepth,
        counters: crate::document::counters::CounterSnapshot,
        navigation: &mut BoxNavigation,
    ) {
        let heading_level = element.heading_level();
        if heading_level.is_none() && style.bookmark.level.is_none() {
            return;
        }
        let destination = self.destination(element.id());
        navigation.destination = Some(destination);
        let heading = heading_level.map(|level| {
            let index = self.navigation.headings.len();
            self.navigation.headings.push(Heading {
                id: element
                    .attr(ID_ATTRIBUTE)
                    .filter(|id| !id.is_empty())
                    .map(str::to_owned),
                level,
                text: String::new(),
                destination,
            });
            index
        });
        let index = style.bookmark.level.map(|level| {
            let index = self.navigation.bookmarks.len();
            self.navigation.bookmarks.push(Bookmark {
                level,
                label: String::new(),
                open: style.bookmark.open,
                destination,
            });
            index
        });
        self.labels.insert(
            element.id(),
            PendingLabel {
                index,
                heading,
                content: style.bookmark.label.clone(),
                counters,
                quotes: style.quotes.clone(),
                marker: marker.map(str::to_owned),
                depth,
                before: Default::default(),
                after: Default::default(),
            },
        );
    }
    pub(crate) fn generated(
        &mut self,
        element: Element<'_>,
        pseudo: GeneratedPseudoElement,
        text: &str,
        style: &Style,
    ) {
        if let Some(label) = self.labels.get_mut(&element.id()) {
            match pseudo {
                GeneratedPseudoElement::Before => {
                    label.before = crate::document::content::GeneratedText::capture(text, style);
                }
                GeneratedPseudoElement::After => {
                    label.after = crate::document::content::GeneratedText::capture(text, style);
                }
                GeneratedPseudoElement::FootnoteCall | GeneratedPseudoElement::FootnoteMarker => {}
            }
        }
    }
    pub(crate) fn finish(
        mut self,
        html: &DocumentTree,
        generated: &crate::document::generated::GeneratedReferences,
    ) -> Result<Navigation, DocumentError> {
        for (id, mut input) in self.labels {
            input.content = generated
                .bind(
                    &input.content,
                    crate::style::PageStyleTarget {
                        element: id,
                        pseudo: None,
                    },
                )?
                .into_owned();
            let element = html
                .element(id)
                .expect("navigation element remains in the document");
            if let Some(index) = input.heading {
                self.navigation.headings[index].text = element_text(element)?;
            }
            if let Some(index) = input.index {
                let text = input
                    .heading
                    .map(|heading| self.navigation.headings[heading].text.as_str());
                self.navigation.bookmarks[index].label = label(element, &input, text)?;
            }
        }
        self.navigation.element_destinations = self.destinations;
        Ok(self.navigation)
    }
}

fn without_fragment(mut url: Url) -> Url {
    url.set_fragment(None);
    url
}

/// The indicated element of a same-document `fragment` (HTML §7.4.6.4): an ID, then an `a`
/// name, first as written and then percent-decoded; an empty, absent or `top` fragment is the
/// top of the document, the root element.
fn indicated<T: Copy>(
    fragment: Option<&str>,
    find: impl Fn(&str) -> Option<T>,
    root: T,
) -> Option<T> {
    let Some(fragment) = fragment.filter(|fragment| !fragment.is_empty()) else {
        return Some(root);
    };
    fragment_match(fragment, &find).or_else(|| {
        percent_encoding::percent_decode_str(fragment)
            .decode_utf8()
            .ok()
            .filter(|decoded| decoded.eq_ignore_ascii_case(TOP_FRAGMENT))
            .map(|_| root)
    })
}

fn fragment_match<T>(fragment: &str, find: impl Fn(&str) -> Option<T>) -> Option<T> {
    find(fragment).or_else(|| {
        let decoded = percent_encoding::percent_decode_str(fragment)
            .decode_utf8()
            .ok()?;
        find(&decoded)
    })
}

fn label(
    element: Element<'_>,
    input: &PendingLabel,
    known_text: Option<&str>,
) -> Result<String, DocumentError> {
    let mut content = crate::document::content::ElementText::new(
        element,
        known_text,
        &input.before,
        &input.after,
        input.marker.as_deref(),
    );
    let mut depth = input.depth;
    Ok(input.content.element_text_with_sources(
        MAX_INLINE_TEXT_BYTES,
        &mut |source| content.resolve(source),
        &mut |request| input.counters.text(request),
        &input.quotes,
        &mut depth,
    )?)
}

#[cfg(test)]
mod target_tests {
    use super::*;
    use crate::style::{StyleError, TargetReferenceError};

    #[test]
    fn target_lookup_reuses_first_id_ownership_without_link_only_fallbacks() {
        let document = DocumentTree::parse(
            "<h2 id='chap ter'>First</h2><h2 id='chap ter'>Second</h2><a name=legacy>Named</a>",
        )
        .unwrap();
        let base: ResourceBase = Url::parse("https://example.org/docs/index.html")
            .unwrap()
            .into();
        let index = FragmentIndex::new(&document, &base, &base).unwrap();
        let target = index.target("#chap%20ter").unwrap();
        assert_eq!(
            document
                .element(target)
                .unwrap()
                .text_in(crate::dom::TreeRelation::Formatting)
                .collect::<String>(),
            "First"
        );
        for (reference, expected) in [
            ("", TargetReferenceError::MissingFragment),
            ("#", TargetReferenceError::MissingFragment),
            ("#missing", TargetReferenceError::MissingId),
            ("#legacy", TargetReferenceError::MissingId),
            ("#top", TargetReferenceError::MissingId),
            (
                "other.html#chap%20ter",
                TargetReferenceError::ExternalDocument,
            ),
            (
                "https://other.example/#chap%20ter",
                TargetReferenceError::ExternalDocument,
            ),
            ("http://[", TargetReferenceError::InvalidUrl),
        ] {
            match index.target(reference) {
                Err(StyleError::TargetReference {
                    reference: actual,
                    reason,
                }) => {
                    assert_eq!(actual, reference);
                    assert_eq!(reason, expected, "target error for {reference:?}");
                }
                result => panic!("expected {expected:?} for {reference:?}, got {result:?}"),
            }
        }
    }
}
