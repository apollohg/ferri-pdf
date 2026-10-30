//! Conformance of the engine to the CSS specification data in `spec/css`: grammars accept
//! generated valid values and reject invalid mutations, initial values compute to the
//! engine's defaults, inheritance follows the data, and every other property is rejected.
use crate::dom::{DocumentTree, Selector};
use crate::resources::FileResolver;
use crate::style::{
    Style, StyleError, Stylesheet, StylesheetSource,
    component::Component,
    declaration,
    grammar::{
        Base, BlockKind, DESCRIPTORS, Initial, Keyword, Label, NODES, Node, NodeId, Origin,
        Production, Property, Range, SPEC_UNITS, UNBOUNDED,
    },
    properties::computed,
};
use std::collections::{BTreeMap, BTreeSet};

const MAX_SAMPLES: usize = 8;
const MAX_SAMPLE_DEPTH: usize = 24;
/// Repetitions sampled beyond a multiplier's minimum when its maximum is unbounded.
const UNBOUNDED_SAMPLE_EXTRA: usize = 2;
const BOGUS_KEYWORD: &str = "zz-not-a-css-keyword";
const NEGATIVE_LENGTH: &str = "-5pt";
const NEGATIVE_NUMBER: &str = "-5";
const NEGATIVE_PERCENTAGE: &str = "-5%";
const PARENT_ELEMENT: &str = "#parent";
const CHILD_ELEMENT: &str = "#child";
const DOCUMENT: &str = "<div id=parent><span id=child>X</span></div>";
/// Properties whose initial value the data gives as prose because it depends on the user agent.
const USER_AGENT_INITIAL: &[Property] = &[Property::FontFamily];
const INITIAL_GAPS: &[(Property, &str)] = &[];
/// Supported properties whose values no computed field records, because every accepted value
/// renders alike, or, for `will-change`, only the boxes its named properties generate count;
/// a user agent `!important` rule fixes `overlay` (CSS Position 4 §3.4).
const WITHOUT_COMPUTED_FIELD: &[Property] = &[
    Property::Content,
    Property::WillChange,
    Property::FontSynthesisPosition,
    Property::FontSynthesisSmallCaps,
    Property::Overlay,
];
const EXTENSION_PROPERTIES: [&str; 2] = ["table-column-span", "table-row-span"];
/// `calc-size()` arguments are `<calc-sum>`s that must resolve to lengths, which the grammar
/// leaves to CSS Values 5 §10; every sizing property accepts these bases.
const CALC_SIZE_SAMPLES: [&str; 3] = [
    "calc-size(min-content, size * 2)",
    "calc-size(any, 12pt)",
    "calc-size(50%, size + 1pt)",
];

type Tokens = Vec<String>;

/// Deterministic, bounded valid values for a grammar node.
struct Sampler {
    stack: Vec<NodeId>,
}

impl Sampler {
    fn new() -> Self {
        Self { stack: Vec::new() }
    }

    fn samples(&mut self, id: NodeId) -> Vec<Tokens> {
        if self.stack.contains(&id) || self.stack.len() > MAX_SAMPLE_DEPTH {
            return Vec::new();
        }
        self.stack.push(id);
        let mut samples = match &NODES[usize::from(id)] {
            Node::Keyword(keyword) => vec![vec![keyword.name().to_owned()]],
            Node::Delim(c) => vec![vec![c.to_string()]],
            Node::Base(base, range) => base_samples(*base, *range)
                .into_iter()
                .map(|sample| vec![sample])
                .collect(),
            Node::Function(Keyword::CalcSize, _) => CALC_SIZE_SAMPLES
                .iter()
                .map(|sample| vec![(*sample).to_owned()])
                .collect(),
            Node::Function(name, body) => match body {
                None => vec![vec![format!("{}()", name.name())]],
                Some(body) => self
                    .samples(*body)
                    .into_iter()
                    .map(|tokens| vec![format!("{}({})", name.name(), joined(tokens))])
                    .collect(),
            },
            Node::Block(kind, body) => {
                let (open, close) = match kind {
                    BlockKind::Parenthesis => ("(", ")"),
                    BlockKind::SquareBracket => ("[", "]"),
                };
                self.samples(*body)
                    .into_iter()
                    .map(|tokens| vec![format!("{open}{}{close}", joined(tokens))])
                    .collect()
            }
            Node::Sequence(children) => self.sequence(children),
            Node::OneOf(alternatives) => {
                let per_alternative: Vec<Vec<Tokens>> =
                    alternatives.iter().map(|alt| self.samples(*alt)).collect();
                interleave(&per_alternative)
            }
            Node::AnyOf(alternatives) => self.unordered(alternatives, false),
            Node::AllOf(alternatives) => self.unordered(alternatives, true),
            Node::Repeat {
                node,
                min,
                max,
                comma,
            } => self.repeat(*node, usize::from(*min), usize::from(*max), *comma),
            Node::NonEmpty(node) => self
                .samples(*node)
                .into_iter()
                .map(normalized)
                .filter(|tokens| !tokens.is_empty())
                .collect(),
            Node::Reference(_, target) => self.samples(*target),
        };
        self.stack.pop();
        samples.truncate(MAX_SAMPLES);
        samples
    }

