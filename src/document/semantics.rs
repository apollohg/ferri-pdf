use ferri_pdf_dom::{DocumentTree, Element, ElementId, TreeRelation};
use std::{collections::HashMap, ops::Range};

/// An element's logical position, independent of painted boxes and glyphs.
#[derive(Clone, Debug)]
pub(crate) struct SemanticEvent {
    pub order: Range<usize>,
    pub context: ferri_pdf_style::PageStyleTarget,
    pub assignment: Option<usize>,
    pub visible: bool,
}

pub(crate) struct SemanticTree(HashMap<ElementId, Range<usize>>);
impl SemanticTree {
    pub(crate) fn new(document: &DocumentTree) -> Self {
        let mut positions = HashMap::new();
        let mut next = 0;
        let mut events = vec![(document.root_element(), false)];
        while let Some((element, closing)) = events.pop() {
            if closing {
                let range: &mut Range<usize> = positions
                    .get_mut(&element.id())
                    .expect("element was entered");
                range.end = next;
            } else {
                positions.insert(element.id(), next..next);
                next += 1;
                events.push((element, true));
                events.extend(
                    element
                        .children(TreeRelation::Formatting)
                        .rev()
                        .map(|child| (child, false)),
                );
            }
        }
        Self(positions)
    }
    pub(crate) fn event(
        &self,
        element: Element<'_>,
        assignment: Option<usize>,
        visible: bool,
    ) -> SemanticEvent {
        SemanticEvent {
            order: self.0[&element.id()].clone(),
            context: ferri_pdf_style::PageStyleTarget {
                element: element.id(),
                pseudo: None,
            },
            assignment,
            visible,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct InlineEvent {
    pub offset: usize,
    pub event: SemanticEvent,
    pub(crate) anchors_before: usize,
}
