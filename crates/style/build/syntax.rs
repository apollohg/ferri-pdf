//! CSS Values 4 §2 Value Definition Syntax parser.
use std::fmt;

const INFINITY: char = '∞';
const UNBOUNDED_ONE_OR_MORE: (u32, Option<u32>) = (1, None);

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Bound {
    pub(crate) value: f64,
    pub(crate) unit: String,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Term {
    Keyword(String),
    Literal(char),
    /// `<name>`, `<name()>` or `<name [min,max]>`.
    Type {
        name: String,
        range: Option<(Bound, Bound)>,
    },
    /// `<'property'>`.
    Property(String),
    Function {
        name: String,
        body: Option<Box<Term>>,
    },
    /// A literal simple block: `( … )` or `'[' … ']'`.
    Block {
        open: char,
        body: Box<Term>,
    },
    Sequence(Vec<Term>),
    AllOf(Vec<Term>),
    AnyOf(Vec<Term>),
    OneOf(Vec<Term>),
    Repeat {
        term: Box<Term>,
        min: u32,
        max: Option<u32>,
        comma: bool,
    },
    /// `!`: the group must produce at least one value.
    NonEmpty(Box<Term>),
}

#[derive(Debug)]
pub(crate) struct SyntaxError {
    pub(crate) offset: usize,
    pub(crate) message: &'static str,
}
impl fmt::Display for SyntaxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} at character {}", self.message, self.offset)
    }
}

pub(crate) fn parse(text: &str) -> Result<Term, SyntaxError> {
    let mut parser = Parser {
        chars: text.chars().collect(),
        pos: 0,
    };
    let term = parser.one_of()?;
    parser.skip_whitespace();
    if parser.pos != parser.chars.len() {
        return Err(parser.error("unexpected character"));
    }
    Ok(term)
}

struct Parser {
    chars: Vec<char>,
    pos: usize,
}

