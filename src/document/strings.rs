use crate::document::{
    BlockBox, DocumentError, MAX_INLINE_TEXT_BYTES,
    content::ElementText,
    counters::{CounterSnapshot, Counters},
    generated::PageNumbers,
};
use crate::dom::{DocumentTree, Element, ElementId};
use crate::style::{
    GeneratedContent, GeneratedPseudoElement, PageStyleTarget, StringAssignment, StyleError,
};
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
    /// The running element or pseudo-element.
    pub target: PageStyleTarget,
    block: BlockBox,
    /// Copies showing the page counters of the pages that place them, where its content
    /// shows page counters (CSS GCPM 3 §1.2.2).
    copies: Vec<(PageNumbers, BlockBox)>,
}
impl RunningElement {
    /// The copy a page-margin box places on a page whose `page` counter is `page` of
    /// `total` pages.
    pub(crate) fn block(&self, page: i128, total: usize) -> &BlockBox {
        self.copies
            .iter()
            .find(|(numbers, _)| numbers.matches(page, total))
            .map_or(&self.block, |(_, block)| block)
    }
    pub(crate) fn blocks(&self) -> impl Iterator<Item = &BlockBox> {
        std::iter::once(&self.block).chain(self.copies.iter().map(|(_, block)| block))
    }
}

/// A running element whose box formatting completes after its position records it.
struct PendingRunning {
    name: Arc<str>,
    target: PageStyleTarget,
    block: Option<BlockBox>,
    copies: Vec<(PageNumbers, BlockBox)>,
}

struct PendingValue {
    name: Arc<str>,
    content: GeneratedContent,
    counters: CounterSnapshot,
}
struct PendingOwner {
    owner: ElementId,
    values: Vec<PendingValue>,
    running: Option<PendingRunning>,
    before: crate::document::content::GeneratedText,
    after: crate::document::content::GeneratedText,
    marker: Option<String>,
}
pub(crate) struct StringAssignments {
    /// The index of the first of `owners`, after those of the assignments it continues.
    first: usize,
    owners: Vec<PendingOwner>,
    tree: std::rc::Rc<crate::document::semantics::SemanticTree>,
    indices: HashMap<ElementId, usize>,
}
impl StringAssignments {
    pub(crate) fn new(document: &DocumentTree) -> Self {
        Self {
            first: 0,
            owners: Vec::new(),
            indices: HashMap::new(),
            tree: std::rc::Rc::new(crate::document::semantics::SemanticTree::new(document)),
        }
    }
    /// The assignments of a copy of a subtree formatted from here: they take the indices
    /// these would next take, and are discarded.
    pub(crate) fn detached(&self) -> Self {
        Self {
            first: self.first + self.owners.len(),
            owners: Vec::new(),
            indices: HashMap::new(),
            tree: self.tree.clone(),
        }
    }
    fn owner(&mut self, index: usize) -> &mut PendingOwner {
        &mut self.owners[index - self.first]
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
        let index = self.first + self.owners.len();
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
    /// Records a running element at the position of its string assignments, if any, with its
    /// copies showing page counters.
    pub(crate) fn record_running(
        &mut self,
        element: Element<'_>,
        assignment: Option<usize>,
        name: Arc<str>,
        copies: Vec<(PageNumbers, BlockBox)>,
    ) -> usize {
        let index = assignment.unwrap_or_else(|| self.push(element, Vec::new(), None));
        self.owner(index).running = Some(PendingRunning {
            name,
            target: PageStyleTarget {
                element: element.id(),
                pseudo: None,
            },
            block: None,
            copies,
        });
        index
    }
    /// Records a running `::before` or `::after`, which assigns no strings, and returns the
    /// event that places its assignment where the pseudo-element leaves the flow.
    pub(crate) fn record_generated_running(
        &mut self,
        element: Element<'_>,
        pseudo: GeneratedPseudoElement,
        name: Arc<str>,
        block: BlockBox,
        copies: Vec<(PageNumbers, BlockBox)>,
    ) -> crate::document::SemanticEvent {
        let index = self.first + self.owners.len();
        self.owners.push(PendingOwner {
            owner: element.id(),
            values: Vec::new(),
            running: Some(PendingRunning {
                name,
                target: PageStyleTarget {
                    element: element.id(),
                    pseudo: Some(pseudo.into()),
                },
                block: Some(block),
                copies,
            }),
            before: Default::default(),
            after: Default::default(),
            marker: None,
        });
        let mut event = self.tree.event(element, Some(index), true);
        event.context.pseudo = Some(pseudo.into());
        event
    }
    pub(crate) fn running_blocks_mut(&mut self) -> impl Iterator<Item = &mut BlockBox> {
        self.owners
            .iter_mut()
            .filter_map(|owner| owner.running.as_mut())
            .flat_map(|running| {
                running
                    .block
                    .iter_mut()
                    .chain(running.copies.iter_mut().map(|(_, block)| block))
            })
    }
    pub(crate) fn attach_running(&mut self, element: ElementId, block: BlockBox) {
        let index = self.indices[&element];
        self.owner(index)
            .running
            .as_mut()
            .expect("running boxes belong to recorded running elements")
            .block = Some(block);
    }
    pub(crate) fn generated(
        &mut self,
        element: Element<'_>,
        pseudo: GeneratedPseudoElement,
        text: &str,
        style: &crate::style::Style,
    ) {
        let Some(&index) = self.indices.get(&element.id()) else {
            return;
        };
        let owner = self.owner(index);
        match pseudo {
            GeneratedPseudoElement::Before => {
                owner.before = crate::document::content::GeneratedText::capture(text, style);
            }
            GeneratedPseudoElement::After => {
                owner.after = crate::document::content::GeneratedText::capture(text, style);
            }
            GeneratedPseudoElement::FootnoteCall
            | GeneratedPseudoElement::FootnoteMarker
            | GeneratedPseudoElement::Checkmark
            | GeneratedPseudoElement::PickerIcon => {}
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
                    element: owner.running.map(|running| RunningElement {
                        name: running.name,
                        target: running.target,
                        block: running
                            .block
                            .expect("running element boxes complete with formatting"),
                        copies: running.copies,
                    }),
                })
            })
            .collect()
    }
}