    /// The first sample of every child, then one variation per child.
    fn sequence(&mut self, children: &[NodeId]) -> Vec<Tokens> {
        let per_child: Vec<Vec<Tokens>> =
            children.iter().map(|child| self.samples(*child)).collect();
        if per_child.iter().any(Vec::is_empty) {
            return Vec::new();
        }
        let mut samples = vec![
            per_child
                .iter()
                .flat_map(|s| s[0].clone())
                .collect::<Tokens>(),
        ];
        for (index, child) in per_child.iter().enumerate() {
            for variation in child.iter().skip(1) {
                samples.push(
                    per_child
                        .iter()
                        .enumerate()
                        .flat_map(|(i, s)| {
                            if i == index {
                                variation.clone()
                            } else {
                                s[0].clone()
                            }
                        })
                        .collect(),
                );
            }
        }
        samples
    }

    /// Each alternative alone (for `||`), then all in grammar order and all reversed.
    fn unordered(&mut self, alternatives: &[NodeId], all: bool) -> Vec<Tokens> {
        let firsts: Vec<Option<Tokens>> = alternatives
            .iter()
            .map(|alternative| self.samples(*alternative).into_iter().next())
            .collect();
        let Some(firsts) = firsts.into_iter().collect::<Option<Vec<_>>>() else {
            return Vec::new();
        };
        let mut samples = vec![
            firsts.iter().flatten().cloned().collect::<Tokens>(),
            firsts.iter().rev().flatten().cloned().collect(),
        ];
        if !all {
            samples.extend(firsts.iter().cloned());
        }
        samples
    }

    fn repeat(&mut self, node: NodeId, min: usize, max: usize, comma: bool) -> Vec<Tokens> {
        let items = self.samples(node);
        let Some(first) = items.first() else {
            return if min == 0 {
                vec![Vec::new()]
            } else {
                Vec::new()
            };
        };
        let largest = if max == usize::from(UNBOUNDED) {
            min.max(1) + UNBOUNDED_SAMPLE_EXTRA
        } else {
            max
        };
        let mut counts = BTreeSet::from([min, min.max(1).min(max), largest]);
        counts.retain(|count| *count <= max);
        counts
            .into_iter()
            .map(|count| {
                let mut tokens = Tokens::new();
                for index in 0..count {
                    if index > 0 && comma {
                        tokens.push(",".into());
                    }
                    tokens.extend(items.get(index % items.len()).unwrap_or(first).clone());
                }
                tokens
            })
            .collect()
    }
}

fn base_samples(base: Base, range: Range) -> Vec<String> {
    let number = |preferred: f32| {
        if range.contains(preferred) {
            preferred
        } else if range.min.is_finite() {
            range.min
        } else {
            range.max
        }
    };
    let signed = |positive: &str, negative: &str| {
        let mut samples = vec![positive.to_owned()];
        if range.min < 0.0 {
            samples.push(negative.to_owned());
        }
        samples
    };
    match base {
        Base::Angle => signed("45deg", "-30deg"),
        Base::AnglePercentage => vec!["45deg".into(), "10%".into()],
        Base::AttrUnit => vec!["px".into(), "%".into()],
        Base::Color => vec!["red".into(), "#123456".into(), "currentcolor".into()],
        Base::CustomIdent => vec!["sample-ident".into()],
        Base::DashedIdent => vec!["--sample".into()],
        Base::DeclarationValue => vec!["sample tokens".into()],
        Base::Dimension => vec!["3px".into()],
        Base::Flex => vec!["1fr".into()],
        Base::HashToken => vec!["#sample".into()],
        Base::Ident => vec!["sample".into()],
        Base::Integer => {
            let mut samples = vec![format!("{}", number(2.0) as i32)];
            if range.contains(3.0) {
                samples.push("3".into());
            }
            samples
        }
        Base::Length => signed("12pt", "-3pt"),
        Base::LengthPercentage => {
            let mut samples = signed("12pt", "-3pt");
            samples.push("50%".into());
            samples
        }
        Base::Number => {
            let mut samples = vec![format!("{}", number(2.0))];
            if range.min < 0.0 {
                samples.push("-1.5".into());
            }
            samples
        }
        Base::Percentage => vec![format!("{}%", number(50.0))],
        Base::Resolution => vec!["2dppx".into()],
        Base::String => vec!["'text'".into()],
        Base::Time => signed("2s", "-1s"),
        Base::UnicodeRangeToken => vec!["U+0-7F".into()],
        Base::Url => vec!["url(sample.png)".into()],
        Base::Zero => vec!["0".into()],
    }
}

/// Round-robin over alternatives so every alternative contributes before any repeats.
fn interleave(per_alternative: &[Vec<Tokens>]) -> Vec<Tokens> {
    let longest = per_alternative.iter().map(Vec::len).max().unwrap_or(0);
    (0..longest)
        .flat_map(|index| {
            per_alternative
                .iter()
                .filter_map(move |samples| samples.get(index).cloned())
        })
        .collect()
}

/// CSS Values 4 §2.6: grammar commas next to omitted terms are omitted.
fn normalized(tokens: Tokens) -> Tokens {
    let mut output: Tokens = Vec::new();
    for token in tokens {
        if token == "," && output.last().is_none_or(|last| last == ",") {
            continue;
        }
        output.push(token);
    }
    if output.last().is_some_and(|last| last == ",") {
        output.pop();
    }
    output
}

fn joined(tokens: Tokens) -> String {
    normalized(tokens).join(" ")
}

/// What a grammar can accept outside function arguments, for choosing sound invalid values.
#[derive(Default)]
struct Reach {
    bases: Vec<(Base, Range)>,
    delims: BTreeSet<char>,
    comma_lists: bool,
}

