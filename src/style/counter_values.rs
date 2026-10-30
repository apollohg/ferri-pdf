use crate::style::{
    StyleError, box_values::unsupported, component::Component, grammar::Keyword, units::Relative,
};
use std::sync::{Arc, LazyLock};

pub(crate) const LIST_ITEM_COUNTER: &str = "list-item";
/// The predefined counter footnote elements increment (CSS GCPM 3 §2.5).
pub(crate) const FOOTNOTE_COUNTER: &str = "footnote";
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
