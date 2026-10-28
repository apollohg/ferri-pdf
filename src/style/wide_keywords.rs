use cssparser::{Parser, ParserInput};

/// CSS Cascade 5 §7.3 CSS-wide keywords, valid as the entire value of any property.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WideKeyword {
    Initial,
    Inherit,
    Unset,
    Revert,
    RevertLayer,
}

/// What a CSS-wide keyword resolves to for one property.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Resolved {
    Initial,
    Inherit,
}

impl WideKeyword {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Initial => "initial",
            Self::Inherit => "inherit",
            Self::Unset => "unset",
            Self::Revert => "revert",
            Self::RevertLayer => "revert-layer",
        }
    }
    pub(crate) fn parse(value: &str) -> Option<Self> {
        let mut input = ParserInput::new(value);
        let mut parser = Parser::new(&mut input);
        let keyword = Self::parse_ident(parser.expect_ident().ok()?)?;
        parser.expect_exhausted().ok()?;
        Some(keyword)
    }
    pub(crate) fn parse_ident(ident: &str) -> Option<Self> {
        [
            ("initial", Self::Initial),
            ("inherit", Self::Inherit),
            ("unset", Self::Unset),
            ("revert", Self::Revert),
            ("revert-layer", Self::RevertLayer),
        ]
        .into_iter()
        .find(|(name, _)| ident.eq_ignore_ascii_case(name))
        .map(|(_, keyword)| keyword)
    }
    pub(crate) fn resolve(self, inherited: bool) -> Resolved {
        match self {
            Self::Initial => Resolved::Initial,
            Self::Inherit => Resolved::Inherit,
            Self::Unset if inherited => Resolved::Inherit,
            Self::Unset => Resolved::Initial,
            Self::Revert | Self::RevertLayer => Self::Unset.resolve(inherited),
        }
    }
}
