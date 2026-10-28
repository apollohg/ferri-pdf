//! Declaration grammar validation, whole-value selection, and shorthand expansion.
use crate::style::{
    StyleError,
    cascade::Origin,
    component::Component,
    grammar::{Descriptor, Initial, PROPERTIES, Property},
    matcher::{ValueContext, match_value},
    parser::Declaration,
    variables,
    wide_keywords::WideKeyword,
};
use cssparser::{DeclarationParser, Delimiter, ParseErrorKind, Parser, ParserInput, Token};
use std::{
    rc::Rc,
    sync::{Arc, OnceLock},
};

pub(super) fn supports_declaration(name: &str, value: &str) -> bool {
    let mut name_input = ParserInput::new(name);
    let name_parser = &mut Parser::new(&mut name_input);
    let Ok(name) = name_parser.expect_ident_cloned() else {
        return false;
    };
    if name_parser.expect_exhausted().is_err()
        || !crate::style::parser::balanced_components(value).unwrap_or(false)
    {
        return false;
    }
    let base = crate::resources::ResourceBase::default();
    let mut value_input = ParserInput::new(value);
    let input = &mut Parser::new(&mut value_input);
    let state = input.state();
    let Ok(declaration) = input.parse_entirely(|input| {
        input.parse_until_before(Delimiter::Semicolon, |input| {
            crate::style::parser::Declarations { base: &base }.parse_value(name, input, &state)
        })
    }) else {
        return false;
    };
    let substitutions = if declaration.name.starts_with(CUSTOM_PROPERTY_PREFIX) {
        variables::Value::with_substitutions(&declaration.value)
    } else {
        variables::Value::with_property_substitutions(&declaration.value)
    };
    let Ok(variables) = substitutions else {
        return false;
    };
    if declaration.name.starts_with(CUSTOM_PROPERTY_PREFIX) {
        return true;
    }
    let Some(property) = Property::from_name(&declaration.name) else {
        return false;
    };
    if !longhands(property).any(crate::style::properties::supports_property) {
        return false;
    }
    if variables.is_some() {
        return true;
    }
    let Ok(Some(expanded)) = expand(property, &declaration.value) else {
        return false;
    };
    supported(&expanded)
}

fn supported(expanded: &[(Property, Specified)]) -> bool {
    let relative = crate::style::units::Relative::validation();
    expanded
        .iter()
        .all(|(longhand, specified)| match specified {
            Specified::Wide(_) => true,
            Specified::Value(value) if crate::style::properties::supports_property(*longhand) => {
                crate::style::properties::supports_value(*longhand, value, &relative)
            }
            Specified::Value(value) => initial(*longhand) == Some(value.as_ref()),
            Specified::Toggle(values) => values
                .iter()
                .all(|value| supported(&[(*longhand, value.value.clone())])),
            Specified::Unexpanded(..) => false,
            Specified::FlowRelative(_, value) => supported(&[(*longhand, value.as_ref().clone())]),
        })
}

pub(super) const CUSTOM_PROPERTY_PREFIX: &str = "--";

#[derive(Clone, Debug)]
pub(crate) enum Specified {
    Value(Arc<Component>),
    Wide(WideKeyword),
    Toggle(Arc<[ToggleArgument]>),
    /// A valid value of this shorthand that the engine cannot split into longhands.
    Unexpanded(Property, Arc<Component>),
    /// A flow-relative declaration that won a physical property whose counterpart on the
    /// parent is another physical property: its parent-dependent value reads the parent with
    /// its box properties arranged for this element's flow (CSS Logical 1 §4).
    FlowRelative(crate::style::logical::Flow, Arc<Specified>),
}

#[derive(Clone, Debug)]
pub(crate) struct ToggleArgument {
    pub(crate) value: Specified,
    pub(crate) font_namespace: Option<crate::fonts::FontNamespace>,
    pub(crate) location: Option<Arc<crate::style::SourceLocation>>,
}

/// A declaration matched after arbitrary value substitution.
#[derive(Debug)]
pub(crate) struct Pending {
    pub(crate) property: Property,
    pub(crate) value: variables::Value,
    pub(crate) location: crate::style::SourceLocation,
    pub(crate) namespaces: Option<crate::style::selector::Namespaces>,
}

#[derive(Clone, Debug)]
pub(crate) enum Declared {
    Longhand(Property, Specified),
    Pending(Property, Rc<Pending>),
    Custom(Arc<str>, Arc<str>, bool),
}

