use crate::{
    SourceLocation, StyleError,
    cascade::Origin,
    component::Component,
    declaration::{self, PropertyDeclaration, append},
    grammar::Property,
    parser::Declaration,
};
use ferri_pdf_dom::{Element, HTML_NAMESPACE, Node, QuirksMode, TreeRelation};
use ferri_pdf_resources::ResourceBase;
use std::sync::{Arc, OnceLock};

const DEFAULT_TABLE_BORDER_PX: f64 = 1.0;
const LEGACY_COLOR_LIMIT: usize = 128;
const COLOR_COMPONENTS: usize = 3;
const LEGACY_COMPONENT_LIMIT: usize = 8;
const HEX_CHANNEL_DIGITS: usize = 2;
const SHORT_COLOR_DIGITS: usize = 3;
const HEX_DIGIT_EXPANSION: u8 = 17;
const FONT_SIZE_BASE: f64 = 3.0;
const HEADING_DEFAULTS: &[(&str, &str)] = &[
    ("2em", ".67em"),
    ("1.5em", ".83em"),
    ("1.17em", "1em"),
    ("1em", "1.33em"),
    (".83em", "1.67em"),
    (".67em", "2.33em"),
];
const FONT_SIZES: [&str; 7] = [
    "x-small",
    "small",
    "medium",
    "large",
    "x-large",
    "xx-large",
    "xxx-large",
];
const ROW_GROUPS: [&str; 3] = ["thead", "tbody", "tfoot"];
const TABLE_TEXT_ELEMENTS: &[&str] = &["thead", "tbody", "tfoot", "tr", "td", "th"];
const RULES: [&str; 5] = ["none", "groups", "rows", "cols", "all"];
const HR_SOLID_BORDER_RATIO: f64 = 0.5;
const HR_BORDER_EDGES_PX: f64 = 2.0;
const HR_MINIMUM_SIZE_PX: f64 = 1.0;
/// Conditional parent/child UA rules outrank the simple tag defaults.
pub(super) const CONDITIONAL_SPECIFICITY: u32 = 2;
const DEFAULT_MARGIN_ELEMENTS: &[&str] = &[
    "blockquote",
    "dir",
    "dl",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "listing",
    "menu",
    "ol",
    "p",
    "plaintext",
    "pre",
    "ul",
    "xmp",
];

fn list_counter_hint(output: &mut Vec<PropertyDeclaration>, element: Element<'_>) {
    let (attribute, property) = match element.name() {
        "ol" => ("start", Property::CounterReset),
        "li" => ("value", Property::CounterSet),
        _ => return,
    };
    let reversed = element.name() == "ol" && element.attr("reversed").is_some();
    let value = element
        .attr(attribute)
        .and_then(ferri_pdf_dom::integer_digits)
        .map(|(negative, digits)| {
            const DECIMAL_RADIX: i128 = 10;
            let value = digits.bytes().fold(0i128, |value, digit| {
                let value = value.saturating_mul(DECIMAL_RADIX);
                let digit = i128::from(digit - b'0');
                if negative {
                    value.saturating_sub(digit)
                } else {
                    value.saturating_add(digit)
                }
            });
            if property == Property::CounterReset {
                if reversed {
                    value.saturating_add(crate::DEFAULT_COUNTER_INCREMENT)
                } else {
                    value.saturating_sub(crate::DEFAULT_COUNTER_INCREMENT)
                }
            } else {
                value
            }
        });
    if value.is_none() && !reversed {
        return;
    }
    let name = Component::Ident(crate::LIST_ITEM_COUNTER.into());
    let mut parts = vec![if reversed {
        Component::Function(crate::grammar::Keyword::Reversed, Some(Box::new(name)))
    } else {
        name
    }];
    if let Some(value) = value {
        parts.push(Component::HtmlInteger(value));
    }
    declaration::append_longhand(
        output,
        Origin::AuthorHint,
        property,
        declaration::Specified::Value(Arc::new(Component::Group(parts))),
    );
}