fn reach(root: NodeId) -> Reach {
    let mut reach = Reach::default();
    let mut seen = BTreeSet::new();
    let mut pending = vec![root];
    while let Some(id) = pending.pop() {
        if !seen.insert(id) {
            continue;
        }
        match &NODES[usize::from(id)] {
            Node::Base(base, range) => reach.bases.push((*base, *range)),
            Node::Delim(c) => {
                reach.delims.insert(*c);
            }
            Node::Keyword(_) | Node::Function(..) | Node::Block(..) => {}
            Node::Repeat { node, comma, .. } => {
                reach.comma_lists |= *comma;
                pending.push(*node);
            }
            Node::NonEmpty(node) | Node::Reference(_, node) => pending.push(*node),
            Node::Sequence(children)
            | Node::AllOf(children)
            | Node::AnyOf(children)
            | Node::OneOf(children) => pending.extend(*children),
        }
    }
    reach
}

impl Reach {
    fn accepts_any_ident(&self) -> bool {
        self.bases.iter().any(|(base, _)| {
            matches!(
                base,
                Base::Ident
                    | Base::CustomIdent
                    | Base::DashedIdent
                    | Base::AttrUnit
                    | Base::DeclarationValue
            )
        })
    }
    fn any(&self, bases: &[Base]) -> bool {
        self.bases.iter().any(|(base, _)| bases.contains(base))
    }
    fn negative(&self, bases: &[Base]) -> bool {
        self.bases
            .iter()
            .any(|(base, range)| bases.contains(base) && range.min < 0.0)
    }
}

/// The most tokens a grammar accepts, or `None` without a bound.
fn max_tokens(id: NodeId, stack: &mut Vec<NodeId>) -> Option<usize> {
    if stack.contains(&id) {
        return None;
    }
    stack.push(id);
    let tokens = match &NODES[usize::from(id)] {
        Node::Base(Base::DeclarationValue | Base::UnicodeRangeToken, _) => None,
        Node::Keyword(_)
        | Node::Delim(_)
        | Node::Base(..)
        | Node::Function(..)
        | Node::Block(..) => Some(1),
        Node::Sequence(children) | Node::AllOf(children) | Node::AnyOf(children) => {
            children.iter().map(|child| max_tokens(*child, stack)).sum()
        }
        Node::OneOf(children) => children
            .iter()
            .map(|child| max_tokens(*child, stack))
            .collect::<Option<Vec<_>>>()
            .and_then(|counts| counts.into_iter().max()),
        Node::Repeat {
            node, max, comma, ..
        } if *max != UNBOUNDED => max_tokens(*node, stack).map(|per| {
            let max = usize::from(*max);
            per * max + if *comma { max.saturating_sub(1) } else { 0 }
        }),
        Node::Repeat { .. } => None,
        Node::NonEmpty(node) | Node::Reference(_, node) => max_tokens(*node, stack),
    };
    stack.pop();
    tokens
}

fn target(root: NodeId) -> NodeId {
    match NODES[usize::from(root)] {
        Node::Reference(_, target) => target,
        _ => root,
    }
}

/// Values the grammar must reject, each with the reason it is invalid.
fn invalid_values(root: NodeId, valid: &[Tokens]) -> Vec<(String, &'static str)> {
    let reach = reach(root);
    let mut invalid = Vec::new();
    if !reach.accepts_any_ident() {
        invalid.push((BOGUS_KEYWORD.to_owned(), "keyword outside the grammar"));
    }
    if reach.any(&[Base::Length, Base::LengthPercentage])
        && !reach.negative(&[Base::Length, Base::LengthPercentage, Base::Dimension])
        && !reach.any(&[Base::DeclarationValue])
    {
        invalid.push((
            NEGATIVE_LENGTH.into(),
            "negative length below the [0,∞] range",
        ));
    }
    if reach.any(&[Base::Number, Base::Integer])
        && !reach.negative(&[Base::Number, Base::Integer])
        && !reach.any(&[Base::DeclarationValue])
    {
        invalid.push((NEGATIVE_NUMBER.into(), "negative number below the range"));
    }
    if reach.any(&[Base::Percentage, Base::LengthPercentage])
        && !reach.negative(&[
            Base::Percentage,
            Base::LengthPercentage,
            Base::AnglePercentage,
        ])
        && !reach.any(&[Base::DeclarationValue])
    {
        invalid.push((
            NEGATIVE_PERCENTAGE.into(),
            "negative percentage below the range",
        ));
    }
    if let (Some(limit), Some(token)) = (
        max_tokens(root, &mut Vec::new()),
        valid
            .iter()
            .filter_map(|tokens| tokens.first())
            .find(|token| *token != ","),
    ) {
        invalid.push((
            vec![token.as_str(); limit + 1].join(" "),
            "one component beyond the multiplier maximum",
        ));
    }
    if let Node::Repeat {
        node, max, comma, ..
    } = NODES[usize::from(target(root))]
        && max >= 2
        && let Some(item) = Sampler::new()
            .samples(node)
            .into_iter()
            .find(|tokens| tokens.len() == 1)
        && !reach.any(&[Base::DeclarationValue])
    {
        let item = &item[0];
        if comma && !reach.delims.contains(&'/') {
            invalid.push((
                format!("{item} / {item}"),
                "slash where the list needs commas",
            ));
        }
        if !comma && !reach.delims.contains(&',') && !reach.comma_lists {
            invalid.push((
                format!("{item}, {item}"),
                "comma where the list needs spaces",
            ));
        }
    }
    invalid
}