#[derive(Clone, Debug)]
pub(crate) struct PropertyDeclaration {
    pub(crate) declared: Declared,
    pub(crate) location: Option<Arc<crate::style::SourceLocation>>,
    pub(crate) important: bool,
    pub(crate) origin: Origin,
    pub(crate) layer: crate::style::layers::LayerId,
    pub(crate) font_namespace: crate::fonts::FontNamespace,
    pub(crate) namespaces: Option<crate::style::selector::Namespaces>,
}

/// Appends one entry per longhand. Unknown properties and invalid values are dropped, as
/// CSS Syntax 3 requires.
pub(crate) fn parse(
    declaration: &Declaration,
    output: &mut Vec<PropertyDeclaration>,
) -> Result<(), StyleError> {
    parse_at_origin(declaration, Origin::Author, output)
}

pub(crate) fn parse_at_origin(
    declaration: &Declaration,
    origin: Origin,
    output: &mut Vec<PropertyDeclaration>,
) -> Result<(), StyleError> {
    let important = declaration.important;
    let location = Some(Arc::new(declaration.location.clone()));
    let mut push = |declared| {
        output.push(PropertyDeclaration {
            declared,
            location: location.clone(),
            important,
            origin,
            layer: Default::default(),
            font_namespace: crate::fonts::FontNamespace::ROOT,
            namespaces: declaration.namespaces.clone(),
        });
    };
    if declaration.name.starts_with(CUSTOM_PROPERTY_PREFIX) {
        let contextual = variables::Value::with_substitutions(&declaration.value)?
            .is_some_and(|value| value.requires_context());
        push(Declared::Custom(
            declaration.name.as_str().into(),
            declaration.value.as_str().into(),
            contextual,
        ));
        return Ok(());
    }
    let Some(property) = Property::from_name(&declaration.name) else {
        return Ok(());
    };
    if let Some(value) = variables::Value::with_property_substitutions(&declaration.value)? {
        let pending = Rc::new(Pending {
            property,
            value,
            location: declaration.location.clone(),
            namespaces: declaration.namespaces.clone(),
        });
        for longhand in longhands(property) {
            push(Declared::Pending(longhand, pending.clone()));
        }
        return Ok(());
    }
    for (longhand, specified) in expand(property, &declaration.value)?.unwrap_or_default() {
        push(Declared::Longhand(longhand, specified));
    }
    Ok(())
}

/// The longhand values of `property: source`, or `None` if the value is invalid.
pub(crate) fn expand(
    property: Property,
    source: &str,
) -> Result<Option<Vec<(Property, Specified)>>, StyleError> {
    expand_with_context(property, source, ValueContext::Element)
}
fn expand_with_context(
    property: Property,
    source: &str,
    context: ValueContext<'_>,
) -> Result<Option<Vec<(Property, Specified)>>, StyleError> {
    let steps = &mut 0;
    match select(Target::Property(property), source, steps, 0)? {
        Selection::Source(source) => expand_plain(property, source, steps, context),
        Selection::Invalid => Ok(None),
        Selection::ComputedInvalid => Ok(Some(wide(property, WideKeyword::Unset))),
    }
}

fn expand_plain(
    property: Property,
    source: &str,
    steps: &mut usize,
    context: ValueContext<'_>,
) -> Result<Option<Vec<(Property, Specified)>>, StyleError> {
    let mut input = ParserInput::new(source);
    let input = &mut Parser::new(&mut input);
    if matches!(input.next(), Ok(Token::Function(name)) if name.eq_ignore_ascii_case(TOGGLE)) {
        return toggle(property, input, steps, context);
    }
    if let Some(keyword) = WideKeyword::parse(source) {
        return Ok(Some(wide(property, keyword)));
    }
    let Some(value) = value_with_context(property, source, steps, context)? else {
        return Ok(None);
    };
    Ok(Some(if property.longhands().is_empty() {
        vec![(property, Specified::Value(Arc::new(value)))]
    } else {
        crate::style::shorthands::expand(property, value)?
    }))
}

const FIRST_VALID: &str = "first-valid";
pub(super) const TOGGLE: &str = "toggle";
const TOGGLE_FORBIDDEN: &[&str] = &[TOGGLE, "attr", "calc"];

