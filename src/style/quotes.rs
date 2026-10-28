use crate::style::{StyleError, box_values::unsupported, component::Component, grammar::Keyword};
use std::sync::Arc;

use ferri_pdf_spec_data::quotes as data;

const OPENING: usize = 0;
const CLOSING: usize = 1;
const PAIR_LENGTH: usize = 2;
const AUTOMATIC_LEVELS: usize = 2;
const LANGUAGE_SUBTAG: usize = 0;
const SCRIPT_SUBTAG: usize = 1;
const AFTER_SCRIPT: usize = 2;
const SCRIPT_LENGTH: usize = 4;
const ROOT_LANGUAGE: &str = "root";

/// Computed quotation system; automatic systems retain their used content language.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub(crate) enum Quotes {
    Auto(Arc<str>),
    MatchParent(Arc<str>),
    None,
    Pairs(Arc<[[String; PAIR_LENGTH]]>),
}

impl Default for Quotes {
    fn default() -> Self {
        Self::Auto(Arc::from(""))
    }
}

impl Quotes {
    pub(crate) fn inherited(&self, language: &str) -> Self {
        match self {
            Self::Auto(_) => Self::Auto(language.into()),
            other => other.clone(),
        }
    }

    pub(crate) fn from_component(
        value: &Component,
        language: &str,
        parent: &Self,
    ) -> Result<Self, StyleError> {
        match value.keyword() {
            Some(Keyword::None) => Ok(Self::None),
            Some(Keyword::Auto) => Ok(Self::Auto(language.into())),
            Some(Keyword::MatchParent) => Ok(match parent {
                Self::Auto(language) => Self::MatchParent(language.clone()),
                other => other.clone(),
            }),
            _ => {
                let mut strings = Vec::new();
                collect_strings(value, &mut strings)?;
                if strings.is_empty() || !strings.len().is_multiple_of(PAIR_LENGTH) {
                    return Err(unsupported(value));
                }
                Ok(Self::Pairs(Arc::from(strings.as_chunks::<PAIR_LENGTH>().0)))
            }
        }
    }

    fn mark(&self, depth: usize, closing: bool) -> &str {
        let side = if closing { CLOSING } else { OPENING };
        match self {
            Self::None => "",
            Self::Pairs(pairs) => &pairs[depth.min(pairs.len() - 1)][side],
            Self::Auto(language) | Self::MatchParent(language) => {
                let pairs = automatic(language);
                pairs[depth.min(pairs.len() - 1)][side]
            }
        }
    }
}

fn collect_strings(value: &Component, output: &mut Vec<String>) -> Result<(), StyleError> {
    match value.unwrap() {
        Component::String(value) => output.push(value.to_string()),
        Component::Group(values) => {
            for value in values {
                collect_strings(value, output)?;
            }
        }
        _ => return Err(unsupported(value)),
    }
    Ok(())
}

fn automatic(language: &str) -> &'static [[&'static str; PAIR_LENGTH]; AUTOMATIC_LEVELS] {
    let mut language = language.to_ascii_lowercase();
    if let Some(extension) = language.split('-').skip(1).find(|subtag| subtag.len() == 1) {
        let start = language
            .find(&format!("-{extension}-"))
            .unwrap_or(language.len());
        language.truncate(start);
    }
    loop {
        if let Ok(index) = data::LOCALES.binary_search_by_key(&language.as_str(), |(tag, _)| *tag) {
            return &data::SYSTEMS[data::LOCALES[index].1];
        }
        let parts: Vec<_> = language.split('-').collect();
        if parts.len() > AFTER_SCRIPT && parts[SCRIPT_SUBTAG].len() == SCRIPT_LENGTH {
            let without_script = std::iter::once(parts[LANGUAGE_SUBTAG])
                .chain(parts[AFTER_SCRIPT..].iter().copied())
                .collect::<Vec<_>>()
                .join("-");
            if let Ok(index) =
                data::LOCALES.binary_search_by_key(&without_script.as_str(), |(tag, _)| *tag)
            {
                return &data::SYSTEMS[data::LOCALES[index].1];
            }
        }
        let Some(separator) = language.rfind('-') else {
            let index = data::LOCALES
                .binary_search_by_key(&ROOT_LANGUAGE, |(tag, _)| *tag)
                .expect("CLDR contains its root locale");
            return &data::SYSTEMS[data::LOCALES[index].1];
        };
        language.truncate(separator);
    }
}

/// Document-order quote depth; it is independent of element nesting and box layout.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct QuoteDepth(usize);

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub(crate) enum QuoteAction {
    Open,
    Close,
    NoOpen,
    NoClose,
}

impl QuoteDepth {
    pub(crate) fn apply<'a>(&mut self, action: QuoteAction, quotes: &'a Quotes) -> &'a str {
        match action {
            QuoteAction::Open | QuoteAction::NoOpen => {
                let depth = self.0;
                self.0 += 1;
                if action == QuoteAction::Open {
                    quotes.mark(depth, false)
                } else {
                    ""
                }
            }
            QuoteAction::Close | QuoteAction::NoClose if self.0 > 0 => {
                self.0 -= 1;
                if action == QuoteAction::Close {
                    quotes.mark(self.0, true)
                } else {
                    ""
                }
            }
            _ => "",
        }
    }
}