fn check_grammar(
    name: &str,
    syntax: &str,
    root: NodeId,
    matches: impl Fn(&str) -> Result<bool, StyleError>,
    failures: &mut Vec<String>,
) -> usize {
    let valid = Sampler::new().samples(root);
    if valid.is_empty() {
        failures.push(format!("{name}: no sample generated from `{syntax}`"));
    }
    let mut checked = 0;
    for tokens in &valid {
        let value = joined(tokens.clone());
        checked += 1;
        match matches(&value) {
            Ok(true) => {}
            Ok(false) => failures.push(format!(
                "{name}: generated valid value `{value}` was rejected by `{syntax}`"
            )),
            Err(error) => failures.push(format!("{name}: `{value}` failed to match: {error}")),
        }
    }
    for (value, reason) in invalid_values(root, &valid) {
        checked += 1;
        match matches(&value) {
            Ok(false) => {}
            Ok(true) => failures.push(format!(
                "{name}: invalid value `{value}` ({reason}) was accepted by `{syntax}`"
            )),
            Err(error) => failures.push(format!("{name}: `{value}` failed to match: {error}")),
        }
    }
    checked
}

fn report(what: &str, failures: &[String]) {
    assert!(
        failures.is_empty(),
        "{} {what}:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn property_grammars_accept_generated_values_and_reject_mutations() {
    let mut failures = Vec::new();
    let mut checked = 0;
    for property in Property::ALL {
        let data = property.data();
        checked += check_grammar(
            property.name(),
            data.syntax,
            data.root,
            // A value the grammar matches but specification prose excludes still matched.
            |value| {
                Ok(declaration::value(property, value, &mut 0)?.is_some()
                    || crate::style::matcher::match_value(
                        data.root,
                        value,
                        &mut 0,
                        crate::style::matcher::ValueContext::Element,
                    )?
                    .is_some_and(|matched| {
                        !crate::style::grid_values::valid(property, &matched)
                            || !crate::style::logical::valid(property, &matched)
                            || !crate::style::inert::valid(property, &matched)
                            || !crate::style::counter_style::valid(&matched)
                            || !crate::style::colors::interpolation_spaces_exist(&matched)
                    }))
            },
            &mut failures,
        );
    }
    for data in &DESCRIPTORS {
        checked += check_grammar(
            &format!("{} {}", data.at_rule, data.name),
            data.syntax,
            data.root,
            |value| Ok(declaration::descriptor(data.descriptor, value)?.is_some()),
            &mut failures,
        );
    }
    println!("checked {checked} generated values");
    report("grammar conformance failures", &failures);
}

#[test]
fn matching_follows_value_definition_semantics() {
    use Property as P;
    let cases: &[(Property, &str, bool, &str)] = &[
        (P::Margin, "1px 2px 3px 4px", true, "{1,4} takes four"),
        (
            P::Margin,
            "1px 2px 3px 4px 5px",
            false,
            "{1,4} stops at four",
        ),
        (P::Border, "solid 1px red", true, "|| accepts any order"),
        (
            P::Border,
            "1px solid red solid",
            false,
            "|| takes each alternative once",
        ),
        (
            P::Background,
            "url(a.png), red",
            true,
            "the final layer takes the colour",
        ),
        (
            P::Background,
            "red, url(a.png)",
            false,
            "only the final layer has a colour",
        ),
        (
            P::Background,
            "red",
            true,
            "a comma after omitted terms is omitted",
        ),
        (
            P::Background,
            ", red",
            false,
            "a written comma needs a term before it",
        ),
        (
            P::Content,
            "counter(page)",
            true,
            "a comma before omitted terms is omitted",
        ),
        (
            P::Content,
            "counter(page,)",
            false,
            "a written comma needs a term after it",
        ),
        (
            P::Content,
            "counter(page decimal)",
            false,
            "a comma between written terms is required",
        ),
        (
            P::Font,
            "bold 12px/1.5 Arial, sans-serif",
            true,
            "juxtaposition in order",
        ),
        (P::Font, "bold 12px", false, "the family is required"),
        (P::TransformOrigin, "top left", true, "&& accepts any order"),
        (
            P::TransformOrigin,
            "top 10pt",
            false,
            "lengths are horizontal first",
        ),
        (P::Width, "-1px", false, "[0,∞] excludes negatives"),
        (
            P::MarginTop,
            "-1px",
            true,
            "unrestricted lengths may be negative",
        ),
        (
            P::Width,
            "calc(100% - 10px)",
            true,
            "math functions where lengths are",
        ),
        (
            P::Width,
            "calc(100% -)",
            false,
            "math arguments follow the calc grammar",
        ),
        (
            P::Color,
            "canvastext",
            true,
            "system colours through the CSS Color grammar",
        ),
        (
            P::Color,
            "color-mix(in srgb, red, blue)",
            true,
            "CSS Color 5 mixing",
        ),
        (P::Color, "notacolor", false, "unknown colour keywords"),
        (P::AspectRatio, "16 / 9", true, "literal slash"),
        (
            P::BoxShadow,
            "1px 1px 2px red, inset 0 0 1px blue",
            true,
            "# lists of || groups",
        ),
        (
            P::GridTemplateColumns,
            "repeat(2, 1fr) 100px",
            true,
            "nested functions",
        ),
        (
            P::GridTemplateAreas,
            "'a a' 'b a'",
            false,
            "named areas are filled rectangles",
        ),
        (
            P::GridTemplateAreas,
            "'a b' 'c'",
            false,
            "area rows have equal lengths",
        ),
        (
            P::GridTemplateAreas,
            "'a . .' '... b b'",
            true,
            "runs of full stops are one null cell",
        ),
        (P::GridTemplateAreas, "'a!'", false, "trash tokens"),
        (P::GridRowStart, "0", false, "grid line zero"),
        (P::GridArea, "1 / span 0", false, "spans are positive"),
        (P::GridRow, "-1 / span name", true, "named spans"),
        (
            P::GridTemplate,
            "'a b' 'c c' / 1fr 2fr",
            true,
            "area strings with explicit columns",
        ),
        (
            P::GridTemplateColumns,
            "repeat(auto-fill, 10px) repeat(auto-fit, 10px)",
            false,
            "one auto repetition",
        ),
        (
            P::GridTemplateColumns,
            "-1px",
            false,
            "track sizes are non-negative",
        ),
        (P::Display, "inline flex", true, "two-value display"),
        (P::Display, "flex flex", false, "an inner type once"),
        (
            P::TextTransform,
            "uppercase full-width",
            true,
            "|| inside |",
        ),
        (
            P::TextTransform,
            "uppercase lowercase",
            false,
            "one case keyword",
        ),
        (P::ZIndex, "1.5", false, "<integer> rejects fractions"),
        (P::FontWeight, "1001", false, "[1,1000] bounds numbers"),
        (
            P::ListStyle,
            "inside circle",
            true,
            "custom idents after keywords",
        ),
    ];
    let mut failures = Vec::new();
    for (property, value, valid, rule) in cases {
        match declaration::value(*property, value, &mut 0) {
            Ok(matched) if matched.is_some() == *valid => {}
            result => failures.push(format!(
                "{}: `{value}` must be {} ({rule}), got {result:?}",
                property.name(),
                if *valid { "valid" } else { "invalid" }
            )),
        }
    }
    let matched = |property: Property, value: &str| {
        declaration::value(property, value, &mut 0)
            .expect("bounded value")
            .expect("valid value")
    };
    let list_style = matched(P::ListStyle, "inside circle");
    if list_style
        .find(Label::Property(P::ListStylePosition))
        .and_then(Component::keyword)
        != Some(Keyword::Inside)
    {
        failures.push(format!(
            "list-style: `inside` must claim the position keyword: {list_style:?}"
        ));
    }
    if !matches!(
        list_style.find(Label::Production(Production::CounterStyleName)).map(Component::unwrap),
        Some(Component::Ident(name)) if name.as_ref() == "circle"
    ) {
        failures.push(format!(
            "list-style: `circle` must be the counter style: {list_style:?}"
        ));
    }
    report("matching semantics failures", &failures);
}

/// Identifiers and keywords in source order, as the match tree claimed them.
fn claims(component: &Component, output: &mut Vec<String>) {
    match component {
        Component::Keyword(keyword) => output.push(format!("keyword {}", keyword.name())),
        Component::Ident(name) => output.push(format!("ident {name}")),
        Component::String(text) => output.push(format!("string {text}")),
        Component::Integer(integer) => output.push(format!("integer {integer}")),
        Component::Reference(_, inner) => claims(inner, output),
        Component::Group(items) => items.iter().for_each(|item| claims(item, output)),
        Component::Slots(slots) => slots.iter().flatten().for_each(|slot| claims(slot, output)),
        _ => output.push(format!("{component:?}")),
    }
}

#[test]
fn custom_idents_yield_only_to_keywords_valid_at_their_position() {
    use Property as P;
    // CSS Values 4 §4.2: a keyword that could start a list entry is no `<custom-ident>` there,
    // while the same word later in a multi-word name, where no keyword fits, is one.
    let cases: &[(Property, &str, Option<&[&str]>)] = &[
        (P::FontFamily, "serif", Some(&["keyword serif"])),
        (
            P::FontFamily,
            "DejaVu Serif, sans-serif",
            Some(&["ident DejaVu", "ident Serif", "keyword sans-serif"]),
        ),
        (
            P::FontFamily,
            "'sans-serif', Open Monospace Light, monospace",
            Some(&[
                "string sans-serif",
                "ident Open",
                "ident Monospace",
                "ident Light",
                "keyword monospace",
            ]),
        ),
        (P::FontFamily, "Serif Display, serif", None),
        (P::FontFamily, "Math Sans", None),
        (P::FontFamily, "Foo, cursive Hand", None),
        (
            P::TransitionProperty,
            "opacity, all, transform",
            Some(&["ident opacity", "keyword all", "ident transform"]),
        ),
        (P::TransitionProperty, "none", Some(&["keyword none"])),
        (P::TransitionProperty, "none, opacity", None),
        (P::TransitionProperty, "all opacity", None),
        (P::CounterReset, "none", Some(&["keyword none"])),
        (
            P::CounterReset,
            "chapter 2 section",
            Some(&["ident chapter", "integer 2", "ident section"]),
        ),
        (P::CounterReset, "none chapter", None),
        (
            P::WillChange,
            "contents, transform, scroll-position",
            Some(&[
                "keyword contents",
                "ident transform",
                "keyword scroll-position",
            ]),
        ),
        (P::WillChange, "auto, transform", None),
    ];
    let mut failures = Vec::new();
    for (property, value, expected) in cases {
        let matched = declaration::value(*property, value, &mut 0).expect("bounded value");
        let found = matched.as_ref().map(|component| {
            let mut output = Vec::new();
            claims(component, &mut output);
            output
        });
        let expected = expected.map(|claims| claims.iter().map(ToString::to_string).collect());
        if found != expected {
            failures.push(format!(
                "{}: `{value}` claims {found:?}, expected {expected:?}",
                property.name()
            ));
        }
    }
    report("positional keyword failures", &failures);
}

fn styles(css: &str) -> Result<(Style, Style), StyleError> {
    let sheet = Stylesheet::parse(&[StylesheetSource {
        text: css.into(),
        base: Default::default(),
    }])?;
    let html = DocumentTree::parse(DOCUMENT).expect("parse test HTML");
    let select = |selector: &str| {
        html.select(&Selector::parse(selector).expect("static selector"))
            .next()
            .expect("element in the static document")
    };
    let parent = sheet.computed_with_resolver(
        select(PARENT_ELEMENT),
        &Style::default(),
        &FileResolver::default(),
    )?;
    let child =
        sheet.computed_with_resolver(select(CHILD_ELEMENT), &parent, &FileResolver::default())?;
    Ok((parent, child))
}

fn styles_in_context(property: Property, css: &str) -> Result<(Style, Style), StyleError> {
    if Property::BorderWidth
        .longhands()
        .contains(&crate::style::logical::physical(
            property,
            Default::default(),
        ))
    {
        styles(&format!(
            "{PARENT_ELEMENT},{CHILD_ELEMENT}{{border-style:solid}}{css}"
        ))
    } else if property == Property::OutlineWidth {
        styles(&format!(
            "{PARENT_ELEMENT},{CHILD_ELEMENT}{{outline-style:solid}}{css}"
        ))
    } else {
        styles(css)
    }
}

fn computed_text(style: &Style) -> String {
    format!("{style:?}")
}

fn supported_longhands() -> impl Iterator<Item = Property> {
    Property::ALL
        .into_iter()
        .filter(|property| computed(*property) && property.longhands().is_empty())
}

fn initial_text(property: Property) -> Option<&'static str> {
    match property.data().initial {
        Initial::Value(value) => Some(value),
        Initial::Prose | Initial::Longhands(_) => None,
    }
}