fn toggle(
    property: Property,
    input: &mut Parser<'_, '_>,
    steps: &mut usize,
    context: ValueContext<'_>,
) -> Result<Option<Vec<(Property, Specified)>>, StyleError> {
    let candidates = input.parse_nested_block(|input| {
        input.parse_comma_separated(|input| {
            crate::style::parser::declaration_argument(input, false)?.ok_or_else(|| {
                input.new_custom_error(StyleError::Invalid("empty toggle() argument".into()))
            })
        })
    });
    let candidates = match candidates {
        Ok(candidates) if input.expect_exhausted().is_ok() => candidates,
        Err(error) => {
            if let ParseErrorKind::Custom(error @ StyleError::GrammarLimit { .. }) = error.kind {
                return Err(error);
            }
            return Ok(None);
        }
        _ => return Ok(None),
    };
    let mut values: Vec<(Property, Vec<Specified>)> = longhands(property)
        .map(|property| (property, Vec::new()))
        .collect();
    for candidate in candidates {
        let mut input = ParserInput::new(candidate);
        if !toggle_argument(&mut Parser::new(&mut input), 0)? {
            return Ok(None);
        }
        let candidate = match select(Target::Property(property), candidate, steps, 0)? {
            Selection::Source(source) => expand_plain(property, source, steps, context)?,
            Selection::Invalid => return Ok(None),
            Selection::ComputedInvalid => Some(wide(property, WideKeyword::Unset)),
        };
        let Some(candidate) = candidate else {
            return Ok(None);
        };
        for (property, value) in &mut values {
            let Some((_, argument)) = candidate.iter().find(|(longhand, _)| longhand == property)
            else {
                return Ok(None);
            };
            value.push(argument.clone());
        }
    }
    Ok(Some(
        values
            .into_iter()
            .map(|(property, values)| {
                (
                    property,
                    Specified::Toggle(
                        values
                            .into_iter()
                            .map(|value| ToggleArgument {
                                value,
                                font_namespace: None,
                                location: None,
                            })
                            .collect(),
                    ),
                )
            })
            .collect(),
    ))
}

pub(super) fn toggle_argument(
    input: &mut Parser<'_, '_>,
    depth: usize,
) -> Result<bool, StyleError> {
    if depth >= crate::style::parser::MAX_COMPONENT_DEPTH {
        return Err(StyleError::GrammarLimit {
            resource: "whole-value nesting",
            limit: crate::style::parser::MAX_COMPONENT_DEPTH,
        });
    }
    while !input.is_exhausted() {
        let token = input
            .next()
            .map_err(|error| StyleError::Invalid(format!("{error:?}")))?;
        match token {
            Token::Function(name)
                if TOGGLE_FORBIDDEN
                    .iter()
                    .any(|function| name.eq_ignore_ascii_case(function)) =>
            {
                return Ok(false);
            }
            Token::Function(_)
            | Token::ParenthesisBlock
            | Token::SquareBracketBlock
            | Token::CurlyBracketBlock => {
                let valid = input
                    .parse_nested_block(|input| {
                        toggle_argument(input, depth + 1)
                            .map_err(|error| input.new_custom_error(error))
                    })
                    .map_err(crate::style::parser::failure)?;
                if !valid {
                    return Ok(false);
                }
            }
            _ => {}
        }
    }
    Ok(true)
}

#[derive(Clone, Copy)]
enum Target {
    Property(Property),
    Page(Descriptor),
}

enum Selection<'a> {
    Source(&'a str),
    Invalid,
    ComputedInvalid,
}

fn select<'a>(
    target: Target,
    source: &'a str,
    steps: &mut usize,
    depth: usize,
) -> Result<Selection<'a>, StyleError> {
    let mut input = ParserInput::new(source);
    let input = &mut Parser::new(&mut input);
    if !matches!(input.next(), Ok(Token::Function(name)) if name.eq_ignore_ascii_case(FIRST_VALID))
    {
        return Ok(Selection::Source(source));
    }
    if depth >= crate::style::parser::MAX_COMPONENT_DEPTH {
        return Err(StyleError::GrammarLimit {
            resource: "whole-value nesting",
            limit: crate::style::parser::MAX_COMPONENT_DEPTH,
        });
    }
    let candidates = input.parse_nested_block(|input| {
        input.parse_comma_separated(|input| {
            crate::style::parser::declaration_argument(input, false)?.ok_or_else(|| {
                input.new_custom_error(StyleError::Invalid("empty first-valid() argument".into()))
            })
        })
    });
    let candidates = match candidates {
        Ok(candidates) if input.expect_exhausted().is_ok() => candidates,
        Err(error) => {
            if let ParseErrorKind::Custom(error @ StyleError::GrammarLimit { .. }) = error.kind {
                return Err(error);
            }
            return Ok(Selection::Invalid);
        }
        _ => return Ok(Selection::Invalid),
    };
    for candidate in candidates {
        match select(target, candidate, steps, depth + 1)? {
            Selection::ComputedInvalid => return Ok(Selection::ComputedInvalid),
            Selection::Invalid => {}
            Selection::Source(candidate) => {
                let accepted = match target {
                    Target::Property(property) => {
                        expand_plain(property, candidate, steps, ValueContext::Element)?
                            .is_some_and(|value| supported(&value))
                    }
                    Target::Page(descriptor) => {
                        WideKeyword::parse(candidate).is_some()
                            || match_value(
                                descriptor.data().root,
                                candidate,
                                steps,
                                ValueContext::Element,
                            )?
                            .is_some_and(|value| {
                                crate::style::stylesheet::supports_page_value(descriptor, &value)
                            })
                    }
                };
                if accepted {
                    return Ok(Selection::Source(candidate));
                }
            }
        }
    }
    Ok(Selection::ComputedInvalid)
}

