use crate::{
    StyleError,
    conditions::{Condition, NamedValue, Truth, block_source},
    declaration::{CUSTOM_PROPERTY_PREFIX, supports_declaration},
    parser::{AtRuleName, balanced_components},
    selector::{Namespaces, supports_selectors_with_namespaces},
};
use cssparser::{Parser, ParserInput, Token};
use std::{cell::RefCell, collections::BTreeMap, rc::Rc};

pub(super) type SupportNames = Rc<RefCell<BTreeMap<String, NamedValue>>>;

#[derive(Clone)]
pub(super) enum SupportTest {
    Known(bool),
    Named(String),
    Selector(String, Namespaces),
}

impl SupportTest {
    pub(crate) fn declaration(source: &str) -> Option<Self> {
        let mut input = ParserInput::new(source);
        let input = &mut Parser::new(&mut input);
        let name = input.expect_ident_cloned().ok()?;
        if name.starts_with(CUSTOM_PROPERTY_PREFIX) && input.is_exhausted() {
            return Some(Self::Named(name.to_string()));
        }
        input.expect_colon().ok()?;
        Some(Self::Known(supports_declaration(
            &name,
            block_source(input),
        )))
    }

    fn parse(source: &str, namespaces: &Namespaces) -> Option<Self> {
        if let Some(test) = Self::declaration(source) {
            return Some(test);
        }
        let mut input = ParserInput::new(source);
        let input = &mut Parser::new(&mut input);
        let Token::Function(name) = input.next().ok()?.clone() else {
            return None;
        };
        let test = input.parse_nested_block(|nested| {
            if name.eq_ignore_ascii_case("selector") {
                return Ok(Self::Selector(block_source(nested).to_owned(), namespaces.clone()));
            }
            let supported = if name.eq_ignore_ascii_case("font-format") {
                nested.expect_ident_cloned().is_ok_and(|format| crate::values::supports_font_format(&format))
            } else if name.eq_ignore_ascii_case("font-tech") {
                nested.expect_ident_cloned().is_ok_and(|tech| crate::values::supports_font_technology(&tech))
            } else if name.eq_ignore_ascii_case("at-rule") {
                matches!(nested.next(), Ok(Token::AtKeyword(name)) if AtRuleName::parse(name).is_some() || crate::MarginLocation::parse(name).is_some())
            } else {
                block_source(nested);
                false
            };
            nested.expect_exhausted()?;
            Ok::<_, cssparser::ParseError<'_, ()>>(Self::Known(supported))
        }).ok()?;
        input.expect_exhausted().ok()?;
        Some(test)
    }

    pub(crate) fn evaluate(&self, names: &SupportNames) -> bool {
        self.evaluate_with(names, None)
    }

    fn evaluate_with(&self, names: &SupportNames, namespaces: Option<&Namespaces>) -> bool {
        match self {
            Self::Known(value) => *value,
            Self::Selector(source, original) => supports_selectors_with_namespaces(
                source,
                false,
                &namespaces.unwrap_or(original).borrow(),
            ),
            Self::Named(name) => names
                .borrow()
                .get(name)
                .and_then(|value| value.matching)
                .unwrap_or(false),
        }
    }

    pub(crate) fn name(&self) -> Option<&str> {
        match self {
            Self::Named(name) => Some(name),
            Self::Known(_) | Self::Selector(..) => None,
        }
    }
}

#[derive(Clone)]
pub(super) struct SupportsCondition {
    condition: Condition<SupportTest>,
    names: SupportNames,
}

impl SupportsCondition {
    pub(crate) fn parse_with_namespaces(
        source: &str,
        namespaces: &Namespaces,
    ) -> Result<Option<Self>, StyleError> {
        if !balanced_components(source)? {
            return Ok(None);
        }
        Ok(Condition::parse(source, true, Truth::False, |source| {
            SupportTest::parse(source, namespaces)
        })?
        .map(|condition| Self {
            condition,
            names: Default::default(),
        }))
    }

    pub(crate) fn matches_with(
        &self,
        names: &SupportNames,
        namespaces: Option<&Namespaces>,
    ) -> bool {
        self.condition
            .evaluate(|test| test.evaluate_with(names, namespaces).into())
            == Truth::True
    }

    pub(crate) fn bind(&mut self, names: &SupportNames) {
        self.names = names.clone();
    }
    pub(crate) fn dependencies(&self) -> impl Iterator<Item = &str> {
        self.condition.leaves().filter_map(SupportTest::name)
    }
    pub(crate) fn uses_paper(&self) -> bool {
        let names = self.names.borrow();
        self.dependencies()
            .any(|name| names.get(name).is_some_and(|value| value.paper_dependent))
    }
}

pub(super) fn matches_supports(condition: &SupportsCondition) -> bool {
    condition
        .condition
        .evaluate(|test| test.evaluate(&condition.names).into())
        == Truth::True
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conditions::MAX_CONDITION_DEPTH;
    #[test]
    fn supports_nesting_retains_the_common_typed_bound() {
        for depth in [MAX_CONDITION_DEPTH, MAX_CONDITION_DEPTH + 1] {
            let source = format!("{}width:12pt{}", "(".repeat(depth), ")".repeat(depth));
            let result = SupportsCondition::parse_with_namespaces(&source, &Namespaces::default());
            if depth == MAX_CONDITION_DEPTH {
                assert!(
                    result
                        .unwrap()
                        .is_some_and(|query| matches_supports(&query))
                );
            } else {
                assert!(matches!(result, Err(StyleError::GrammarLimit { .. })));
            }
        }
    }
}