fn hr_hints(output: &mut Vec<PropertyDeclaration>, element: Element<'_>) -> Result<(), StyleError> {
    dimension_hint(output, element, "width", Property::Width, false)?;
    color_hint(output, Property::Color, element.attr("color"))?;
    if let Some(align) = element.attr("align") {
        let margins = if align.eq_ignore_ascii_case("left") {
            Some(("0", "auto"))
        } else if align.eq_ignore_ascii_case("right") {
            Some(("auto", "0"))
        } else if align.eq_ignore_ascii_case("center") {
            Some(("auto", "auto"))
        } else {
            None
        };
        if let Some((left, right)) = margins {
            hint(output, Property::MarginLeft, left)?;
            hint(output, Property::MarginRight, right)?;
        }
    }
    let solid = element.attr("color").is_some() || element.attr("noshade").is_some();
    if solid {
        hint(output, Property::BorderStyle, "solid")?;
    }
    if let Some(size) = element.attr("size").and_then(non_negative) {
        if solid {
            pixel_hint(
                output,
                Property::BorderWidth,
                Some(size * HR_SOLID_BORDER_RATIO),
            )?;
        } else if size == HR_MINIMUM_SIZE_PX {
            pixel_hint(output, Property::BorderBottomWidth, Some(0.0))?;
        } else if size > HR_MINIMUM_SIZE_PX {
            pixel_hint(output, Property::Height, Some(size - HR_BORDER_EDGES_PX))?;
        }
    }
    Ok(())
}

fn embedded_hints(
    output: &mut Vec<PropertyDeclaration>,
    element: Element<'_>,
) -> Result<(), StyleError> {
    let tag = element.name();
    let image_button = tag == "input"
        && element
            .attr("type")
            .is_some_and(|kind| kind.eq_ignore_ascii_case("image"));
    if matches!(tag, "embed" | "iframe" | "img" | "object" | "video") || image_button {
        dimension_hint(output, element, "width", Property::Width, false)?;
        dimension_hint(output, element, "height", Property::Height, false)?;
    }
    if (matches!(tag, "embed" | "iframe" | "img" | "object") || image_button)
        && let Some(align) = element.attr("align")
    {
        match align.to_ascii_lowercase().as_str() {
            "left" | "right" => hint(output, Property::Float, &align.to_ascii_lowercase())?,
            "top" | "baseline" | "bottom" => {
                hint(output, Property::VerticalAlign, &align.to_ascii_lowercase())?
            }
            "texttop" => hint(output, Property::VerticalAlign, "text-top")?,
            "absmiddle" | "abscenter" => hint(output, Property::VerticalAlign, "middle")?,
            "center" | "middle" => declaration::append_longhand(
                output,
                Origin::AuthorHint,
                Property::VerticalAlign,
                declaration::Specified::Value(Arc::new(Component::HtmlBaselineMiddle)),
            ),
            _ => {}
        }
    }
    if matches!(tag, "embed" | "img" | "object") || image_button {
        for (attribute, properties) in [
            ("hspace", [Property::MarginLeft, Property::MarginRight]),
            ("vspace", [Property::MarginTop, Property::MarginBottom]),
        ] {
            for property in properties {
                dimension_hint(output, element, attribute, property, false)?;
            }
        }
    }
    if (matches!(tag, "img" | "object") || image_button)
        && let Some(width) = element
            .attr("border")
            .and_then(non_negative)
            .filter(|width| *width > 0.0)
    {
        pixel_hint(output, Property::BorderWidth, Some(width))?;
        hint(output, Property::BorderStyle, "solid")?;
    }
    if tag == "iframe"
        && let Some(value) = element.attr("frameborder")
    {
        let nonzero = ferri_pdf_dom::integer_digits(value)
            .is_some_and(|(_, digits)| digits.bytes().any(|digit| digit != b'0'));
        if !nonzero {
            pixel_hint(output, Property::BorderWidth, Some(0.0))?;
        }
    }
    Ok(())
}