/// Page-context properties share the ordinary whole-value selection path.
pub(super) fn page_value(
    descriptor: Descriptor,
    source: &str,
) -> Result<Option<Specified>, StyleError> {
    let steps = &mut 0;
    let source = match select(Target::Page(descriptor), source, steps, 0)? {
        Selection::Source(source) => source,
        Selection::Invalid => return Ok(None),
        Selection::ComputedInvalid => return Ok(Some(Specified::Wide(WideKeyword::Unset))),
    };
    if let Some(keyword) = WideKeyword::parse(source) {
        return Ok(Some(Specified::Wide(keyword)));
    }
    Ok(
        match_value(descriptor.data().root, source, steps, ValueContext::Element)?
            .map(|value| Specified::Value(Arc::new(value))),
    )
}

pub(crate) fn wide(property: Property, keyword: WideKeyword) -> Vec<(Property, Specified)> {
    longhands(property)
        .map(|longhand| (longhand, Specified::Wide(keyword)))
        .collect()
}

pub(crate) fn append(
    output: &mut Vec<PropertyDeclaration>,
    origin: Origin,
    property: Property,
    value: &str,
) -> Result<(), StyleError> {
    for (property, value) in expand(property, value)?.unwrap_or_default() {
        append_longhand(output, origin, property, value);
    }
    Ok(())
}

pub(crate) fn append_longhand(
    output: &mut Vec<PropertyDeclaration>,
    origin: Origin,
    property: Property,
    value: Specified,
) {
    output.push(PropertyDeclaration {
        declared: Declared::Longhand(property, value),
        location: None,
        important: false,
        origin,
        layer: Default::default(),
        font_namespace: crate::fonts::FontNamespace::ROOT,
        namespaces: None,
    });
}

/// Ordinary property grammar; global keywords and whole-value functions use `expand`.
pub(crate) fn value(
    property: Property,
    source: &str,
    steps: &mut usize,
) -> Result<Option<Component>, StyleError> {
    value_with_context(property, source, steps, ValueContext::Element)
}
fn value_with_context(
    property: Property,
    source: &str,
    steps: &mut usize,
    context: ValueContext<'_>,
) -> Result<Option<Component>, StyleError> {
    Ok(
        match_value(property.data().root, source, steps, context)?.filter(|value| {
            crate::style::grid_values::valid(property, value)
                && crate::style::inert::valid(property, value)
                && crate::style::font_values::valid(property, value)
        }),
    )
}

pub(crate) fn descriptor(
    descriptor: Descriptor,
    source: &str,
) -> Result<Option<Component>, StyleError> {
    if descriptor == Descriptor::FontFaceSrc {
        crate::style::matcher::match_comma_list(descriptor.data().root, source)
    } else {
        Ok(match_value(
            descriptor.data().root,
            source,
            &mut 0,
            ValueContext::Independent,
        )?
        .filter(|value| crate::style::font_values::valid_descriptor(descriptor, value)))
    }
}

/// The final longhands of a property; a longhand is its own.
pub(crate) fn longhands(property: Property) -> impl Iterator<Item = Property> {
    let mut stack = if property == Property::All {
        let mut properties = Property::ALL
            .iter()
            .copied()
            .rev()
            .filter(|property| {
                !matches!(
                    property,
                    Property::All | Property::Direction | Property::UnicodeBidi
                ) && property.longhands().is_empty()
            })
            .collect::<Vec<_>>();
        // CSS Logical 1 §4: shorthands covering both mappings set physical longhands last.
        properties.sort_by_key(|property| crate::style::logical::is_flow_relative(*property));
        properties
    } else {
        vec![property]
    };
    std::iter::from_fn(move || {
        while let Some(next) = stack.pop() {
            let nested = next.longhands();
            if nested.is_empty() {
                return Some(next);
            }
            stack.extend(nested.iter().rev());
        }
        None
    })
}

