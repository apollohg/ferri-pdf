use crate::document::{
    DocumentError, MAX_INLINE_TEXT_BYTES,
    content::ElementText,
    counters::{CounterSnapshot, Counters},
};
use crate::dom::{DocumentTree, Element, ElementId};
use crate::style::{GeneratedContent, GeneratedPseudoElement, StringAssignment, StyleError};
use std::{collections::HashMap, sync::Arc};

/// Assignments at one element's semantic position, before their page is known.
pub(crate) struct RunningStringAssignment {
    pub values: Vec<StringAssignment>,
    /// The element's box when `position: running()` moves it to page-margin boxes (CSS GCPM 3
    /// §1.2.1).
    pub element: Option<RunningElement>,
}

pub(crate) struct RunningElement {
    pub name: Arc<str>,
    pub block: crate::document::BlockBox,
}

struct PendingValue {
    name: Arc<str>,
    content: GeneratedContent,
    counters: CounterSnapshot,
}
struct PendingOwner {
    owner: ElementId,
    values: Vec<PendingValue>,
    running: Option<(Arc<str>, Option<crate::document::BlockBox>)>,
    before: crate::document::content::GeneratedText,
    after: crate::document::content::GeneratedText,
    marker: Option<String>,
}
pub(crate) struct StringAssignments {
    owners: Vec<PendingOwner>,
    tree: crate::document::semantics::SemanticTree,
    indices: HashMap<ElementId, usize>,
}
impl StringAssignments {
    pub(crate) fn new(document: &DocumentTree) -> Self {
        Self {
            owners: Vec::new(),
            indices: HashMap::new(),
            tree: crate::document::semantics::SemanticTree::new(document),
        }
    }
    pub(crate) fn event(
        &self,
        element: Element<'_>,
        assignment: Option<usize>,
        visible: bool,
    ) -> crate::document::SemanticEvent {
        self.tree.event(element, assignment, visible)
    }

    pub(crate) fn generated_event(
        &self,
        element: Element<'_>,
        pseudo: GeneratedPseudoElement,
    ) -> crate::document::SemanticEvent {
        let mut event = self.tree.event(element, None, false);
        event.context.pseudo = Some(pseudo.into());
        event
    }

    pub(crate) fn record(
        &mut self,
        element: Element<'_>,
        assignments: &[StringAssignment],
        counters: &mut Counters,
        marker: Option<&str>,
    ) -> Result<Option<usize>, StyleError> {
        if assignments.is_empty() {
            return Ok(None);
        }
        let values = assignments
            .iter()
            .map(|assignment| {
                Ok(PendingValue {
                    name: assignment.name.clone(),
                    content: assignment.content.clone(),
                    counters: counters.snapshot(&assignment.content)?,
                })
            })
            .collect::<Result<_, StyleError>>()?;
        Ok(Some(self.push(element, values, marker)))
    }
    fn push(
        &mut self,
        element: Element<'_>,
        values: Vec<PendingValue>,
        marker: Option<&str>,
    ) -> usize {
        let index = self.owners.len();
        self.indices.insert(element.id(), index);
        self.owners.push(PendingOwner {
            owner: element.id(),
            values,
            running: None,
            before: Default::default(),
            after: Default::default(),
            marker: marker.map(str::to_owned),
        });
        index
    }
    /// Records a running element at the position of its string assignments, if any.
    pub(crate) fn record_running(
        &mut self,
        element: Element<'_>,
        assignment: Option<usize>,
        name: Arc<str>,
    ) -> usize {
        let index = assignment.unwrap_or_else(|| self.push(element, Vec::new(), None));
        self.owners[index].running = Some((name, None));
        index
    }
    /// Records a running `::before` or `::after`, which assigns no strings, and returns the
    /// event that places its assignment where the pseudo-element leaves the flow.
    pub(crate) fn record_generated_running(
        &mut self,
        element: Element<'_>,
        pseudo: GeneratedPseudoElement,
        name: Arc<str>,
        block: crate::document::BlockBox,
    ) -> crate::document::SemanticEvent {
        let index = self.owners.len();
        self.owners.push(PendingOwner {
            owner: element.id(),
            values: Vec::new(),
            running: Some((name, Some(block))),
            before: Default::default(),
            after: Default::default(),
            marker: None,
        });
        let mut event = self.tree.event(element, Some(index), true);
        event.context.pseudo = Some(pseudo.into());
        event
    }
    pub(crate) fn running_blocks_mut(
        &mut self,
    ) -> impl Iterator<Item = &mut crate::document::BlockBox> {
        self.owners
            .iter_mut()
            .filter_map(|owner| owner.running.as_mut()?.1.as_mut())
    }
    pub(crate) fn attach_running(&mut self, element: ElementId, block: crate::document::BlockBox) {
        let owner = &mut self.owners[self.indices[&element]];
        owner
            .running
            .as_mut()
            .expect("running boxes belong to recorded running elements")
            .1 = Some(block);
    }
    pub(crate) fn generated(
        &mut self,
        element: Element<'_>,
        pseudo: GeneratedPseudoElement,
        text: &str,
        style: &crate::style::Style,
    ) {
        let Some(index) = self.indices.get(&element.id()) else {
            return;
        };
        let owner = &mut self.owners[*index];
        match pseudo {
            GeneratedPseudoElement::Before => {
                owner.before = crate::document::content::GeneratedText::capture(text, style);
            }
            GeneratedPseudoElement::After => {
                owner.after = crate::document::content::GeneratedText::capture(text, style);
            }
            GeneratedPseudoElement::FootnoteCall | GeneratedPseudoElement::FootnoteMarker => {}
        }
    }
    pub(crate) fn finish(
        self,
        html: &DocumentTree,
    ) -> Result<Vec<RunningStringAssignment>, DocumentError> {
        self.owners
            .into_iter()
            .map(|owner| {
                let element = html
                    .element(owner.owner)
                    .expect("assignment owner remains in the document");
                let mut text = ElementText::new(
                    element,
                    None,
                    &owner.before,
                    &owner.after,
                    owner.marker.as_deref(),
                );
                let values = owner
                    .values
                    .into_iter()
                    .map(|value| {
                        Ok(StringAssignment {
                            name: value.name,
                            content: value.content.bind_element_sources(
                                MAX_INLINE_TEXT_BYTES,
                                &mut |source| text.resolve(source),
                                &mut |request| value.counters.text(request),
                            )?,
                        })
                    })
                    .collect::<Result<_, DocumentError>>()?;
                Ok(RunningStringAssignment {
                    values,
                    element: owner.running.map(|(name, block)| RunningElement {
                        name,
                        block: block.expect("running element boxes complete with formatting"),
                    }),
                })
            })
            .collect()
    }
}