pub(super) fn pseudo_declarations(
    element: Element<'_>,
    pseudo: crate::PseudoElement,
) -> Result<Vec<PropertyDeclaration>, StyleError> {
    let mut output = Vec::new();
    if element.namespace() == HTML_NAMESPACE
        && element.name() == "rt"
        && let Some(ruby) = element.parent(TreeRelation::Formatting)
        && ruby.namespace() == HTML_NAMESPACE
        && ruby.name() == "ruby"
        && !ruby
            .children(TreeRelation::Formatting)
            .any(|child| child.namespace() == HTML_NAMESPACE && child.name() == "rp")
    {
        append(
            &mut output,
            Origin::UserAgent,
            Property::Content,
            match pseudo {
                crate::PseudoElement::Before => "'('",
                crate::PseudoElement::After => "')'",
            },
        )?;
    }
    Ok(output)
}

fn ascii_trim(value: &str) -> &str {
    value.trim_matches(|ch: char| ch.is_ascii_whitespace())
}

fn digits(value: &str) -> usize {
    value.bytes().take_while(u8::is_ascii_digit).count()
}

fn bounded_number(value: &str) -> Option<f64> {
    value
        .parse::<f64>()
        .ok()
        .map(|number| number.min(f64::from(f32::MAX)))
}

/// HTML rules for parsing non-negative integers, including a sign and trailing garbage.
pub(super) fn non_negative(value: &str) -> Option<f64> {
    bounded_number(ferri_pdf_dom::non_negative_integer_digits(value)?)
}

/// HTML dimension parsing accepts a decimal prefix and an immediately following percent.
fn dimension(value: &str) -> Option<(f64, bool)> {
    let value = value.trim_start_matches(|ch: char| ch.is_ascii_whitespace());
    let mut end = digits(value);
    if end == 0 {
        return None;
    }
    if value.as_bytes().get(end) == Some(&b'.') {
        end += 1;
        end += digits(&value[end..]);
    }
    Some((
        bounded_number(&value[..end])?,
        value.as_bytes().get(end) == Some(&b'%'),
    ))
}

fn hint(
    output: &mut Vec<PropertyDeclaration>,
    property: Property,
    value: &str,
) -> Result<(), StyleError> {
    append(output, Origin::AuthorHint, property, value)
}

fn pixel_hint(
    output: &mut Vec<PropertyDeclaration>,
    property: Property,
    value: Option<f64>,
) -> Result<(), StyleError> {
    if let Some(value) = value {
        hint(output, property, &format!("{value}px"))?;
    }
    Ok(())
}

fn dimension_hint(
    output: &mut Vec<PropertyDeclaration>,
    element: Element<'_>,
    attribute: &str,
    property: Property,
    ignore_zero: bool,
) -> Result<(), StyleError> {
    if let Some((number, percentage)) = element.attr(attribute).and_then(dimension)
        && (!ignore_zero || number != 0.0)
    {
        hint(
            output,
            property,
            &format!("{number}{}", if percentage { "%" } else { "px" }),
        )?;
    }
    Ok(())
}