impl Parser {
    fn error(&self, message: &'static str) -> SyntaxError {
        SyntaxError {
            offset: self.pos,
            message,
        }
    }
    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }
    fn peek_at(&self, offset: usize) -> Option<char> {
        self.chars.get(self.pos + offset).copied()
    }
    fn skip_whitespace(&mut self) {
        while self.peek().is_some_and(char::is_whitespace) {
            self.pos += 1;
        }
    }
    fn expect(&mut self, expected: char, message: &'static str) -> Result<(), SyntaxError> {
        if self.peek() == Some(expected) {
            self.pos += 1;
            Ok(())
        } else {
            Err(self.error(message))
        }
    }
    /// Consumes `operator` (`|`, `||` or `&&`) after optional whitespace.
    fn eat_operator(&mut self, operator: &str) -> bool {
        let start = self.pos;
        self.skip_whitespace();
        let chars: Vec<char> = operator.chars().collect();
        let matches = chars
            .iter()
            .enumerate()
            .all(|(index, c)| self.peek_at(index) == Some(*c))
            && (operator != "|" || self.peek_at(1) != Some('|'));
        if matches {
            self.pos += chars.len();
        } else {
            self.pos = start;
        }
        matches
    }

    fn one_of(&mut self) -> Result<Term, SyntaxError> {
        let mut terms = vec![self.any_of()?];
        while self.eat_operator("|") {
            terms.push(self.any_of()?);
        }
        Ok(combine(terms, Term::OneOf))
    }
    fn any_of(&mut self) -> Result<Term, SyntaxError> {
        let mut terms = vec![self.all_of()?];
        while self.eat_operator("||") {
            terms.push(self.all_of()?);
        }
        Ok(combine(terms, Term::AnyOf))
    }
    fn all_of(&mut self) -> Result<Term, SyntaxError> {
        let mut terms = vec![self.sequence()?];
        while self.eat_operator("&&") {
            terms.push(self.sequence()?);
        }
        Ok(combine(terms, Term::AllOf))
    }
    fn sequence(&mut self) -> Result<Term, SyntaxError> {
        let mut terms = Vec::new();
        loop {
            self.skip_whitespace();
            match self.peek() {
                None | Some('|' | ']' | ')') => break,
                Some('&') if self.peek_at(1) == Some('&') => break,
                Some('\'') if self.peek_at(1) == Some(']') && self.peek_at(2) == Some('\'') => {
                    break;
                }
                _ => terms.push(self.multiplied()?),
            }
        }
        if terms.is_empty() {
            return Err(self.error("empty group"));
        }
        Ok(combine(terms, Term::Sequence))
    }

    fn multiplied(&mut self) -> Result<Term, SyntaxError> {
        let mut term = self.primary()?;
        loop {
            let (min, max, comma) = match self.peek() {
                Some('*') => (0, None, false),
                Some('+') => (1, None, false),
                Some('?') => (0, Some(1), false),
                Some('#') => {
                    self.pos += 1;
                    let (min, max) = if self.peek() == Some('{') {
                        self.bounds()?
                    } else {
                        UNBOUNDED_ONE_OR_MORE
                    };
                    term = Term::Repeat {
                        term: Box::new(term),
                        min,
                        max,
                        comma: true,
                    };
                    continue;
                }
                Some('{') => {
                    let (min, max) = self.bounds()?;
                    term = Term::Repeat {
                        term: Box::new(term),
                        min,
                        max,
                        comma: false,
                    };
                    continue;
                }
                Some('!') => {
                    self.pos += 1;
                    term = Term::NonEmpty(Box::new(term));
                    continue;
                }
                _ => return Ok(term),
            };
            self.pos += 1;
            term = Term::Repeat {
                term: Box::new(term),
                min,
                max,
                comma,
            };
        }
    }

    /// `{A}`, `{A,}` or `{A,B}`.
    fn bounds(&mut self) -> Result<(u32, Option<u32>), SyntaxError> {
        self.expect('{', "expected {")?;
        let min = self.integer()?;
        let max = if self.peek() == Some(',') {
            self.pos += 1;
            if self.peek() == Some('}') {
                None
            } else {
                Some(self.integer()?)
            }
        } else {
            Some(min)
        };
        self.expect('}', "expected }")?;
        if max.is_some_and(|max| max < min) {
            return Err(self.error("multiplier maximum below minimum"));
        }
        Ok((min, max))
    }
    fn integer(&mut self) -> Result<u32, SyntaxError> {
        let start = self.pos;
        while self.peek().is_some_and(|c| c.is_ascii_digit()) {
            self.pos += 1;
        }
        self.chars[start..self.pos]
            .iter()
            .collect::<String>()
            .parse()
            .map_err(|_| self.error("expected integer"))
    }

    fn primary(&mut self) -> Result<Term, SyntaxError> {
        self.skip_whitespace();
        match self.peek() {
            Some('[') => {
                self.pos += 1;
                let term = self.one_of()?;
                self.skip_whitespace();
                self.expect(']', "expected ]")?;
                Ok(term)
            }
            Some('(') => {
                self.pos += 1;
                let body = self.one_of()?;
                self.skip_whitespace();
                self.expect(')', "expected )")?;
                Ok(Term::Block {
                    open: '(',
                    body: Box::new(body),
                })
            }
            Some('<') => self.angle_bracketed(),
            Some('\'') => self.quoted(),
            Some(c @ (',' | '/')) => {
                self.pos += 1;
                Ok(Term::Literal(c))
            }
            Some(c) if is_identifier(c) => self.keyword_or_function(),
            _ => Err(self.error("expected component")),
        }
    }

    fn quoted(&mut self) -> Result<Term, SyntaxError> {
        self.pos += 1;
        let literal = self
            .peek()
            .ok_or_else(|| self.error("unterminated literal"))?;
        self.pos += 1;
        self.expect('\'', "literals must be one character")?;
        if literal != '[' {
            return Ok(Term::Literal(literal));
        }
        let body = self.one_of()?;
        self.skip_whitespace();
        for expected in ['\'', ']', '\''] {
            self.expect(expected, "expected ']'")?;
        }
        Ok(Term::Block {
            open: '[',
            body: Box::new(body),
        })
    }

    fn keyword_or_function(&mut self) -> Result<Term, SyntaxError> {
        let name = self.identifier();
        if self.peek() != Some('(') {
            return Ok(Term::Keyword(name.to_ascii_lowercase()));
        }
        self.pos += 1;
        self.skip_whitespace();
        let body = if self.peek() == Some(')') {
            None
        } else {
            Some(Box::new(self.one_of()?))
        };
        self.skip_whitespace();
        self.expect(')', "expected ) after function arguments")?;
        Ok(Term::Function {
            name: name.to_ascii_lowercase(),
            body,
        })
    }

    fn identifier(&mut self) -> String {
        let start = self.pos;
        while self.peek().is_some_and(is_identifier) {
            self.pos += 1;
        }
        self.chars[start..self.pos].iter().collect()
    }

    fn angle_bracketed(&mut self) -> Result<Term, SyntaxError> {
        self.pos += 1;
        if self.peek() == Some('\'') {
            self.pos += 1;
            let name = self.identifier();
            self.expect('\'', "expected ' after property name")?;
            self.expect('>', "expected > after property reference")?;
            return Ok(Term::Property(name));
        }
        let mut name = self.identifier();
        if self.peek() == Some('(') && self.peek_at(1) == Some(')') {
            self.pos += 2;
            name.push_str("()");
        }
        self.skip_whitespace();
        let range = if self.peek() == Some('[') {
            self.pos += 1;
            let min = self.bound()?;
            self.expect(',', "expected , in range")?;
            let max = self.bound()?;
            self.expect(']', "expected ] after range")?;
            self.skip_whitespace();
            Some((min, max))
        } else {
            None
        };
        self.expect('>', "expected > after type")?;
        if name.is_empty() {
            return Err(self.error("empty type name"));
        }
        Ok(Term::Type { name, range })
    }

    fn bound(&mut self) -> Result<Bound, SyntaxError> {
        self.skip_whitespace();
        let negative = self.peek() == Some('-');
        if negative {
            self.pos += 1;
        }
        let value = if self.peek() == Some(INFINITY) {
            self.pos += 1;
            f64::INFINITY
        } else {
            let start = self.pos;
            while self.peek().is_some_and(|c| c.is_ascii_digit() || c == '.') {
                self.pos += 1;
            }
            self.chars[start..self.pos]
                .iter()
                .collect::<String>()
                .parse::<f64>()
                .map_err(|_| self.error("expected range bound"))?
        };
        let unit = self.identifier().to_ascii_lowercase();
        self.skip_whitespace();
        Ok(Bound {
            value: if negative { -value } else { value },
            unit,
        })
    }
}

