use crate::document::{
    DocumentError, MAX_INLINE_TEXT_BYTES,
    content::ElementText,
    counters::{CounterSnapshot, Counters},
};
use ferri_pdf_dom::{DocumentTree, Element, ElementId};
use ferri_pdf_style::{GeneratedContent, PseudoElement, StringAssignment, StyleError};
use std::{collections::HashMap, sync::Arc};

/// Assignments at one element's semantic position, before their page is known.
pub(crate) struct RunningStringAssignment {
    pub values: Vec<StringAssignment>,
}

struct PendingValue {
    name: Arc<str>,
    content: GeneratedContent,
    counters: CounterSnapshot,
}
struct PendingOwner {
    owner: ElementId,
    values: Vec<PendingValue>,
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
        pseudo: PseudoElement,
    ) -> crate::document::SemanticEvent {
        let mut event = self.tree.event(element, None, false);
        event.context.pseudo = Some(pseudo);
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
        let index = self.owners.len();
        self.indices.insert(element.id(), index);
        self.owners.push(PendingOwner {
            owner: element.id(),
            values,
            before: Default::default(),
            after: Default::default(),
            marker: marker.map(str::to_owned),
        });
        Ok(Some(index))
    }
    pub(crate) fn generated(
        &mut self,
        element: Element<'_>,
        pseudo: PseudoElement,
        text: &str,
        style: &ferri_pdf_style::Style,
    ) {
        let Some(index) = self.indices.get(&element.id()) else {
            return;
        };
        let owner = &mut self.owners[*index];
        match pseudo {
            PseudoElement::Before => {
                owner.before = crate::document::content::GeneratedText::capture(text, style);
            }
            PseudoElement::After => {
                owner.after = crate::document::content::GeneratedText::capture(text, style);
            }
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
                Ok(RunningStringAssignment { values })
            })
            .collect()
    }
}
