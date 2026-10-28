//! CSS Values 5's restricted syntax definitions, composed with the common value matcher.
use crate::{
    StyleError,
    grammar::{Base, Production},
    parser::MAX_COMPONENT_DEPTH,
};
use cssparser::{Parser, ParserInput, Token};

#[derive(Clone, Debug)]
pub(super) enum Syntax {
    Universal,
    Alternatives(Vec<Clause>),
}
#[derive(Clone, Debug)]
pub(super) struct Clause {
    pub atom: Atom,
    pub list: List,
}
#[derive(Clone, Debug)]
pub(super) enum Atom {
    Base(Base),
    Production(Production),
    Literal(String),
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum List {
    Once,
    Space,
    Comma,
}
impl Syntax {
    pub(super) fn parse(
        input: &mut Parser<'_, '_>,
        depth: usize,
    ) -> Result<Option<Self>, StyleError> {
        if depth >= MAX_COMPONENT_DEPTH {
            return Err(StyleError::GrammarLimit {
                resource: "syntax nesting",
                limit: MAX_COMPONENT_DEPTH,
            });
        }
        let parsed = input.parse_entirely(|input| {
            if let Ok(source) = input.try_parse(Parser::expect_string_cloned) {
                input.expect_exhausted()?;
                let mut raw = ParserInput::new(&source);
                return Self::parse(&mut Parser::new(&mut raw), depth + 1)
                    .map_err(|error| input.new_custom_error(error))?
                    .ok_or_else(|| {
                        input.new_custom_error(StyleError::Invalid("invalid syntax string".into()))
                    });
            }
            if input.try_parse(|input| input.expect_delim('*')).is_ok() {
                input.expect_exhausted()?;
                return Ok(Self::Universal);
            }
            let mut clauses = Vec::new();
            loop {
                let (atom, transform_list) = match input.next()?.clone() {
                    Token::Ident(name) => (Atom::Literal(name.to_string()), false),
                    Token::Delim('<') => {
                        let name = match input.next_including_whitespace()?.clone() {
                            Token::Ident(name) => name,
                            token => return Err(input.new_unexpected_token_error(token)),
                        };
                        match input.next_including_whitespace()?.clone() {
                            Token::Delim('>') => {}
                            token => return Err(input.new_unexpected_token_error(token)),
                        }
                        let (atom, list) = match name.to_ascii_lowercase().as_str() {
                            "angle" => (Atom::Base(Base::Angle), false),
                            "color" => (Atom::Base(Base::Color), false),
                            "custom-ident" => (Atom::Base(Base::CustomIdent), false),
                            "image" => (Atom::Production(Production::Image), false),
                            "integer" => (Atom::Base(Base::Integer), false),
                            "length" => (Atom::Base(Base::Length), false),
                            "length-percentage" => (Atom::Base(Base::LengthPercentage), false),
                            "number" => (Atom::Base(Base::Number), false),
                            "percentage" => (Atom::Base(Base::Percentage), false),
                            "resolution" => (Atom::Base(Base::Resolution), false),
                            "string" => (Atom::Base(Base::String), false),
                            "time" => (Atom::Base(Base::Time), false),
                            "url" => (Atom::Base(Base::Url), false),
                            "transform-function" => {
                                (Atom::Production(Production::TransformFunction), false)
                            }
                            "transform-list" => {
                                (Atom::Production(Production::TransformFunction), true)
                            }
                            _ => {
                                return Err(input.new_custom_error(StyleError::Invalid(
                                    "unknown syntax type".into(),
                                )));
                            }
                        };
                        (atom, list)
                    }
                    token => return Err(input.new_unexpected_token_error(token)),
                };
                let multiplier = input
                    .try_parse(|input| match input.next_including_whitespace()?.clone() {
                        Token::Delim('+') => Ok(List::Space),
                        Token::Delim('#') => Ok(List::Comma),
                        token => Err(input.new_unexpected_token_error::<StyleError>(token)),
                    })
                    .ok();
                if transform_list && multiplier.is_some() {
                    return Err(input.new_custom_error(StyleError::Invalid(
                        "transform-list multiplier".into(),
                    )));
                }
                clauses.push(Clause {
                    atom,
                    list: if transform_list {
                        List::Space
                    } else {
                        multiplier.unwrap_or(List::Once)
                    },
                });
                if input.is_exhausted() {
                    break;
                }
                input.expect_delim('|')?;
            }
            Ok(Self::Alternatives(clauses))
        });
        match parsed {
            Ok(value) => Ok(Some(value)),
            Err(error) => match error.kind {
                cssparser::ParseErrorKind::Custom(error @ StyleError::GrammarLimit { .. }) => {
                    Err(error)
                }
                _ => Ok(None),
            },
        }
    }
}
