use crate::{LayoutError, measure::Measurer};
use ferri_pdf_document::BlockBox;
use ferri_pdf_style::{LengthPercentage, Size};
use std::rc::Rc;

#[derive(Clone, Copy, Default)]
struct Extremes {
    positive: f32,
    negative: f32,
}
impl Extremes {
    fn add(&mut self, value: f32) {
        self.positive = self.positive.max(value);
        self.negative = self.negative.min(value);
    }
    fn merge(&mut self, other: Self) {
        self.add(other.positive);
        self.add(other.negative);
    }
    fn value(self) -> f32 {
        self.positive + self.negative
    }
}
/// Adjoining margins collapsed in tree order (CSS 2.1 §8.3.1).
#[derive(Clone, Default)]
pub(super) struct MarginStrut<'a> {
    all: Extremes,
    /// Margins from the first forced break point inside the strut onwards, which survive that
    /// break (CSS Fragmentation 3 §5.2).
    after_break: Option<Extremes>,
    source: Option<Rc<MarginSource<'a>>>,
    forced: bool,
    clearance_owner: Option<&'a BlockBox>,
}
#[derive(Clone)]
enum MarginSource<'a> {
    Edge(&'a BlockBox, usize),
    Merge(MarginStrut<'a>, MarginStrut<'a>),
    Within(&'a BlockBox, MarginStrut<'a>),
    Truncated(MarginStrut<'a>),
}
impl<'a> MarginSource<'a> {
    fn take_children(&mut self, pending: &mut Vec<Rc<Self>>) {
        match self {
            Self::Edge(..) => {}
            Self::Merge(earlier, later) => {
                pending.extend(earlier.source.take());
                pending.extend(later.source.take());
            }
            Self::Within(_, value) | Self::Truncated(value) => pending.extend(value.source.take()),
        }
    }
}
impl Drop for MarginSource<'_> {
    fn drop(&mut self) {
        let mut pending = Vec::new();
        self.take_children(&mut pending);
        while let Some(source) = pending.pop() {
            if let Ok(mut source) = Rc::try_unwrap(source) {
                source.take_children(&mut pending);
            }
        }
    }
}
impl<'a> MarginStrut<'a> {
    pub fn from(value: f32) -> Self {
        let mut all = Extremes::default();
        all.add(value);
        Self {
            all,
            after_break: None,
            source: None,
            forced: false,
            clearance_owner: None,
        }
    }
    pub fn is_empty(&self) -> bool {
        self.clearance_owner.is_none()
            && self.source.is_none()
            && self.after_break.is_none()
            && self.all.positive == 0.0
            && self.all.negative == 0.0
    }
    pub fn for_clearance(mut self, block: &'a BlockBox) -> Self {
        self.clearance_owner = Some(block);
        self
    }
    pub fn belongs_to(&self, block: &BlockBox) -> bool {
        self.clearance_owner
            .is_some_and(|owner| std::ptr::eq(owner, block))
    }
    pub fn edge(block: &'a BlockBox, edge: usize, width: f32) -> Self {
        let value = &block.style().margin[edge];
        let mut result = Self::from(value.resolve(width).unwrap_or(0.0));
        if matches!(value, Size::Value(value) if !matches!(value, LengthPercentage::Length(_))) {
            result.source = Some(Rc::new(MarginSource::Edge(block, edge)));
        }
        result
    }
    pub fn within(mut self, parent: &'a BlockBox) -> Self {
        if self.source.is_some() {
            self.source = Some(Rc::new(MarginSource::Within(parent, self.clone())));
            self.forced = false;
        }
        self
    }
    pub fn truncated(self) -> Self {
        let mut result = Self::from(self.after_break().unwrap_or(0.0));
        result.clearance_owner = self.clearance_owner;
        if self.source.is_some() {
            result.source = Some(Rc::new(MarginSource::Truncated(self)));
        }
        result
    }
    pub fn resolve(&self, width: f32, measurer: &Measurer<'_>) -> Result<Self, LayoutError> {
        if self.source.is_none() {
            return Ok(self.clone());
        }
        enum Step<'s, 'a> {
            Visit(&'s MarginStrut<'a>, f32),
            Merge,
            Truncate,
            Force,
        }
        let mut steps = vec![Step::Visit(self, width)];
        let mut values: Vec<MarginStrut<'a>> = Vec::new();
        while let Some(step) = steps.pop() {
            measurer.work.spend(1)?;
            match step {
                Step::Visit(strut, width) => {
                    if strut.forced {
                        steps.push(Step::Force);
                    }
                    match strut.source.as_deref() {
                        None => values.push(Self {
                            source: None,
                            ..strut.clone()
                        }),
                        Some(MarginSource::Edge(block, edge)) => values.push(Self::from(
                            block.style().margin[*edge].resolve(width).unwrap_or(0.0),
                        )),
                        Some(MarginSource::Merge(earlier, later)) => {
                            steps.push(Step::Merge);
                            steps.push(Step::Visit(later, width));
                            steps.push(Step::Visit(earlier, width));
                        }
                        Some(MarginSource::Within(parent, value)) => {
                            let geometry = crate::measure::block_geometry(
                                measurer, parent, 0.0, width, false, None,
                            )?;
                            steps.push(Step::Visit(value, geometry.width));
                        }
                        Some(MarginSource::Truncated(value)) => {
                            steps.push(Step::Truncate);
                            steps.push(Step::Visit(value, width));
                        }
                    }
                }
                Step::Merge => {
                    let later = values.pop().expect("later margin was resolved");
                    values
                        .last_mut()
                        .expect("earlier margin was resolved")
                        .merge(later);
                }
                Step::Truncate => {
                    let value = values.pop().expect("truncated margin was resolved");
                    values.push(value.truncated());
                }
                Step::Force => {
                    let value = values.pop().expect("forced margin was resolved");
                    values.push(value.after_forced_break());
                }
            }
        }
        let mut resolved = values.pop().expect("one margin strut was resolved");
        resolved.source = self.source.clone();
        resolved.forced = self.forced;
        resolved.clearance_owner = self.clearance_owner;
        Ok(resolved)
    }
    /// Merges `later`, which follows these margins in tree order.
    pub fn merge(&mut self, later: Self) {
        if later.is_empty() {
            return;
        }
        if self.is_empty() {
            *self = later;
            return;
        }
        let source = (self.source.is_some() || later.source.is_some())
            .then(|| Rc::new(MarginSource::Merge(self.clone(), later.clone())));
        self.all.merge(later.all);
        self.after_break = match self.after_break {
            Some(mut after) => {
                after.merge(later.all);
                Some(after)
            }
            None => later.after_break,
        };
        self.source = source;
        self.forced = false;
    }
    /// These margins with a forced break point before them.
    pub fn after_forced_break(self) -> Self {
        Self {
            after_break: Some(self.all),
            forced: true,
            ..self
        }
    }
    pub fn value(&self) -> f32 {
        self.all.value()
    }
    /// Collapsed margins that follow the strut's forced break point, if it has one.
    pub fn after_break(&self) -> Option<f32> {
        self.after_break.map(Extremes::value)
    }
}