fn color(value: &str) -> Option<String> {
    if value.is_empty() {
        return None;
    }
    let value = ascii_trim(value);
    if value.eq_ignore_ascii_case("transparent") {
        return None;
    }
    if value.chars().all(|ch| ch.is_ascii_alphabetic())
        && let Ok(parsed) = crate::colors::parse(value, &crate::units::Relative::initial())
        && let cssparser_color::Color::Rgba(color) = parsed.color
    {
        return Some(format!(
            "#{:02x}{:02x}{:02x}",
            color.red, color.green, color.blue
        ));
    }
    if let Some(short) = value.strip_prefix('#').filter(|value| {
        value.len() == SHORT_COLOR_DIGITS && value.bytes().all(|byte| byte.is_ascii_hexdigit())
    }) {
        let channels: Vec<_> = short
            .bytes()
            .map(|byte| (byte as char).to_digit(16).expect("hex digit") as u8 * HEX_DIGIT_EXPANSION)
            .collect();
        return Some(format!(
            "#{:02x}{:02x}{:02x}",
            channels[0], channels[1], channels[2]
        ));
    }
    let expanded: String = value
        .chars()
        .flat_map(|ch| {
            if u32::from(ch) > u32::from(u16::MAX) {
                vec!['0', '0']
            } else {
                vec![ch]
            }
        })
        .take(LEGACY_COLOR_LIMIT)
        .collect();
    let input = expanded.strip_prefix('#').unwrap_or(&expanded);
    let mut input: Vec<_> = input
        .chars()
        .map(|ch| {
            if ch.is_ascii_hexdigit() {
                ch as u8
            } else {
                b'0'
            }
        })
        .collect();
    while input.is_empty() || !input.len().is_multiple_of(COLOR_COMPONENTS) {
        input.push(b'0');
    }
    let length = input.len() / COLOR_COMPONENTS;
    let mut channels: Vec<&[u8]> = input
        .chunks_exact(length)
        .map(|channel| &channel[length.saturating_sub(LEGACY_COMPONENT_LIMIT)..])
        .collect();
    while channels[0].len() > HEX_CHANNEL_DIGITS
        && channels.iter().all(|channel| channel[0] == b'0')
    {
        channels
            .iter_mut()
            .for_each(|channel| *channel = &channel[1..]);
    }
    let mut output = String::from("#");
    for channel in channels {
        let number = channel
            .iter()
            .take(HEX_CHANNEL_DIGITS)
            .fold(0u8, |value, byte| {
                value * 16 + (*byte as char).to_digit(16).expect("hex digit") as u8
            });
        use std::fmt::Write;
        write!(output, "{number:02x}").expect("writing a String cannot fail");
    }
    Some(output)
}

fn color_hint(
    output: &mut Vec<PropertyDeclaration>,
    property: Property,
    value: Option<&str>,
) -> Result<(), StyleError> {
    if let Some(value) = value.and_then(color) {
        hint(output, property, &value)?;
    }
    Ok(())
}

fn background_hint(
    output: &mut Vec<PropertyDeclaration>,
    element: Element<'_>,
    base: &ResourceBase,
) -> Result<(), StyleError> {
    if let Some(value) = element.attr("background").filter(|value| !value.is_empty()) {
        let url = match base.resolve(value) {
            Ok(url) => url,
            Err(ferri_pdf_resources::ResourceError::Url { .. }) => return Ok(()),
            Err(error) => return Err(error.into()),
        };
        let mut quoted = String::new();
        cssparser::serialize_string(url.as_str(), &mut quoted)
            .expect("writing a String cannot fail");
        let declaration = Declaration {
            name: Property::BackgroundImage.name().into(),
            value: format!("url({quoted})"),
            important: false,
            namespaces: None,
            location: SourceLocation {
                base: base.clone(),
                line: 1,
                column: 1,
            },
        };
        declaration::parse_at_origin(&declaration, Origin::AuthorHint, output)?;
    }
    Ok(())
}

