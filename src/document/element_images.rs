//! `element()` images (CSS Images 4 §2.7): the element each names, what the last layout
//! measured of it, and the references whose dependency graph has a cycle.
use crate::dom::{DocumentTree, Element, ElementId, HTML_NAMESPACE, SVG_NAMESPACE, TreeRelation};
use std::collections::{HashMap, HashSet};

/// SVG elements providing a paint server, and so a paint source (§2.7.1).
const PAINT_SERVERS: [&str; 3] = ["linearGradient", "radialGradient", "pattern"];
/// HTML §4.12.5: a `canvas` element's bitmap is 300 by 150 CSS pixels unless its attributes
/// say otherwise.
const CANVAS_SIZE: [(&str, u32); 2] = [("width", 300), ("height", 150)];

/// What a layout measured of an element that `element()` names.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum ElementState {
    /// The element has no box.
    NotRendered,
    /// The element has a box that generates no stacking context.
    Unstacked,
    /// The element generates a stacking context whose decorated bounding box spans this many
    /// CSS pixels.
    Stacked([f32; 2]),
}

/// What one formatting and layout of the document found of its `element()` images: the
/// state of each element they name, and each reference, from the element whose style made it
/// (none for a page context) to the element it names.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct ElementMeasurements {
    pub(crate) states: HashMap<ElementId, ElementState>,
    pub(crate) references: HashSet<(Option<ElementId>, ElementId)>,
}

/// A paint source (§2.7.1) of an element without a box.
enum PaintSource {
    /// An `img` element's selected source and its pixel density.
    Image(String, Option<f32>),
    /// A `video` element's poster frame, if any.
    Video(Option<String>),
    /// A `canvas` element's bitmap, transparent black without script, of this many CSS pixels.
    Canvas([u32; 2]),
    /// A standalone SVG document filling its viewport with an SVG paint server.
    PaintServer(String),
}

/// The `element()` images of one formatting pass: what the last pass measured, decided once
/// for the elements it named, and the references this pass makes.
pub(crate) struct ElementImages {
    ids: std::rc::Rc<crate::document::navigation::FragmentIndex>,
    measured: ElementMeasurements,
    cyclic: HashSet<(Option<ElementId>, ElementId)>,
    sources: HashMap<ElementId, PaintSource>,
    references: std::cell::RefCell<HashSet<(Option<ElementId>, ElementId)>>,
}