#[test]
fn initial_values_compute_to_the_engine_defaults() {
    let (_, default) = styles("").expect("empty stylesheet");
    let default = computed_text(&default);
    let mut failures = Vec::new();
    let mut gaps = Vec::new();
    for property in supported_longhands() {
        let name = property.name();
        let parsed = initial_text(property).filter(|_| declaration::initial(property).is_some());
        let Some(initial) = parsed else {
            if !USER_AGENT_INITIAL.contains(&property) {
                failures.push(format!(
                    "{name}: initial value {:?} is prose or does not match its own grammar",
                    property.data().initial
                ));
            }
            continue;
        };
        match styles(&format!("{CHILD_ELEMENT}{{{name}:{initial}}}")) {
            Ok((_, child)) if computed_text(&child) == default => {}
            Ok((_, child)) => failures.push(format!(
                "{name}: initial `{initial}` computes to\n  {}\nbut the engine default is\n  {default}",
                computed_text(&child)
            )),
            Err(error) => gaps.push((property, format!("{name}: {initial}: {error}"))),
        }
    }
    let known: Vec<Property> = INITIAL_GAPS.iter().map(|(property, _)| *property).collect();
    let found: Vec<Property> = gaps.iter().map(|(property, _)| *property).collect();
    if found != known {
        failures.push(format!(
            "initial values the engine cannot compute changed: {:?}, recorded {INITIAL_GAPS:?}",
            gaps.iter().map(|(_, message)| message).collect::<Vec<_>>()
        ));
    }
    report("initial-value conformance failures", &failures);
}