fn cell_table(element: Element<'_>) -> Option<Element<'_>> {
    let row = element
        .parent(TreeRelation::Selector)
        .filter(|parent| parent.name() == "tr")?;
    row_table(row)
}

fn row_table(element: Element<'_>) -> Option<Element<'_>> {
    let parent = element.parent(TreeRelation::Selector)?;
    let table = if ROW_GROUPS.contains(&parent.name()) {
        parent.parent(TreeRelation::Selector)?
    } else {
        parent
    };
    (table.name() == "table").then_some(table)
}

fn border(table: Element<'_>) -> Option<f64> {
    table
        .attr("border")
        .map(|value| non_negative(value).unwrap_or(DEFAULT_TABLE_BORDER_PX))
}

fn table_hints(
    output: &mut Vec<PropertyDeclaration>,
    element: Element<'_>,
) -> Result<(), StyleError> {
    let tag = element.name();
    if tag == "table" {
        pixel_hint(
            output,
            Property::BorderSpacing,
            element.attr("cellspacing").and_then(non_negative),
        )?;
        if element
            .attr("rules")
            .is_some_and(|value| RULES.iter().any(|known| value.eq_ignore_ascii_case(known)))
        {
            hint(output, Property::BorderStyle, "hidden")?;
            hint(output, Property::BorderCollapse, "collapse")?;
        }
        if let Some(width) = border(element) {
            pixel_hint(output, Property::BorderWidth, Some(width))?;
            if width > 0.0 {
                hint(output, Property::BorderStyle, "outset")?;
            }
        }
        if let Some(frame) = element.attr("frame") {
            let styles = match frame.to_ascii_lowercase().as_str() {
                "void" => Some("hidden"),
                "above" => Some("outset hidden hidden hidden"),
                "below" => Some("hidden hidden outset hidden"),
                "hsides" => Some("outset hidden"),
                "lhs" => Some("hidden hidden hidden outset"),
                "rhs" => Some("hidden outset hidden hidden"),
                "vsides" => Some("hidden outset"),
                "box" | "border" => Some("outset"),
                _ => None,
            };
            if let Some(styles) = styles {
                hint(output, Property::BorderStyle, styles)?;
            }
        }
        if element
            .attr("rules")
            .is_some_and(|rules| RULES.iter().any(|known| rules.eq_ignore_ascii_case(known)))
            || element.attr("frame").is_some_and(|frame| {
                [
                    "void", "above", "below", "hsides", "lhs", "rhs", "vsides", "box", "border",
                ]
                .iter()
                .any(|known| frame.eq_ignore_ascii_case(known))
            })
        {
            append(output, Origin::UserAgent, Property::BorderColor, "black")?;
        }
        color_hint(output, Property::BorderColor, element.attr("bordercolor"))?;
        if let Some(align) = element.attr("align") {
            match align.to_ascii_lowercase().as_str() {
                "center" => {
                    hint(output, Property::MarginLeft, "auto")?;
                    hint(output, Property::MarginRight, "auto")?;
                }
                "left" | "right" => hint(output, Property::Float, &align.to_ascii_lowercase())?,
                _ => {}
            }
        }
    }
    if matches!(tag, "td" | "th")
        && let Some(table) = cell_table(element)
    {
        pixel_hint(
            output,
            Property::Padding,
            table.attr("cellpadding").and_then(non_negative),
        )?;
        if border(table).is_some_and(|width| width > 0.0) {
            hint(output, Property::BorderWidth, "1px")?;
            hint(output, Property::BorderStyle, "inset")?;
        }
        if let Some(rules) = table
            .attr("rules")
            .map(str::to_ascii_lowercase)
            .filter(|value| RULES.contains(&value.as_str()))
        {
            hint(output, Property::BorderWidth, "1px")?;
            hint(
                output,
                Property::BorderStyle,
                match rules.as_str() {
                    "cols" => "none solid",
                    "all" => "solid",
                    _ => "none",
                },
            )?;
            append(output, Origin::UserAgent, Property::BorderColor, "black")?;
        }
    }
    if tag == "tr"
        && let Some(table) = row_table(element)
        && table
            .attr("rules")
            .is_some_and(|value| value.eq_ignore_ascii_case("rows"))
    {
        hint(output, Property::BorderTop, "1px solid")?;
        hint(output, Property::BorderBottom, "1px solid")?;
    }
    if (ROW_GROUPS.contains(&tag) || tag == "colgroup")
        && let Some(table) = element
            .parent(TreeRelation::Selector)
            .filter(|parent| parent.name() == "table")
        && table
            .attr("rules")
            .is_some_and(|value| value.eq_ignore_ascii_case("groups"))
    {
        let properties = if tag == "colgroup" {
            [Property::BorderLeft, Property::BorderRight]
        } else {
            [Property::BorderTop, Property::BorderBottom]
        };
        for property in properties {
            hint(output, property, "1px solid")?;
        }
    }
    Ok(())
}

fn substantial(node: Node<'_>) -> bool {
    node.element().is_some()
        || node
            .value()
            .as_text()
            .is_some_and(|text| !ascii_trim(text).is_empty())
}

fn quirks_margins(
    output: &mut Vec<PropertyDeclaration>,
    element: Element<'_>,
) -> Result<(), StyleError> {
    if !DEFAULT_MARGIN_ELEMENTS.contains(&element.name()) {
        return Ok(());
    }
    let Some(parent) = element.parent(TreeRelation::Selector).filter(|parent| {
        parent.namespace() == HTML_NAMESPACE && matches!(parent.name(), "body" | "td" | "th")
    }) else {
        return Ok(());
    };
    let current = element.as_node();
    let first = !parent
        .child_nodes(TreeRelation::Selector)
        .take_while(|node| *node != current)
        .any(substantial);
    let last_in_cell = parent.name() != "body"
        && !parent
            .child_nodes(TreeRelation::Selector)
            .rev()
            .take_while(|node| *node != current)
            .any(substantial);
    let blank = !element.child_nodes(TreeRelation::Selector).any(substantial);
    if first || (last_in_cell && blank) {
        append(output, Origin::UserAgent, Property::MarginTop, "0")?;
    }
    if (first && blank) || (last_in_cell && element.name() == "p") {
        append(output, Origin::UserAgent, Property::MarginBottom, "0")?;
    }
    Ok(())
}

fn quirks(output: &mut Vec<PropertyDeclaration>, element: Element<'_>) -> Result<(), StyleError> {
    if element.quirks_mode() != QuirksMode::Quirks {
        return Ok(());
    }
    quirks_margins(output, element)?;
    let tag = element.name();
    if tag == "img"
        && let Some(align) = element.attr("align")
    {
        let side = if align.eq_ignore_ascii_case("left") {
            Some(Property::MarginRight)
        } else if align.eq_ignore_ascii_case("right") {
            Some(Property::MarginLeft)
        } else {
            None
        };
        if let Some(property) = side {
            append(output, Origin::UserAgent, property, "3px")?;
        }
    }
    if tag == "form" {
        append(output, Origin::UserAgent, Property::MarginBottom, "1em")?;
    }
    if tag == "table" {
        for property in [
            Property::FontWeight,
            Property::FontStyle,
            Property::FontVariant,
            Property::FontSize,
            Property::LineHeight,
            Property::WhiteSpace,
            Property::TextAlign,
        ] {
            append(output, Origin::UserAgent, property, "initial")?;
        }
    }
    let list = |name| matches!(name, "dir" | "menu" | "ol" | "ul");
    if tag == "li" || list(tag) {
        let nested = element
            .ancestors(TreeRelation::Selector)
            .any(|ancestor| ancestor.namespace() == HTML_NAMESPACE && list(ancestor.name()));
        let value = if nested {
            Some("unset")
        } else if tag == "li" {
            Some("inside")
        } else if element
            .ancestors(TreeRelation::Selector)
            .any(|ancestor| ancestor.namespace() == HTML_NAMESPACE && ancestor.name() == "li")
        {
            Some("outside")
        } else {
            None
        };
        if let Some(value) = value {
            append(
                output,
                Origin::UserAgent,
                Property::ListStylePosition,
                value,
            )?;
        }
    }
    if matches!(tag, "td" | "th")
        && element.attr("nowrap").is_some()
        && element
            .attr("width")
            .and_then(dimension)
            .is_some_and(|(width, percent)| width > 0.0 && !percent)
    {
        hint(output, Property::WhiteSpace, "normal")?;
    }
    Ok(())
}

pub(super) struct DefaultDeclaration {
    pub declaration: PropertyDeclaration,
    pub specificity: u32,
}

pub(super) fn declarations(
    element: Element<'_>,
    base: &ResourceBase,
) -> Result<Vec<DefaultDeclaration>, StyleError> {
    let mut output = Vec::new();
    if element.namespace() != HTML_NAMESPACE {
        return Ok(Vec::new());
    }
    let tag = element.name();
    embedded_hints(&mut output, element)?;
    list_counter_hint(&mut output, element);
    if let Some(level) = element.heading_level() {
        let (size, margin) = HEADING_DEFAULTS[usize::from(level).min(HEADING_DEFAULTS.len()) - 1];
        for (property, value) in [
            (Property::FontSize, size),
            (Property::MarginTop, margin),
            (Property::MarginBottom, margin),
        ] {
            append(&mut output, Origin::UserAgent, property, value)?;
        }
        append(
            &mut output,
            Origin::UserAgent,
            Property::BookmarkLevel,
            &level.to_string(),
        )?;
    }
    if tag == "hr" {
        hr_hints(&mut output, element)?;
    }
    if tag == "br"
        && let Some(clear) = element.attr("clear")
    {
        let value = if clear.eq_ignore_ascii_case("left") {
            Some("left")
        } else if clear.eq_ignore_ascii_case("right") {
            Some("right")
        } else if clear.eq_ignore_ascii_case("all") || clear.eq_ignore_ascii_case("both") {
            Some("both")
        } else {
            None
        };
        if let Some(value) = value {
            hint(&mut output, Property::Clear, value)?;
        }
    }
    match tag {
        "table" | "td" | "th" => {
            dimension_hint(&mut output, element, "width", Property::Width, true)?;
            dimension_hint(
                &mut output,
                element,
                "height",
                Property::Height,
                tag != "table",
            )?;
        }
        "col" => dimension_hint(&mut output, element, "width", Property::Width, false)?,
        "thead" | "tbody" | "tfoot" | "tr" => {
            dimension_hint(&mut output, element, "height", Property::Height, false)?
        }
        _ => {}
    }
    if tag == "body" {
        for (attributes, properties) in [
            (
                ["marginheight", "topmargin"],
                [Property::MarginTop, Property::MarginBottom],
            ),
            (
                ["marginwidth", "leftmargin"],
                [Property::MarginLeft, Property::MarginRight],
            ),
        ] {
            let value = attributes
                .iter()
                .find_map(|name| element.attr(name))
                .and_then(non_negative);
            for property in properties {
                pixel_hint(&mut output, property, value)?;
            }
        }
        color_hint(&mut output, Property::Color, element.attr("text"))?;
    }
    if matches!(
        tag,
        "body" | "table" | "thead" | "tbody" | "tfoot" | "tr" | "td" | "th"
    ) {
        background_hint(&mut output, element, base)?;
        color_hint(
            &mut output,
            Property::BackgroundColor,
            element.attr("bgcolor"),
        )?;
    }
    if tag == "a"
        && element.attr("href").is_some()
        && let Some(body) = element
            .ancestors(TreeRelation::Selector)
            .find(|ancestor| ancestor.name() == "body" && ancestor.namespace() == HTML_NAMESPACE)
    {
        color_hint(&mut output, Property::Color, body.attr("link"))?;
    }
    if matches!(
        tag,
        "div"
            | "p"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "thead"
            | "tbody"
            | "tfoot"
            | "tr"
            | "td"
            | "th"
    ) && let Some(align) = element.attr("align")
    {
        match align.to_ascii_lowercase().as_str() {
            "center" => hint(&mut output, Property::TextAlign, "center")?,
            "middle" if tag == "div" || TABLE_TEXT_ELEMENTS.contains(&tag) => {
                hint(&mut output, Property::TextAlign, "center")?;
            }
            "absmiddle" if TABLE_TEXT_ELEMENTS.contains(&tag) => {
                hint(&mut output, Property::TextAlign, "center")?;
            }
            "left" | "right" | "justify" => hint(
                &mut output,
                Property::TextAlign,
                &align.to_ascii_lowercase(),
            )?,
            _ => {}
        }
    }
    if tag == "center" {
        hint(&mut output, Property::TextAlign, "center")?;
    }
    if TABLE_TEXT_ELEMENTS.contains(&tag) {
        if let Some(value) = element.attr("valign")
            && ["top", "middle", "bottom", "baseline"]
                .iter()
                .any(|known| value.eq_ignore_ascii_case(known))
        {
            hint(
                &mut output,
                Property::VerticalAlign,
                &value.to_ascii_lowercase(),
            )?;
        }
        if matches!(tag, "td" | "th") && element.attr("nowrap").is_some() {
            hint(&mut output, Property::WhiteSpace, "nowrap")?;
        }
    }
    if tag == "caption"
        && element
            .attr("align")
            .is_some_and(|value| value.eq_ignore_ascii_case("bottom"))
    {
        hint(&mut output, Property::CaptionSide, "bottom")?;
    }
    if tag == "pre" && element.attr("wrap").is_some() {
        hint(&mut output, Property::WhiteSpace, "pre-wrap")?;
    }
    if matches!(tag, "ol" | "ul" | "li")
        && let Some(value) = element.attr("type")
    {
        let style = match value {
            "1" if tag != "ul" => Some("decimal"),
            "a" if tag != "ul" => Some("lower-alpha"),
            "A" if tag != "ul" => Some("upper-alpha"),
            "i" if tag != "ul" => Some("lower-roman"),
            "I" if tag != "ul" => Some("upper-roman"),
            _ if tag != "ol" => ["none", "disc", "circle", "square"]
                .into_iter()
                .find(|known| value.eq_ignore_ascii_case(known)),
            _ => None,
        };
        if let Some(style) = style {
            hint(&mut output, Property::ListStyleType, style)?;
        }
    }
    if tag == "font" {
        color_hint(&mut output, Property::Color, element.attr("color"))?;
        if let Some(face) = element.attr("face") {
            hint(&mut output, Property::FontFamily, face)?;
        }
        if let Some(size) = element.attr("size") {
            let size = size.trim_start_matches(|ch: char| ch.is_ascii_whitespace());
            let (mode, value) = if let Some(value) = size.strip_prefix('+') {
                (1, value)
            } else if let Some(value) = size.strip_prefix('-') {
                (-1, value)
            } else {
                (0, size)
            };
            let count = digits(value);
            if count > 0
                && let Some(number) = bounded_number(&value[..count])
            {
                let number = match mode {
                    1 => FONT_SIZE_BASE + number,
                    -1 => FONT_SIZE_BASE - number,
                    _ => number,
                };
                let index = number.clamp(1.0, FONT_SIZES.len() as f64) as usize - 1;
                hint(&mut output, Property::FontSize, FONT_SIZES[index])?;
            }
        }
    }
    table_hints(&mut output, element)?;
    quirks(&mut output, element)?;
    let bidi_rule = if element.input_type() == Some(ferri_pdf_dom::InputType::Text)
        && element
            .attr("dir")
            .is_some_and(|value| value.eq_ignore_ascii_case("auto"))
    {
        let index = output.len();
        append(
            &mut output,
            Origin::UserAgent,
            Property::UnicodeBidi,
            "plaintext",
        )?;
        static SPECIFICITY: OnceLock<u32> = OnceLock::new();
        let specificity = *SPECIFICITY.get_or_init(|| {
            ferri_pdf_dom::Selector::parse("input[dir=auto i]")
                .expect("the static UA input selector is valid")
                .single_specificity()
                .expect("one UA selector")
        });
        Some((index, specificity))
    } else {
        None
    };
    Ok(output
        .into_iter()
        .enumerate()
        .map(|(index, declaration)| DefaultDeclaration {
            declaration,
            specificity: match bidi_rule {
                Some((at, specificity)) if at == index => specificity,
                _ => CONDITIONAL_SPECIFICITY,
            },
        })
        .collect())
}
