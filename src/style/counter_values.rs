use crate::style::{
    StyleError, box_values::unsupported, component::Component, grammar::Keyword, units::Relative,
};
use std::sync::{Arc, LazyLock};

pub(crate) const LIST_ITEM_COUNTER: &str = "list-item";
/// The predefined counter footnote elements increment (CSS GCPM 3 §2.5).
pub(crate) const FOOTNOTE_COUNTER: &str = "footnote";
/// The counter of each page box's number (CSS Page 3 §6.1).
pub(crate) const PAGE_COUNTER: &str = "page";
/// The document's page total, which counter properties cannot change (CSS Page 3 §6.1).
pub(crate) const PAGES_COUNTER: &str = "pages";
pub(crate) const INITIAL_COUNTER_VALUE: i128 = 0;
pub(crate) const DEFAULT_COUNTER_INCREMENT: i128 = 1;

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub(crate) struct CounterChange {
    pub name: Arc<str>,
    pub value: i128,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub(crate) struct CounterReset {
    pub name: Arc<str>,
    pub value: Option<i128>,
    pub reversed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub(crate) struct CounterProperties {
    pub reset: Arc<[CounterReset]>,
    pub increment: Arc<[CounterChange]>,
    pub set: Arc<[CounterChange]>,
}

static EMPTY_COUNTER_RESETS: LazyLock<Arc<[CounterReset]>> = LazyLock::new(Arc::default);
static EMPTY_COUNTER_CHANGES: LazyLock<Arc<[CounterChange]>> = LazyLock::new(Arc::default);
impl Default for CounterProperties {
    fn default() -> Self {
        Self {
            reset: EMPTY_COUNTER_RESETS.clone(),
            increment: EMPTY_COUNTER_CHANGES.clone(),
            set: EMPTY_COUNTER_CHANGES.clone(),
        }
    }
}
impl CounterProperties {
    pub(crate) fn is_empty(&self) -> bool {
        self.reset.is_empty() && self.increment.is_empty() && self.set.is_empty()
    }
    /// The changes these properties make to the counter `name`.
    #[must_use]
    pub(crate) fn named(&self, name: &str) -> Self {
        Self {
            reset: self
                .reset
                .iter()
                .filter(|change| change.name.as_ref() == name)
                .cloned()
                .collect(),
            increment: self
                .increment
                .iter()
                .filter(|change| change.name.as_ref() == name)
                .cloned()
                .collect(),
            set: self
                .set
                .iter()
                .filter(|change| change.name.as_ref() == name)
                .cloned()
                .collect(),
        }
    }
    /// The properties with `name` reset to its initial value unless they reset it already.
    #[must_use]
    pub(crate) fn with_implicit_reset(&self, name: &str) -> Self {
        if self.reset.iter().any(|change| change.name.as_ref() == name) {
            return self.clone();
        }
        Self {
            reset: self
                .reset
                .iter()
                .cloned()
                .chain([CounterReset {
                    name: Arc::from(name),
                    value: Some(INITIAL_COUNTER_VALUE),
                    reversed: false,
                }])
                .collect(),
            ..self.clone()
        }
    }
    /// The properties with `name` incremented by one unless they increment it already.
    #[must_use]
    pub(crate) fn with_implicit_increment(&self, name: &str) -> Self {
        if self
            .increment
            .iter()
            .any(|change| change.name.as_ref() == name)
        {
            return self.clone();
        }
        Self {
            increment: self
                .increment
                .iter()
                .cloned()
                .chain([CounterChange {
                    name: Arc::from(name),
                    value: DEFAULT_COUNTER_INCREMENT,
                }])
                .collect(),
            ..self.clone()
        }
    }
}

/// The initial value of a reversed counter instantiated without one, accumulated from the
/// increments and sets in its scope (CSS Lists 3 §4.4.2).
#[derive(Default)]
pub(crate) struct AutomaticInitial {
    sum: i128,
    last_increment_negated: i128,
    set: Option<i128>,
}
impl AutomaticInitial {
    pub(crate) fn update(&mut self, increment: i128, set: Option<i128>) {
        if self.set.is_some() {
            return;
        }
        let negated = increment.saturating_neg();
        if negated != 0 {
            self.last_increment_negated = negated;
        }
        if let Some(value) = set {
            self.set = Some(value);
        } else {
            self.sum = self.sum.saturating_add(negated);
        }
    }
    pub(crate) fn value(&self) -> i128 {
        // Cancel the bounded CSS increments before adding a potentially maximal HTML value.
        self.set
            .unwrap_or(INITIAL_COUNTER_VALUE)
            .saturating_add(self.sum.saturating_add(self.last_increment_negated))
    }
}

fn flatten<'a>(value: &'a Component, output: &mut Vec<&'a Component>) {
    match value.unwrap() {
        Component::Group(items) => items.iter().for_each(|item| flatten(item, output)),
        item => output.push(item),
    }
}

pub(crate) fn names(value: &Component) -> Vec<Arc<str>> {
    let mut parts = Vec::new();
    flatten(value, &mut parts);
    parts
        .into_iter()
        .filter_map(|item| match item {
            Component::Ident(name) => Some(Arc::from(name.as_ref())),
            Component::Function(Keyword::Reversed, Some(argument)) => match argument.unwrap() {
                Component::Ident(name) => Some(Arc::from(name.as_ref())),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

#[derive(Default)]
pub(crate) struct CounterScopes {
    pub reset: Vec<Arc<str>>,
    pub updates: Vec<Arc<str>>,
}

fn entries(
    value: &Component,
    relative: &Relative<'_>,
    default: i128,
    allow_reversed: bool,
) -> Result<Vec<CounterReset>, StyleError> {
    let mut parts = Vec::new();
    flatten(value, &mut parts);
    let mut parts = parts.into_iter().peekable();
    let mut changes = Vec::new();
    while let Some(item) = parts.next() {
        let (name, reversed) = match item {
            Component::Ident(name) => (name, false),
            Component::Function(Keyword::Reversed, Some(argument)) if allow_reversed => {
                let Component::Ident(name) = argument.unwrap() else {
                    return Err(unsupported(item));
                };
                (name, true)
            }
            _ => return Err(unsupported(item)),
        };
        let value = if parts.peek().is_some_and(|next| {
            !matches!(
                next,
                Component::Ident(_) | Component::Function(Keyword::Reversed, _)
            )
        }) {
            Some(match parts.next().unwrap() {
                Component::HtmlInteger(value) => *value,
                value => crate::style::values::integer(value, relative)?.into(),
            })
        } else if reversed {
            None
        } else {
            Some(default)
        };
        changes.push(CounterReset {
            name: Arc::from(name.as_ref()),
            value,
            reversed,
        });
    }
    Ok(changes)
}

pub(crate) fn resets(
    value: &Component,
    relative: &Relative<'_>,
) -> Result<Arc<[CounterReset]>, StyleError> {
    if value.keyword() == Some(Keyword::None) {
        return Ok(EMPTY_COUNTER_RESETS.clone());
    }
    Ok(entries(value, relative, INITIAL_COUNTER_VALUE, true)?.into())
}

pub(crate) fn changes(
    value: &Component,
    relative: &Relative<'_>,
    default: i128,
) -> Result<Arc<[CounterChange]>, StyleError> {
    if value.keyword() == Some(Keyword::None) {
        return Ok(EMPTY_COUNTER_CHANGES.clone());
    }
    Ok(entries(value, relative, default, false)?
        .into_iter()
        .map(|entry| CounterChange {
            name: entry.name,
            value: entry.value.expect("ordinary counter updates have a value"),
        })
        .collect())
}