/// The first sample that computes without error and changes the element's style.
fn effective_sample(property: Property) -> Option<String> {
    let (default, _) = styles_in_context(property, "").expect("empty stylesheet");
    let default = computed_text(&default);
    Sampler::new()
        .samples(property.data().root)
        .into_iter()
        .map(joined)
        .find(|sample| {
            styles_in_context(
                property,
                &format!("{PARENT_ELEMENT}{{{}:{sample}}}", property.name()),
            )
            .is_ok_and(|(parent, _)| computed_text(&parent) != default)
        })
}

#[test]
fn inheritance_follows_the_specification_data() {
    let mut failures = Vec::new();
    for property in supported_longhands() {
        let name = property.name();
        let Some(sample) = effective_sample(property) else {
            if !WITHOUT_COMPUTED_FIELD.contains(&property) {
                failures.push(format!(
                    "{name}: no generated value changes the computed style"
                ));
            }
            continue;
        };
        let inherited = property.inherited();
        let keyword = if inherited { "inherit" } else { "initial" };
        let implicit = styles_in_context(property, &format!("{PARENT_ELEMENT}{{{name}:{sample}}}"));
        let explicit = styles_in_context(
            property,
            &format!("{PARENT_ELEMENT}{{{name}:{sample}}}{CHILD_ELEMENT}{{{name}:{keyword}}}"),
        );
        match (implicit, explicit) {
            (Ok((_, implicit)), Ok((_, explicit))) if computed_text(&implicit) == computed_text(&explicit) => {}
            (Ok((_, implicit)), Ok((_, explicit))) => failures.push(format!(
                "{name} (inherited: {inherited}): with the parent at `{sample}`, the child computes\n  {}\nbut `{keyword}` gives\n  {}",
                computed_text(&implicit),
                computed_text(&explicit)
            )),
            (implicit, explicit) => failures.push(format!(
                "{name}: `{sample}` failed: {:?} / {:?}",
                implicit.err(),
                explicit.err()
            )),
        }
    }
    report("inheritance conformance failures", &failures);
}