/// An `element()` reference resolved against the last pass's measurements.
pub(crate) enum Resolved<'a> {
    /// An invalid image.
    Invalid,
    /// The element's own rendering, with its decorated bounding box's size in CSS pixels.
    Rendering(ElementId, [f32; 2]),
    /// An `img` element's selected source and pixel density.
    Image(&'a str, Option<f32>),
    /// A `video` element's poster frame, or transparent black without one.
    Poster(Option<&'a str>),
    /// A standalone SVG document showing a paint source.
    Svg(String),
}

impl ElementImages {
    /// `measured` is what the last pass found; `definitions` the SVG content it serialized.
    pub(crate) fn new(
        html: &DocumentTree,
        sheet: &crate::style::Stylesheet,
        ids: std::rc::Rc<crate::document::navigation::FragmentIndex>,
        measured: ElementMeasurements,
        definitions: &crate::document::svg::SvgDefinitions,
    ) -> Self {
        let cyclic = cyclic(html, &measured.references);
        let sources = measured
            .states
            .iter()
            .filter(|(_, state)| **state == ElementState::NotRendered)
            .filter_map(|(id, _)| {
                let element = html.element(*id)?;
                Some((*id, paint_source(element, sheet, definitions)?))
            })
            .collect();
        Self {
            ids,
            measured,
            cyclic,
            sources,
            references: std::cell::RefCell::default(),
        }
    }

    /// What `element(#id)` represents for the element `owner` whose style uses it, or for a
    /// page context without one, recording the reference.
    pub(crate) fn resolve(&self, id: &str, owner: Option<ElementId>) -> Resolved<'_> {
        let Some(target) = self.ids.element(id) else {
            return Resolved::Invalid;
        };
        self.references.borrow_mut().insert((owner, target));
        if self.cyclic.contains(&(owner, target)) {
            return Resolved::Invalid;
        }
        match self.measured.states.get(&target) {
            Some(ElementState::Stacked(size)) => Resolved::Rendering(target, *size),
            Some(ElementState::NotRendered) => match self.sources.get(&target) {
                Some(PaintSource::Image(url, density)) => Resolved::Image(url, *density),
                Some(PaintSource::Video(poster)) => Resolved::Poster(poster.as_deref()),
                Some(PaintSource::Canvas([width, height])) => Resolved::Svg(format!(
                    r#"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}"/>"#
                )),
                Some(PaintSource::PaintServer(xml)) => Resolved::Svg(xml.clone()),
                None => Resolved::Invalid,
            },
            Some(ElementState::Unstacked) | None => Resolved::Invalid,
        }
    }

    /// The elements to measure in a layout of this pass: those it names, and those the last
    /// pass named while laying its pages out.
    pub(crate) fn targets(&self) -> HashSet<ElementId> {
        self.references
            .borrow()
            .iter()
            .chain(&self.measured.references)
            .map(|(_, target)| *target)
            .collect()
    }

    /// The measurements a layout of this pass makes, with `states` its states of
    /// [`Self::targets`]; equal to the last pass's when its references and measurements stand.
    pub(crate) fn measured(&self, states: HashMap<ElementId, ElementState>) -> ElementMeasurements {
        ElementMeasurements {
            states,
            references: self.references.borrow().clone(),
        }
    }

    /// Whether `measurements` are those this pass resolved its references against.
    pub(crate) fn settled(&self, measurements: &ElementMeasurements) -> bool {
        *measurements == self.measured
    }
}

/// The references in a cycle of the dependency graph whose edges run from each element to
/// its children and from each element to the elements its `element()` images name (§2.7.3).
/// A reference from a page context depends on no element.
fn cyclic(
    html: &DocumentTree,
    references: &HashSet<(Option<ElementId>, ElementId)>,
) -> HashSet<(Option<ElementId>, ElementId)> {
    let contains = |ancestor: ElementId, element: ElementId| {
        html.element(element).is_some_and(|element| {
            element.id() == ancestor
                || element
                    .ancestors(TreeRelation::Selector)
                    .any(|candidate| candidate.id() == ancestor)
        })
    };
    references
        .iter()
        .filter(|(owner, target)| {
            let Some(owner) = *owner else {
                return false;
            };
            // The elements reachable from the target: its subtree, and the subtrees of the
            // elements that references from reachable elements name.
            let mut roots = vec![*target];
            let mut grown = true;
            while grown {
                grown = false;
                for (from, to) in references {
                    if let Some(from) = from
                        && !roots.contains(to)
                        && roots.iter().any(|root| contains(*root, *from))
                    {
                        roots.push(*to);
                        grown = true;
                    }
                }
            }
            roots.iter().any(|root| contains(*root, owner))
        })
        .copied()
        .collect()
}

/// The paint source `element` provides, if any.
fn paint_source(
    element: Element<'_>,
    sheet: &crate::style::Stylesheet,
    definitions: &crate::document::svg::SvgDefinitions,
) -> Option<PaintSource> {
    if element.namespace() == SVG_NAMESPACE && PAINT_SERVERS.contains(&element.name()) {
        return definitions
            .paint_server(element.attr("id")?)
            .map(PaintSource::PaintServer);
    }
    if element.namespace() != HTML_NAMESPACE {
        return None;
    }
    match element.name() {
        "img" => sheet
            .image_source(element)
            .map(|source| PaintSource::Image(source.url, source.density)),
        "video" => Some(PaintSource::Video(
            element
                .attr("poster")
                .filter(|poster| !poster.is_empty())
                .map(str::to_owned),
        )),
        "canvas" => Some(PaintSource::Canvas(CANVAS_SIZE.map(|(name, default)| {
            element
                .attr(name)
                .and_then(crate::dom::non_negative_integer)
                .unwrap_or(default)
        }))),
        _ => None,
    }
}