fn is_identifier(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '-' || c == '_'
}

fn combine(mut terms: Vec<Term>, group: fn(Vec<Term>) -> Term) -> Term {
    if terms.len() == 1 {
        terms.remove(0)
    } else {
        group(terms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keyword(name: &str) -> Term {
        Term::Keyword(name.into())
    }
    fn ty(name: &str) -> Term {
        Term::Type {
            name: name.into(),
            range: None,
        }
    }

    #[test]
    fn precedence_is_juxtaposition_then_all_then_any_then_one() {
        assert_eq!(
            parse("a b && c || d | e").unwrap(),
            Term::OneOf(vec![
                Term::AnyOf(vec![
                    Term::AllOf(vec![
                        Term::Sequence(vec![keyword("a"), keyword("b")]),
                        keyword("c"),
                    ]),
                    keyword("d"),
                ]),
                keyword("e"),
            ])
        );
    }

    #[test]
    fn multipliers_stack_and_hash_takes_bounds() {
        assert_eq!(
            parse("<x>#{1,2}? <y>{2,}").unwrap(),
            Term::Sequence(vec![
                Term::Repeat {
                    term: Box::new(Term::Repeat {
                        term: Box::new(ty("x")),
                        min: 1,
                        max: Some(2),
                        comma: true,
                    }),
                    min: 0,
                    max: Some(1),
                    comma: false,
                },
                Term::Repeat {
                    term: Box::new(ty("y")),
                    min: 2,
                    max: None,
                    comma: false,
                },
            ])
        );
    }

    #[test]
    fn ranges_functions_properties_and_literal_blocks() {
        assert_eq!(
            parse("fit-content(<length [0,∞]>) <'margin-top'> '[' <i>* ']' <time [0s,∞]>").unwrap(),
            Term::Sequence(vec![
                Term::Function {
                    name: "fit-content".into(),
                    body: Some(Box::new(Term::Type {
                        name: "length".into(),
                        range: Some((
                            Bound {
                                value: 0.0,
                                unit: String::new()
                            },
                            Bound {
                                value: f64::INFINITY,
                                unit: String::new()
                            }
                        )),
                    })),
                },
                Term::Property("margin-top".into()),
                Term::Block {
                    open: '[',
                    body: Box::new(Term::Repeat {
                        term: Box::new(ty("i")),
                        min: 0,
                        max: None,
                        comma: false,
                    }),
                },
                Term::Type {
                    name: "time".into(),
                    range: Some((
                        Bound {
                            value: 0.0,
                            unit: "s".into()
                        },
                        Bound {
                            value: f64::INFINITY,
                            unit: String::new()
                        }
                    )),
                },
            ])
        );
    }

    #[test]
    fn malformed_grammar_reports_position() {
        for (text, message) in [
            ("a |", "empty group"),
            ("[ a", "expected ]"),
            ("<x", "expected > after type"),
            ("a{3,1}", "multiplier maximum below minimum"),
            ("'ab'", "literals must be one character"),
        ] {
            let error = parse(text).expect_err(text);
            assert_eq!(error.message, message, "{text}: {error}");
        }
    }
}
