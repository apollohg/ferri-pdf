//! Demand-driven child layouts keep native math recursion off the machine stack.
use super::{Builder, DocumentError, Element, Form, MathBox, MathLayout, Stretch, Style};
use crate::style::StyleError;
use std::sync::Arc;

#[derive(Eq, Hash, PartialEq)]
enum Constraint {
    Inline(u32),
    Block(u32, u32),
}
#[derive(Eq, Hash, PartialEq)]
pub(super) struct Key {
    element: crate::dom::ElementId,
    form: Option<Form>,
    constraint: Option<Constraint>,
    root: bool,
}
impl Key {
    fn of(element: Element<'_>, layout: MathLayout, root: bool) -> Self {
        Self {
            element: element.id(),
            form: layout.form,
            constraint: layout.stretch.map(|stretch| match stretch {
                Stretch::Inline(size) => Constraint::Inline(size.to_bits()),
                Stretch::Block(ascent, descent) => {
                    Constraint::Block(ascent.to_bits(), descent.to_bits())
                }
            }),
            root,
        }
    }
}
enum StyleSource {
    Computed(Arc<Style>),
    Parent(Arc<Style>),
}
pub(super) struct Job<'d> {
    element: Element<'d>,
    style: StyleSource,
    depth: usize,
    layout: MathLayout,
    root: bool,
}
pub(super) enum MathError<'d> {
    Document(DocumentError),
    Children(Vec<Job<'d>>),
}
#[derive(Default)]
pub(super) struct Requests<'d>(Vec<Job<'d>>);
impl<'d> Requests<'d> {
    pub(super) fn collect<T>(
        &mut self,
        result: Result<T, MathError<'d>>,
    ) -> Result<Option<T>, MathError<'d>> {
        match result {
            Ok(value) => Ok(Some(value)),
            Err(MathError::Children(children)) => {
                self.0.extend(children);
                Ok(None)
            }
            Err(error) => Err(error),
        }
    }
    pub(super) fn finish(self) -> Result<(), MathError<'d>> {
        if self.0.is_empty() {
            Ok(())
        } else {
            Err(MathError::Children(self.0))
        }
    }
}
impl From<DocumentError> for MathError<'_> {
    fn from(error: DocumentError) -> Self {
        Self::Document(error)
    }
}
impl From<StyleError> for MathError<'_> {
    fn from(error: StyleError) -> Self {
        Self::Document(error.into())
    }
}
impl<'d> Builder<'_, 'd> {
    pub(super) fn run(
        &mut self,
        element: Element<'d>,
        style: &Style,
    ) -> Result<MathBox, DocumentError> {
        let layout = MathLayout::default();
        self.jobs.push(Job {
            element,
            style: StyleSource::Computed(Arc::new(style.clone())),
            depth: 0,
            layout,
            root: true,
        });
        while let Some(mut job) = self.jobs.pop() {
            let style = match &job.style {
                StyleSource::Computed(style) => style.clone(),
                StyleSource::Parent(parent) => Arc::new(self.sheet.computed_with_resolver(
                    job.element,
                    parent,
                    self.resolver,
                )?),
            };
            job.style = StyleSource::Computed(style.clone());
            match self.layout_element(job.element, &style, job.depth, job.root, job.layout) {
                Ok(content) => {
                    self.cache
                        .insert(Key::of(job.element, job.layout, job.root), content);
                }
                Err(MathError::Children(children)) => {
                    self.jobs.push(job);
                    self.jobs.extend(children.into_iter().rev());
                }
                Err(MathError::Document(error)) => return Err(error),
            }
        }
        Ok(self
            .cache
            .remove(&Key::of(element, layout, true))
            .expect("completed root math layout"))
    }
    pub(super) fn element(
        &mut self,
        element: Element<'d>,
        parent: &Arc<Style>,
        depth: usize,
        root: bool,
        layout: MathLayout,
    ) -> Result<MathBox, MathError<'d>> {
        if let Some(content) = self.cache.get(&Key::of(element, layout, root)) {
            return Ok(content.clone());
        }
        if depth >= super::MAX_MATH_DEPTH {
            return Err(DocumentError::Formatting(format!(
                "MathML nesting exceeds {}",
                super::MAX_MATH_DEPTH
            ))
            .into());
        }
        Err(MathError::Children(vec![Job {
            element,
            style: StyleSource::Parent(parent.clone()),
            depth,
            layout,
            root,
        }]))
    }
}
