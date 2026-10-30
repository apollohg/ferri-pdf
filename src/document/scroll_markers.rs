//! The scroll marker groups formatting finds (CSS Overflow 5 §3.1), whose active markers layout
//! determines from where their scroll targets lie (§3.1.8).
use crate::dom::{ElementId, ScrollMarker};
use crate::paint::LengthPercentage;
use crate::style::{Style, logical::Flow};
use std::collections::HashMap;

/// A scroll container as the active marker calculation reads it.
#[derive(Clone, Debug)]
pub(crate) struct Scroller {
    /// Its nearest ancestor scroll container; none for the root element, whose overflow the
    /// viewport scrolls.
    pub parent: Option<ElementId>,
    /// Its writing mode and direction, which give its scrolling axes and their start sides.
    pub flow: Flow,
    /// `scroll-padding`, which insets the region scrolling into view aligns targets with.
    pub padding: [Option<LengthPercentage>; 4],
}

/// The scroll markers of a group in tree order, each with its scroll target.
#[derive(Clone, Debug, Default)]
pub(crate) struct Group {
    /// The scroll container whose `::scroll-marker-group` holds the markers; none for a
    /// `scroll-target-group` container, whose markers' and targets' nearest common ancestor
    /// scroll container the calculation starts from.
    pub scroller: Option<ElementId>,
    pub markers: Vec<(ScrollMarker, ElementId)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum GroupKey {
    /// The `::scroll-marker-group` of a scroll container.
    Markers(ElementId),
    /// An element with `scroll-target-group: auto`.
    Targets(ElementId),
}

/// The scroll marker groups of a document, and what locating their targets reads.
#[derive(Clone, Debug, Default)]
pub(crate) struct ScrollMarkerGroups {
    pub groups: Vec<Group>,
    keys: HashMap<GroupKey, usize>,
    /// Every scroll container.
    pub scrollers: HashMap<ElementId, Scroller>,
    /// The nearest ancestor scroll container of each element with a box, and the element's
    /// tree order among them.
    pub nearest: HashMap<ElementId, (ElementId, usize)>,
    /// The `scroll-margin` of each scroll target with a box, which extends the area scrolling
    /// it into view aligns.
    pub margins: HashMap<ElementId, [f32; 4]>,
}

impl ScrollMarkerGroups {
    pub(crate) fn is_empty(&self) -> bool {
        self.groups.is_empty()
    }

    /// Records the scroll container `element`, which scrolling into view also aligns.
    pub(super) fn scroller(
        &mut self,
        element: ElementId,
        parent: Option<ElementId>,
        style: &Style,
    ) {
        self.margins.insert(element, style.scroll_margin);
        self.scrollers.insert(
            element,
            Scroller {
                parent,
                flow: Flow::of(style),
                padding: style.scroll_padding.clone(),
            },
        );
    }

    /// Records that `element`, whose nearest ancestor scroll container is `scroller`, has a box.
    pub(super) fn element(&mut self, element: ElementId, scroller: ElementId) {
        let order = self.nearest.len();
        self.nearest.insert(element, (scroller, order));
    }

    pub(super) fn target(&mut self, element: ElementId, style: &Style) {
        self.margins.insert(element, style.scroll_margin);
    }

    /// Adds the `::scroll-marker` of `element` to the group of the scroll container
    /// `scroller`.
    pub(super) fn scroll_marker(&mut self, scroller: ElementId, element: ElementId) {
        self.marker(
            GroupKey::Markers(scroller),
            Some(scroller),
            ScrollMarker {
                element,
                pseudo: true,
            },
            element,
        );
    }

    /// Adds the scroll marker `element`, whose scroll target is `target`, to the group of
    /// the `scroll-target-group` container `container`.
    pub(super) fn anchor(&mut self, container: ElementId, element: ElementId, target: ElementId) {
        self.marker(
            GroupKey::Targets(container),
            None,
            ScrollMarker {
                element,
                pseudo: false,
            },
            target,
        );
    }

    fn marker(
        &mut self,
        key: GroupKey,
        scroller: Option<ElementId>,
        marker: ScrollMarker,
        target: ElementId,
    ) {
        let groups = &mut self.groups;
        let index = *self.keys.entry(key).or_insert_with(|| {
            groups.push(Group {
                scroller,
                markers: Vec::new(),
            });
            groups.len() - 1
        });
        self.groups[index].markers.push((marker, target));
    }
}