#[test]
fn unsupported_properties_keep_their_initial_value_or_fail() {
    let mut failures = Vec::new();
    for property in Property::ALL {
        if computed(property) || !declaration::longhands(property).eq([property]) {
            continue;
        }
        let name = property.name();
        if let Some(initial) =
            initial_text(property).filter(|_| declaration::initial(property).is_some())
            && let Err(error) = styles(&format!("{CHILD_ELEMENT}{{{name}:{initial}}}"))
        {
            failures.push(format!(
                "{name}: its initial value `{initial}` must render as the engine does: {error}"
            ));
        }
        let non_initial = Sampler::new()
            .samples(property.data().root)
            .into_iter()
            .map(joined)
            .find(|sample| {
                declaration::value(property, sample, &mut 0)
                    .ok()
                    .flatten()
                    .is_some_and(|value| Some(&value) != declaration::initial(property))
            });
        if let Some(sample) = non_initial {
            match styles(&format!("{CHILD_ELEMENT}{{{name}:{sample}}}")) {
                Err(StyleError::Unsupported(_)) => {}
                other => failures.push(format!(
                    "{name}: unsupported value `{sample}` must be rejected as unsupported, got {:?}",
                    other.err()
                )),
            }
        }
    }
    report("unsupported-property failures", &failures);
}

#[test]
fn every_supported_property_comes_from_the_data() {
    let mut failures = Vec::new();
    for property in Property::ALL.into_iter().filter(|p| computed(*p)) {
        let data = property.data();
        if !property.longhands().is_empty() {
            failures.push(format!(
                "{}: shorthands expand, they have no computed value",
                property.name()
            ));
        }
        if data.origin == Origin::Extension && !EXTENSION_PROPERTIES.contains(&property.name()) {
            failures.push(format!("{}: unexpected extension", property.name()));
        }
        if Property::from_name(property.name()) != Some(property) {
            failures.push(format!(
                "{}: name lookup does not round-trip",
                property.name()
            ));
        }
    }
    let extensions: Vec<&str> = Property::ALL
        .into_iter()
        .filter(|p| p.data().origin == Origin::Extension)
        .map(Property::name)
        .collect();
    if extensions != EXTENSION_PROPERTIES {
        failures.push(format!(
            "extensions {extensions:?}, expected {EXTENSION_PROPERTIES:?}"
        ));
    }
    for unit in SPEC_UNITS {
        if crate::style::units::dimension(unit).is_none() {
            failures.push(format!("unit {unit} from the data has no dimension"));
        }
    }
    report("data provenance failures", &failures);
}