/// A property's initial value from the specification data, or `None` when the data gives
/// prose, such as "depends on user agent".
pub(crate) fn initial(property: Property) -> Option<&'static Component> {
    static INITIAL: [OnceLock<Option<Component>>; PROPERTIES.len()] =
        [const { OnceLock::new() }; PROPERTIES.len()];
    INITIAL[property as usize]
        .get_or_init(|| match property.data().initial {
            Initial::Value(source) => value(property, source, &mut 0).ok().flatten(),
            Initial::Prose | Initial::Longhands(_) => None,
        })
        .as_ref()
}

pub(crate) fn substitute(
    pending: &Pending,
    environment: &variables::Environment,
    inherited: &variables::Environment,
) -> Result<Option<Vec<(Property, Specified)>>, StyleError> {
    let Some(value) = pending.value.substitute(environment, inherited)? else {
        return Ok(None);
    };
    expand_substitution(pending, &value)
}

pub(super) fn expand_expanded(
    property: Property,
    value: &variables::Expanded,
) -> Result<Option<Vec<(Property, Specified)>>, StyleError> {
    let origin = crate::style::matcher::Origins::new(value);
    let result = expand_with_context(property, value.as_str(), ValueContext::Substituted(&origin))?;
    if origin.invalid_url() {
        Ok(None)
    } else {
        Ok(result)
    }
}

pub(super) fn expand_substitution(
    pending: &Pending,
    value: &variables::Expanded,
) -> Result<Option<Vec<(Property, Specified)>>, StyleError> {
    if value.has_taint() && expand_expanded(pending.property, value)?.is_none() {
        return Ok(None);
    }
    let mut declaration = Declaration {
        name: pending.property.name().to_owned(),
        value: value.as_str().to_owned(),
        important: false,
        location: pending.location.clone(),
        namespaces: pending.namespaces.clone(),
    };
    crate::style::parser::absolute_urls(&mut declaration, &pending.location.base)?;
    expand(pending.property, &declaration.value)
}

#[cfg(test)]
mod tests {
    use super::supports_declaration;

    #[test]
    fn recovering_font_sources_keep_one_work_budget_for_the_entire_list() {
        const SOURCE_ITEMS: usize = 50_000;
        let source = std::iter::repeat_n(
            "url(font.ttf) format(truetype) tech(features-opentype)",
            SOURCE_ITEMS,
        )
        .collect::<Vec<_>>()
        .join(",");
        let result = super::descriptor(crate::style::grammar::Descriptor::FontFaceSrc, &source);
        assert!(
            matches!(result, Err(crate::style::StyleError::GrammarLimit { .. })),
            "a long source list must retain the common matching limit"
        );
    }

    #[test]
    fn supports_uses_value_capabilities_for_longhands_shorthands_and_variables() {
        for (name, value, expected) in [
            ("display", "block", true),
            ("display", "grid", true),
            ("display", "inline-grid", true),
            ("display", "ruby", false),
            ("width", "12pt", true),
            ("width", "-1pt", false),
            ("width", "min-content", true),
            ("width", "fit-content(10pt)", false),
            ("width", "calc(2pt * 3)", true),
            ("width", "var(--size)", true),
            ("unknown", "var(--size)", false),
            ("font-style", "oblique 14deg", true),
            ("font-style", "oblique 20deg", true),
            ("font", "italic 12pt serif", true),
            ("font", "small-caps 12pt serif", true),
            (
                "background",
                "url('not-fetched.png') center/cover no-repeat",
                true,
            ),
            (
                "background-image",
                "linear-gradient(red,blue),conic-gradient(red,blue)",
                true,
            ),
            (
                "background-image",
                "linear-gradient(red,blue),element(#source)",
                false,
            ),
            ("text-transform", "uppercase full-width", true),
            (
                "text-transform",
                "full-size-kana lowercase full-width",
                true,
            ),
            ("text-transform", "math-auto", false),
            ("tab-size", "2pt", false),
            ("hyphens", "manual", true),
            ("hyphens", "auto", false),
            ("float", "left", true),
            ("float", "inline-end", true),
            ("clear", "both", true),
            ("float", "top", false),
            ("--custom", "", true),
            ("--", "red", false),
            ("--custom", "{raw:tokens}", true),
            ("--custom", "var(--missing,)", true),
            ("--custom", "var(b)", false),
            ("color", "red!important", true),
            ("color", "red;", false),
            ("color", "red !nope", false),
            ("width", "calc(1pt", false),
            ("--custom", "var(--missing", false),
            ("--custom", "value;", false),
            ("all", "initial", true),
            ("width", "inherit", true),
            ("unknown", "initial", false),
        ] {
            assert_eq!(
                supports_declaration(name, value),
                expected,
                "({name}:{value})"
            );
        }
    }
}