/// Per-module coverage report: `cargo test -p ferri-pdf-style module_coverage
/// -- --nocapture`.
#[test]
fn module_coverage() {
    let mut modules: BTreeMap<&str, (Vec<&str>, usize)> = BTreeMap::new();
    for property in Property::ALL {
        for group in property.data().groups {
            let entry = modules.entry(group).or_default();
            entry.1 += 1;
            if computed(property)
                || (!property.longhands().is_empty() && property_expands(property))
            {
                entry.0.push(property.name());
            }
        }
    }
    println!("| Module (spec data group) | Properties | Computed | Computed properties |");
    println!("|---|---|---|---|");
    let mut totals = (0, 0);
    for (module, (computed, total)) in &modules {
        totals.0 += computed.len();
        totals.1 += total;
        println!(
            "| {module} | {total} | {} | {} |",
            computed.len(),
            computed
                .iter()
                .map(|name| format!("`{name}`"))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    println!("| **All** | {} | {} | |", totals.1, totals.0);
    assert!(
        modules
            .values()
            .map(|(computed, _)| computed.len())
            .sum::<usize>()
            > 0,
        "the registry computes at least one property"
    );
}

/// A shorthand counts as computed when every final longhand has a computed field.
fn property_expands(property: Property) -> bool {
    declaration::longhands(property).all(computed)
}

#[test]
fn supports_classification_agrees_with_real_computation_for_concrete_generated_values() {
    let mut failures = Vec::new();
    let mut checked = 0;
    let mut deferred = 0;
    for property in Property::ALL {
        for tokens in Sampler::new().samples(property.data().root) {
            let value = joined(tokens);
            if declaration::supports_declaration(property.name(), &value) {
                // Values 5 Appendix A defers the entire receiving grammar for substitution.
                if crate::style::variables::Value::with_property_substitutions(&value)
                    .expect("generated substitution syntax")
                    .is_some()
                {
                    deferred += 1;
                    continue;
                }
                checked += 1;
                let result = if property == Property::Content {
                    let sheet = Stylesheet::parse(&[StylesheetSource {
                        text: format!("{CHILD_ELEMENT}::before{{content:{value}}}"),
                        base: Default::default(),
                    }]);
                    sheet.and_then(|sheet| {
                        let html = DocumentTree::parse(DOCUMENT).expect("parse test HTML");
                        let element = html
                            .select(&Selector::parse(CHILD_ELEMENT).unwrap())
                            .next()
                            .unwrap();
                        sheet
                            .pseudo(
                                element,
                                &Style::default(),
                                crate::style::PseudoElement::Before,
                            )
                            .map(|_| ())
                    })
                } else {
                    styles(&format!("{CHILD_ELEMENT}{{{}:{value}}}", property.name())).map(|_| ())
                };
                if let Err(error) = result {
                    failures.push(format!(
                        "{}:{value} reports support but computation failed: {error}",
                        property.name()
                    ));
                }
            }
        }
    }
    assert!(
        checked > supported_longhands().count(),
        "queries must exercise multiple usable values across the property registry; checked {checked}"
    );
    assert!(
        deferred > 0,
        "generated pending values require the separate substitution-stage assertions"
    );
    report("support/application disagreements", &failures);
}

#[test]
fn supports_accepts_pending_values_without_promising_computed_capability() {
    const FIXED_VALUE: &str = "'text' content(first-letter)";
    assert!(!declaration::supports_declaration("content", FIXED_VALUE));
    for substitution in ["attr(sample)", "var(--missing,'')", "if(else:'')"] {
        let value = format!("{FIXED_VALUE} {substitution}");
        assert!(
            declaration::supports_declaration("content", &value),
            "pending substitution assumes the entire property grammar is valid at parse time: {value}"
        );
        let sheet = Stylesheet::parse(&[StylesheetSource {
            text: format!("{CHILD_ELEMENT}::before{{content:{value}}}"),
            base: Default::default(),
        }])
        .unwrap();
        let tree = DocumentTree::parse(DOCUMENT).unwrap();
        let element = tree
            .select(&Selector::parse(CHILD_ELEMENT).unwrap())
            .next()
            .unwrap();
        let result = sheet.pseudo(
            element,
            &Style::default(),
            crate::style::PseudoElement::Before,
        );
        assert!(
            matches!(result, Err(StyleError::Unsupported(_))),
            "acceptance defers capability failure to actual computation instead of dropping the declaration: {value}: {result:?}"
        );
    }
}

#[test]
fn combined_corner_shorthand_grammars_preserve_shape_and_radius_components() {
    for (names, count, valid, invalid) in [
        (
            &[
                "corner-top-left",
                "corner-top-right",
                "corner-bottom-right",
                "corner-bottom-left",
                "corner-start-start",
                "corner-start-end",
                "corner-end-start",
                "corner-end-end",
            ][..],
            2,
            &[
                "bevel 10px 20%",
                "10px 20% notch",
                "superellipse(-0.5)",
                "0",
            ][..],
            &[
                "bevel notch 10px",
                "10px 20px 30px",
                "10px / 20px",
                "-1px bevel",
            ][..],
        ),
        (
            &[
                "corner-top",
                "corner-right",
                "corner-bottom",
                "corner-left",
                "corner-block-start",
                "corner-block-end",
                "corner-inline-start",
                "corner-inline-end",
            ][..],
            4,
            &[
                "round notch 10px 20% / 30px 40%",
                "10px / 20px bevel",
                "scoop square",
                "0",
            ][..],
            &[
                "round notch bevel",
                "1px 2px 3px",
                "1px / 2px 3px 4px",
                "bevel -1px",
            ][..],
        ),
        (
            &["corner"][..],
            8,
            &[
                "round notch bevel square 1px 2px 3px 4px / 5% 6% 7% 8%",
                "1px / 2px squircle",
                "superellipse(infinity)",
                "0",
            ][..],
            &[
                "round notch bevel square scoop",
                "1px 2px 3px 4px 5px",
                "1px / 2px / 3px",
                "bevel -1px",
            ][..],
        ),
    ] {
        for name in names {
            let property = Property::from_name(name)
                .unwrap_or_else(|| panic!("missing CSS Borders4 shorthand {name}"));
            assert_eq!(
                property.longhands().len(),
                count,
                "{name}: every shape and radius longhand must be listed"
            );
            assert!(
                property
                    .longhands()
                    .iter()
                    .all(|longhand| longhand.longhands().is_empty()),
                "{name}: expansion metadata must contain terminal longhands"
            );
            for (values, expected) in [(valid, true), (invalid, false)] {
                for value in values {
                    let matched = crate::style::matcher::match_value(
                        property.data().root,
                        value,
                        &mut 0,
                        crate::style::matcher::ValueContext::Element,
                    )
                    .unwrap();
                    assert_eq!(matched.is_some(), expected, "{name}: {value}");
                }
            }
        }
    }
}

#[test]
fn corner_shape_metadata_follows_physical_and_logical_corner_order() {
    for (name, expected) in [
        (
            "corner-shape",
            &[
                "corner-top-left-shape",
                "corner-top-right-shape",
                "corner-bottom-right-shape",
                "corner-bottom-left-shape",
            ][..],
        ),
        (
            "corner-inline-start-shape",
            &["corner-start-start-shape", "corner-end-start-shape"][..],
        ),
    ] {
        let property = Property::from_name(name).unwrap();
        let actual = property
            .longhands()
            .iter()
            .map(|longhand| longhand.name())
            .collect::<Vec<_>>();
        assert_eq!(actual, expected, "{name}: CSS Borders4 corner order");
    }
}
