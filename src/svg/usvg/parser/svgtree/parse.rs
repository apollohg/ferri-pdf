// Copyright 2021 the Resvg Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use std::collections::HashMap;

use super::css::{Declaration, StyleSheet};
use crate::style::PseudoElement;
use roxmltree::Error;
use svgtypes::FontShorthand;

use super::{AId, Attribute, Document, EId, NodeData, NodeId, NodeKind, ShortRange};

const SVG_NS: &str = "http://www.w3.org/2000/svg";
const XLINK_NS: &str = "http://www.w3.org/1999/xlink";
const XML_NAMESPACE_NS: &str = "http://www.w3.org/XML/1998/namespace";
const MAX_SVG_DEPTH: u32 = 1024;
const MAX_SVG_NODES: usize = 1_000_000;

impl<'input> Document<'input> {
    /// Parses a [`Document`] from a [`roxmltree::Document`].
    pub(crate) fn parse_tree(
        xml: &roxmltree::Document<'input>,
        injected_stylesheet: Option<&'input str>,
    ) -> Result<Document<'input>, Error> {
        parse(xml, injected_stylesheet)
    }

    pub(crate) fn append(&mut self, parent_id: NodeId, kind: NodeKind) -> NodeId {
        let new_child_id = NodeId::from(self.nodes.len());
        self.nodes.push(NodeData {
            effective_zoom: crate::style::INITIAL_ZOOM,
            parent: Some(parent_id),
            next_sibling: None,
            children: None,
            kind,
        });

        let last_child_id = self.nodes[parent_id.get_usize()].children.map(|(_, id)| id);

        if let Some(id) = last_child_id {
            self.nodes[id.get_usize()].next_sibling = Some(new_child_id);
        }

        self.nodes[parent_id.get_usize()].children = Some(
            if let Some((first_child_id, _)) = self.nodes[parent_id.get_usize()].children {
                (first_child_id, new_child_id)
            } else {
                (new_child_id, new_child_id)
            },
        );

        new_child_id
    }

    fn append_attribute(
        &mut self,
        name: AId,
        value: roxmltree::StringStorage<'input>,
        important: bool,
    ) {
        self.attrs.push(Attribute {
            name,
            value,
            important,
            inherited: false,
        });
    }
}

fn parse<'input>(
    xml: &roxmltree::Document<'input>,
    injected_stylesheet: Option<&'input str>,
) -> Result<Document<'input>, Error> {
    let mut doc = Document {
        nodes: Vec::new(),
        variables: HashMap::new(),
        declared: HashMap::new(),
        first_lines: HashMap::new(),
        style_error: None,
        attrs: Vec::new(),
        links: HashMap::new(),
        css_limit: None,
    };

    // build a map of id -> node for resolve_href
    let mut id_map = HashMap::new();
    for node in xml.descendants() {
        if let Some(id) = node.attribute("id") {
            if !id_map.contains_key(id) {
                id_map.insert(id, node);
            }
        }
    }

    // Add a root node.
    doc.nodes.push(NodeData {
        effective_zoom: crate::style::INITIAL_ZOOM,
        parent: None,
        next_sibling: None,
        children: None,
        kind: NodeKind::Root,
    });

    let style_sheet = resolve_css(xml, injected_stylesheet);
    doc.css_limit = style_sheet.limit;

    parse_xml_node_children(
        xml.root(),
        xml.root(),
        doc.root().id,
        &style_sheet,
        false,
        0,
        &mut doc,
        &id_map,
    )?;

    // Check that the root element is `svg`.
    match doc.root().first_element_child() {
        Some(child) => {
            if child.tag_name() != Some(EId::Svg) {
                return Err(Error::NoRootNode);
            }
        }
        None => return Err(Error::NoRootNode),
    }

    // Collect all elements with `id` attribute.
    let mut links = HashMap::new();
    for node in doc.descendants() {
        if let Some(id) = node.attribute::<&str>(AId::Id) {
            links.insert(id.to_string(), node.id);
        }
    }
    doc.links = links;

    fix_recursions(&mut doc);
    if let Err(error) = resolve_zoom(&mut doc) {
        doc.style_error
            .get_or_insert_with(|| std::sync::Arc::new(error));
    }

    Ok(doc)
}

fn resolve_zoom(doc: &mut Document<'_>) -> Result<(), crate::style::StyleError> {
    // Nodes are appended after their parents, including expanded use instances.
    for index in 1..doc.nodes.len() {
        let parent = doc.nodes[index]
            .parent
            .map_or(crate::style::INITIAL_ZOOM, |parent| {
                doc.nodes[parent.get_usize()].effective_zoom
            });
        let own = doc
            .get(index.into())
            .attribute::<f32>(AId::CssZoom)
            .unwrap_or(crate::style::INITIAL_ZOOM);
        doc.nodes[index].effective_zoom = crate::style::accumulate_zoom(parent, own)?;
    }
    Ok(())
}

pub(crate) fn parse_tag_name(node: roxmltree::Node) -> Option<EId> {
    if !node.is_element() {
        return None;
    }

    if !matches!(node.tag_name().namespace(), None | Some(SVG_NS)) {
        return None;
    }

    EId::from_str(node.tag_name().name())
}

fn parse_xml_node_children<'input>(
    parent: roxmltree::Node<'_, 'input>,
    origin: roxmltree::Node,
    parent_id: NodeId,
    style_sheet: &StyleSheet,
    ignore_ids: bool,
    depth: u32,
    doc: &mut Document<'input>,
    id_map: &HashMap<&str, roxmltree::Node<'_, 'input>>,
) -> Result<(), Error> {
    for node in parent.children() {
        parse_xml_node(
            node,
            origin,
            parent_id,
            style_sheet,
            ignore_ids,
            depth,
            doc,
            id_map,
        )?;
    }

    Ok(())
}

fn parse_xml_node<'input>(
    node: roxmltree::Node<'_, 'input>,
    origin: roxmltree::Node,
    parent_id: NodeId,
    style_sheet: &StyleSheet,
    ignore_ids: bool,
    depth: u32,
    doc: &mut Document<'input>,
    id_map: &HashMap<&str, roxmltree::Node<'_, 'input>>,
) -> Result<(), Error> {
    if depth > MAX_SVG_DEPTH {
        return Err(Error::NodesLimitReached);
    }

    let mut tag_name = match parse_tag_name(node) {
        Some(id) => id,
        None => return Ok(()),
    };

    if tag_name == EId::Style {
        return Ok(());
    }

    // TODO: remove?
    // Treat links as groups.
    if tag_name == EId::A {
        tag_name = EId::G;
    }

    let referenced = (tag_name == EId::Use)
        .then(|| resolve_svg_use(node, origin, id_map))
        .flatten();
    let host_instance = match referenced {
        Some(link) => style_sheet.create_instance(link, node, MAX_SVG_NODES)?,
        None => None,
    };
    let node_id = parse_svg_element(
        node,
        parent_id,
        tag_name,
        style_sheet,
        Source::Element { ignore_ids },
        host_instance,
        doc,
    )?;
    if tag_name == EId::Text {
        super::text::parse_svg_text_element(node, node_id, style_sheet, doc)?;
    } else if tag_name == EId::Use {
        if let Some(link) = referenced {
            let _context = style_sheet.enter_instance(host_instance);
            parse_xml_node(
                link,
                node,
                node_id,
                style_sheet,
                true,
                depth + 2,
                doc,
                id_map,
            )?;
        }
    } else {
        parse_xml_node_children(
            node,
            origin,
            node_id,
            style_sheet,
            ignore_ids,
            depth + 1,
            doc,
            id_map,
        )?;
    }

    Ok(())
}

/// What an element node is parsed from.
#[derive(Clone, Copy)]
pub(crate) enum Source {
    /// The XML element; a `use` instance ignores its `id`.
    Element { ignore_ids: bool },
    /// A typographic pseudo-element of the XML element, which only its rules style.
    Pseudo(PseudoElement),
}

/// The properties a typographic pseudo-element of an SVG `text` element ignores: `display`, as
/// its box is inline; white space processing, which the text undergoes before layout finds
/// its lines (SVG 2 §11.5); and those CSS Pseudo-Elements 4 §2.1.2 excludes from
/// `::first-line`.
fn ignored_by(pseudo: PseudoElement, property: &str) -> bool {
    const IGNORED: [&str; 6] = [
        "display",
        "white-space",
        "white-space-collapse",
        "white-space-trim",
        "text-wrap",
        "text-wrap-mode",
    ];
    const FIRST_LINE_EXCLUDED: [&str; 5] = [
        "writing-mode",
        "direction",
        "text-orientation",
        "glyph-orientation-vertical",
        "glyph-orientation-horizontal",
    ];
    IGNORED.contains(&property)
        || (pseudo == PseudoElement::FirstLine && FIRST_LINE_EXCLUDED.contains(&property))
}

pub(crate) fn parse_svg_element<'input>(
    xml_node: roxmltree::Node<'_, 'input>,
    parent_id: NodeId,
    tag_name: EId,
    style_sheet: &StyleSheet,
    source: Source,
    host_instance: Option<crate::dom::TreeScopeId>,
    doc: &mut Document<'input>,
) -> Result<NodeId, Error> {
    let attrs_start_idx = doc.attrs.len();
    let (ignore_ids, pseudo) = match source {
        Source::Element { ignore_ids } => (ignore_ids, None),
        Source::Pseudo(pseudo) => (true, Some(pseudo)),
    };

    // Copy presentational attributes first; a pseudo-element has none.
    for attr in xml_node.attributes().filter(|_| pseudo.is_none()) {
        match attr.namespace() {
            None | Some(SVG_NS) | Some(XLINK_NS) | Some(XML_NAMESPACE_NS) => {}
            _ => continue,
        }

        let aid = match AId::from_str(attr.name()) {
            Some(v) => v,
            None => continue,
        };

        // During a `use` resolving, all `id` attributes must be ignored.
        // Otherwise we will get elements with duplicated id's.
        if ignore_ids && aid == AId::Id {
            continue;
        }

        // For some reason those properties are allowed only inside a `style` attribute and CSS.
        if matches!(aid, AId::MixBlendMode | AId::Isolation | AId::FontKerning) {
            continue;
        } else if aid == AId::ImageRendering
            && matches!(
                attr.value(),
                "smooth" | "high-quality" | "crisp-edges" | "pixelated"
            )
        {
            continue;
        }

        let normalized;
        let (aid, value) = if is_writing_attribute(aid) {
            let Some((mapped, mut value)) =
                writing_attribute(aid, attr.value()).or_else(|| writing_attribute(aid, "initial"))
            else {
                continue;
            };
            if matches!(value.as_str(), "revert" | "revert-layer") {
                value = "inherit".into();
            }
            normalized = roxmltree::StringStorage::new_owned(&value);
            (mapped, normalized)
        } else {
            (aid, attr.value_storage().clone())
        };
        append_attribute(parent_id, tag_name, aid, value, false, doc);
    }

    let presentation_end_idx = doc.attrs.len();
    let inline_declarations =
        if let Some(value) = xml_node.attribute("style").filter(|_| pseudo.is_none()) {
            let mut input = cssparser::ParserInput::new(value);
            let input = &mut cssparser::Parser::new(&mut input);
            match super::css::declarations(input) {
                Ok(declarations) => declarations,
                Err(limit) => {
                    doc.css_limit = Some(limit);
                    Vec::new()
                }
            }
        } else {
            Vec::new()
        };

    let host = || {
        host_instance
            .into_iter()
            .flat_map(|scope| style_sheet.matching_declarations(xml_node, Some(scope)))
    };
    let declarations: Vec<(&Declaration, bool, bool)> = if let Some(pseudo) = pseudo {
        style_sheet
            .pseudo_declarations(xml_node, pseudo)
            .filter(|decl| !ignored_by(pseudo, &decl.name))
            .map(|decl| (decl, true, false))
            .collect()
    } else {
        let mut declarations: Vec<_> = host()
            .filter(|decl| !decl.important)
            .map(|decl| (decl, false, false))
            .collect();
        declarations.extend(
            style_sheet
                .matching_declarations(xml_node, None)
                .map(|decl| (decl, true, false)),
        );
        declarations.extend(inline_declarations.iter().map(|decl| (decl, true, true)));
        declarations.extend(
            host()
                .filter(|decl| decl.important)
                .map(|decl| (decl, true, false)),
        );
        declarations
    };
    let mut custom: HashMap<String, (String, bool, bool)> = HashMap::new();
    let mut custom_baseline = None;
    let mut custom_previous = HashMap::new();
    for (declaration, _, inline) in &declarations {
        if *inline && custom_baseline.is_none() {
            custom_baseline = Some(custom.clone());
        }
        if !declaration.name.starts_with("--") {
            continue;
        }
        match crate::style::svg_cascade::valid_custom(declaration.value, declaration.substitutions)
        {
            Ok(true) => {}
            Ok(false) => continue,
            Err(error) => {
                doc.style_error.get_or_insert(std::sync::Arc::new(error));
                continue;
            }
        }
        if custom
            .get(&*declaration.name)
            .is_some_and(|(_, important, _)| *important && !declaration.important)
        {
            continue;
        }
        let previous = custom_baseline
            .as_ref()
            .filter(|_| *inline)
            .and_then(|baseline| baseline.get(&*declaration.name));
        custom_previous.remove(&*declaration.name);
        if let Some((value, _, substitutions)) = previous {
            custom_previous.insert(
                declaration.name.to_string(),
                (value.clone(), *substitutions),
            );
        }
        let (value, substitutions) =
            if crate::style::svg_cascade::WideKeyword::parse(declaration.value)
                == Some(crate::style::svg_cascade::WideKeyword::RevertLayer)
            {
                previous.map_or_else(
                    || ("inherit".to_owned(), false),
                    |(value, _, substitutions)| (value.clone(), *substitutions),
                )
            } else {
                (declaration.value.to_owned(), declaration.substitutions)
            };
        custom.insert(
            declaration.name.to_string(),
            (value, declaration.important, substitutions),
        );
    }
    let inherited = doc
        .variables
        .get(&parent_id.get_usize())
        .cloned()
        .unwrap_or_default();
    let variables = match inherited.child(
        custom
            .into_iter()
            .map(|(name, (value, _, substitutions))| (name, (value, substitutions)))
            .collect(),
        custom_previous,
    ) {
        Ok(variables) => variables,
        Err(error) => {
            doc.style_error.get_or_insert(std::sync::Arc::new(error));
            inherited
        }
    };

    let legacy_writing_default = xml_node
        .attribute("writing-mode")
        .filter(|_| pseudo.is_none())
        .filter(|value| matches!(*value, "lr" | "lr-tb" | "rl" | "rl-tb" | "tb" | "tb-rl"))
        .and_then(|value| writing_attribute(AId::WritingMode, value).map(|(_, value)| value));
    let inline_phase = std::cell::Cell::new(false);
    let presentation_baseline: HashMap<_, _> = doc.attrs[attrs_start_idx..presentation_end_idx]
        .iter()
        .map(|attr| (attr.name.to_str(), attr.value.to_string()))
        .collect();
    let mut inline_baseline = HashMap::<&str, Option<String>>::new();
    let mut insert_attribute =
        |aid: AId, value: &str, important: bool, override_presentation: bool| {
            use crate::style::svg_cascade::WideKeyword;
            let normalized;
            let (aid, value) = if is_writing_attribute(aid) {
                let Some((mapped, value)) = writing_attribute(aid, value) else {
                    return;
                };
                normalized = value;
                (mapped, normalized.as_str())
            } else {
                (aid, value)
            };
            if inline_phase.get() {
                inline_baseline.entry(aid.to_str()).or_insert_with(|| {
                    doc.attrs[attrs_start_idx..]
                        .iter()
                        .find(|attr| attr.name == aid)
                        .map(|attr| attr.value.to_string())
                });
            }
            let fallback = if aid == AId::WritingMode {
                legacy_writing_default.as_deref().unwrap_or("unset")
            } else {
                "unset"
            };
            let value = match WideKeyword::parse(value) {
                Some(WideKeyword::RevertLayer) => if inline_phase.get() {
                    inline_baseline.get(aid.to_str()).and_then(Option::as_deref)
                } else {
                    presentation_baseline.get(aid.to_str()).map(String::as_str)
                }
                .unwrap_or(fallback),
                Some(WideKeyword::Revert) => fallback,
                _ => value,
            };
            // Check that attribute already exists.
            let idx = doc.attrs[attrs_start_idx..]
                .iter_mut()
                .position(|a| a.name == aid);
            if !override_presentation
                && idx.is_some_and(|idx| attrs_start_idx + idx < presentation_end_idx)
            {
                return;
            }

            // Append an attribute as usual.
            let added = append_attribute(
                parent_id,
                tag_name,
                aid,
                roxmltree::StringStorage::new_owned(value),
                important,
                doc,
            );

            // Check that attribute was actually added, because it could be skipped.
            if added {
                if let Some(idx) = idx {
                    let last_idx = doc.attrs.len() - 1;
                    let existing_idx = attrs_start_idx + idx;

                    let has_precedence = important || !doc.attrs[existing_idx].important;

                    if has_precedence {
                        doc.attrs.swap(existing_idx, last_idx);
                    }

                    // Remove last.
                    doc.attrs.pop();
                }
            }
        };

    let dimension = |aid: AId, name: &str| {
        matches!(aid, AId::Width | AId::Height)
            && crate::style::svg_values::presentation_property(tag_name.to_str(), name).is_some()
    };
    let mut declared: HashMap<&'static str, (String, bool)> = HashMap::new();
    let mut write_declaration = |declaration: &Declaration,
                                 override_presentation: bool|
     -> Result<(), crate::style::StyleError> {
        if declaration.name.starts_with("--") {
            return Ok(());
        }
        // TODO: perform XML attribute normalization
        let imp = declaration.important;
        let substituted = variables.substitute(
            &declaration.name,
            declaration.value,
            declaration.substitutions,
        )?;
        let val = match &substituted {
            crate::style::svg_cascade::Substitution::Value(value) => value.as_ref(),
            crate::style::svg_cascade::Substitution::Unset => "unset",
            crate::style::svg_cascade::Substitution::Discard => return Ok(()),
        };

        if declaration.name == "zoom" {
            if let Some(value) = crate::style::svg_cascade::zoom(val)? {
                insert_attribute(AId::CssZoom, &value, imp, override_presentation);
            }
        } else if declaration.name == "offset" {
            for (property, value) in crate::style::svg_cascade::expand("offset", val)? {
                if let Some(aid) = AId::from_str(property) {
                    insert_attribute(aid, &value, imp, override_presentation);
                }
            }
        } else if let Some(aid) = match &*declaration.name {
            "translate" => Some(AId::CssTranslate),
            "rotate" => Some(AId::CssRotate),
            "scale" => Some(AId::CssScale),
            _ => None,
        } {
            if !crate::style::svg_cascade::expand(&declaration.name, val)?.is_empty() {
                insert_attribute(aid, val, imp, override_presentation);
            }
        } else if declaration.name == "marker" {
            insert_attribute(AId::MarkerStart, val, imp, override_presentation);
            insert_attribute(AId::MarkerMid, val, imp, override_presentation);
            insert_attribute(AId::MarkerEnd, val, imp, override_presentation);
        } else if declaration.name == "font" {
            if crate::style::svg_cascade::WideKeyword::parse(val).is_some() {
                insert_attribute(AId::FontVariant, val, imp, override_presentation);
                for (property, value) in crate::style::svg_cascade::expand("font", val)? {
                    let aid = if property == "font-width" {
                        Some(AId::FontStretch)
                    } else {
                        AId::from_str(property)
                    };
                    if let Some(aid) = aid {
                        insert_attribute(aid, &value, imp, override_presentation);
                    }
                }
            } else if let Ok(shorthand) = FontShorthand::from_str(val) {
                // First we need to reset all values to their default.
                insert_attribute(AId::FontStyle, "normal", imp, override_presentation);
                insert_attribute(AId::FontVariant, "normal", imp, override_presentation);
                insert_attribute(AId::FontWeight, "normal", imp, override_presentation);
                insert_attribute(AId::FontStretch, "normal", imp, override_presentation);
                insert_attribute(AId::LineHeight, "normal", imp, override_presentation);
                insert_attribute(AId::FontSizeAdjust, "none", imp, override_presentation);
                insert_attribute(AId::FontKerning, "auto", imp, override_presentation);
                insert_attribute(AId::FontVariantCaps, "normal", imp, override_presentation);
                insert_attribute(
                    AId::FontVariantLigatures,
                    "normal",
                    imp,
                    override_presentation,
                );
                insert_attribute(
                    AId::FontVariantNumeric,
                    "normal",
                    imp,
                    override_presentation,
                );
                insert_attribute(
                    AId::FontVariantEastAsian,
                    "normal",
                    imp,
                    override_presentation,
                );
                insert_attribute(
                    AId::FontVariantPosition,
                    "normal",
                    imp,
                    override_presentation,
                );

                // Then, we set the properties that have been declared.
                shorthand
                    .font_stretch
                    .map(|s| insert_attribute(AId::FontStretch, s, imp, override_presentation));
                shorthand
                    .font_weight
                    .map(|s| insert_attribute(AId::FontWeight, s, imp, override_presentation));
                shorthand
                    .font_variant
                    .map(|s| insert_attribute(AId::FontVariant, s, imp, override_presentation));
                shorthand
                    .font_style
                    .map(|s| insert_attribute(AId::FontStyle, s, imp, override_presentation));
                insert_attribute(
                    AId::FontSize,
                    shorthand.font_size,
                    imp,
                    override_presentation,
                );
                insert_attribute(
                    AId::FontFamily,
                    shorthand.font_family,
                    imp,
                    override_presentation,
                );
            } else {
                log::warn!(
                    "Failed to parse {} value: '{}'",
                    AId::Font,
                    declaration.value
                );
            }
        } else if let Some(aid) = AId::from_str(&declaration.name)
            .filter(|&aid| aid.is_presentation() || dimension(aid, &declaration.name))
        {
            if (dimension(aid, &declaration.name) || declaration.name.starts_with("offset-"))
                && crate::style::svg_cascade::expand(&declaration.name, val)?.is_empty()
            {
                return Ok(());
            }
            insert_attribute(aid, val, imp, override_presentation);
        } else {
            // Other properties reach the text's computed style; an important declaration
            // keeps its place against later normal ones.
            for (longhand, value) in crate::style::svg_cascade::expand(&declaration.name, val)? {
                if declared
                    .get(longhand)
                    .is_none_or(|(_, important)| imp || !important)
                {
                    declared.insert(longhand, (value, imp));
                }
            }
        }
        Ok(())
    };
    let styled = declarations
        .into_iter()
        .try_for_each(|(declaration, overrides, inline)| {
            inline_phase.set(inline);
            write_declaration(declaration, overrides)
        });
    if let Err(error) = styled {
        doc.style_error.get_or_insert(std::sync::Arc::new(error));
    }
    if pseudo == Some(PseudoElement::FirstLine) {
        doc.append_attribute(
            AId::FirstLine,
            roxmltree::StringStorage::Borrowed(""),
            false,
        );
    }

    if doc.nodes.len() > MAX_SVG_NODES {
        return Err(Error::NodesLimitReached);
    }

    let node_id = doc.append(
        parent_id,
        NodeKind::Element {
            tag_name,
            attributes: ShortRange::new(attrs_start_idx as u32, doc.attrs.len() as u32),
        },
    );

    doc.variables.insert(node_id.get_usize(), variables);
    if !declared.is_empty() {
        doc.declared.insert(
            node_id.get_usize(),
            declared
                .into_iter()
                .map(|(property, (value, _))| (property, value))
                .collect(),
        );
    }
    Ok(node_id)
}

fn is_writing_attribute(aid: AId) -> bool {
    matches!(
        aid,
        AId::WritingMode
            | AId::TextOrientation
            | AId::GlyphOrientationVertical
            | AId::GlyphOrientationHorizontal
    )
}

fn writing_attribute(aid: AId, source: &str) -> Option<(AId, String)> {
    let (name, value) = crate::style::svg_values::writing_attribute(aid.to_str(), source)?;
    Some((AId::from_str(name)?, value.to_owned()))
}

fn append_attribute<'input>(
    parent_id: NodeId,
    tag_name: EId,
    aid: AId,
    value: roxmltree::StringStorage<'input>,
    important: bool,
    doc: &mut Document<'input>,
) -> bool {
    let value = if aid == AId::Display {
        let canonical = {
            let mut input = cssparser::ParserInput::new(&value);
            let parser = &mut cssparser::Parser::new(&mut input);
            parser
                .expect_ident_cloned()
                .ok()
                .filter(|_| parser.expect_exhausted().is_ok())
                .map(|ident| ident.to_ascii_lowercase())
        };
        match canonical {
            Some(keyword) if keyword != &*value => roxmltree::StringStorage::new_owned(&keyword),
            _ => value,
        }
    } else {
        value
    };
    match aid {
        // The `style` attribute will be split into attributes, so we don't need it.
        AId::Style |
        // No need to copy a `class` attribute since CSS were already resolved.
        AId::Class => return false,
        _ => {}
    }

    // Ignore `xlink:href` on `tspan` (which was originally `tref` or `a`),
    // because we will convert `tref` into `tspan` anyway.
    if tag_name == EId::Tspan && aid == AId::Href {
        return false;
    }

    let value = if aid.allows_inherit_value() {
        use crate::style::svg_cascade::WideKeyword;
        match WideKeyword::parse(&value) {
            Some(WideKeyword::Inherit) => return resolve_inherit(parent_id, aid, important, doc),
            Some(WideKeyword::Unset) if aid.is_inheritable() => {
                return resolve_inherit(parent_id, aid, important, doc);
            }
            Some(WideKeyword::Unset | WideKeyword::Initial) => initial_value(aid),
            _ => value,
        }
    } else {
        value
    };
    doc.append_attribute(aid, value, important);
    true
}

fn resolve_inherit(parent_id: NodeId, aid: AId, important: bool, doc: &mut Document) -> bool {
    if aid == AId::FontSize {
        // Inherit the computed parent size, not its relative expression.
        doc.append_attribute(aid, roxmltree::StringStorage::Borrowed("100%"), important);
        return true;
    }
    if aid.is_inheritable() {
        // Inheritable attributes can inherit a value from an any ancestor.
        let node_id = doc
            .get(parent_id)
            .ancestors()
            .find(|n| n.has_attribute(aid))
            .map(|n| n.id);
        if let Some(node_id) = node_id {
            if let Some(attr) = doc
                .get(node_id)
                .attributes()
                .iter()
                .find(|a| a.name == aid)
                .cloned()
            {
                doc.attrs.push(Attribute {
                    name: aid,
                    value: attr.value,
                    important,
                    inherited: true,
                });

                return true;
            }
        }
    } else {
        // Non-inheritable attributes can inherit a value only from a direct parent.
        if let Some(attr) = doc
            .get(parent_id)
            .attributes()
            .iter()
            .find(|a| a.name == aid)
            .cloned()
        {
            doc.attrs.push(Attribute {
                name: aid,
                value: attr.value,
                important,
                inherited: true,
            });

            return true;
        }
    }

    let value = if aid == AId::TransformOrigin {
        roxmltree::StringStorage::Borrowed("0 0")
    } else {
        initial_value(aid)
    };
    doc.append_attribute(aid, value, important);
    true
}

fn initial_value(aid: AId) -> roxmltree::StringStorage<'static> {
    let value = match aid {
        AId::ImageRendering | AId::ShapeRendering | AId::TextRendering => "auto",

        AId::ClipPath
        | AId::Filter
        | AId::MarkerEnd
        | AId::MarkerMid
        | AId::MarkerStart
        | AId::Mask
        | AId::Stroke
        | AId::StrokeDasharray
        | AId::TextDecoration
        | AId::Transform => "none",

        AId::FontStretch
        | AId::FontStyle
        | AId::FontVariant
        | AId::FontWeight
        | AId::LetterSpacing
        | AId::WordSpacing => "normal",

        AId::Fill | AId::FloodColor | AId::StopColor => "black",

        AId::FillOpacity
        | AId::FloodOpacity
        | AId::Opacity
        | AId::StopOpacity
        | AId::StrokeOpacity => "1",

        AId::ClipRule | AId::FillRule => "nonzero",

        AId::BaselineShift => "baseline",
        AId::ColorInterpolationFilters => "linearRGB",
        AId::Direction => "ltr",
        AId::Display => "inline",
        AId::FontSize => "medium",
        AId::Overflow => "visible",
        AId::StrokeDashoffset => "0",
        AId::StrokeLinecap => "butt",
        AId::StrokeLinejoin => "miter",
        AId::StrokeMiterlimit => "4",
        AId::StrokeWidth => "1",
        AId::TextAnchor => "start",
        AId::TextOrientation => "mixed",
        AId::Visibility => "visible",
        AId::WritingMode => "lr-tb",
        _ => {
            let name = match aid {
                AId::CssTranslate => "translate",
                AId::CssRotate => "rotate",
                AId::CssScale => "scale",
                _ => aid.to_str(),
            };
            return crate::style::svg_cascade::initial(name)
                .map(|value| roxmltree::StringStorage::new_owned(&value))
                .unwrap_or(roxmltree::StringStorage::Borrowed(super::CSS_INITIAL));
        }
    };

    roxmltree::StringStorage::Borrowed(value)
}

fn resolve_href<'a, 'input: 'a>(
    node: roxmltree::Node<'a, 'input>,
    id_map: &HashMap<&str, roxmltree::Node<'a, 'input>>,
) -> Option<roxmltree::Node<'a, 'input>> {
    let link_value = node
        .attribute((XLINK_NS, "href"))
        .or_else(|| node.attribute("href"))?;

    let link_id = svgtypes::IRI::from_str(link_value).ok()?.0;

    id_map.get(link_id).copied()
}

fn resolve_svg_use<'a, 'input>(
    node: roxmltree::Node<'a, 'input>,
    origin: roxmltree::Node,
    id_map: &HashMap<&str, roxmltree::Node<'a, 'input>>,
) -> Option<roxmltree::Node<'a, 'input>> {
    let link = resolve_href(node, id_map)?;

    if link == node || link == origin {
        log::warn!(
            "Recursive 'use' detected. '{}' will be skipped.",
            node.attribute((SVG_NS, "id")).unwrap_or_default()
        );
        return None;
    }

    // Make sure we're linked to an SVG element.
    if parse_tag_name(link).is_none() {
        return None;
    }

    // Check that none of the linked node's children reference current `use` node
    // via other `use` node.
    //
    // Example:
    // <g id="g1">
    //     <use xlink:href="#use1" id="use2"/>
    // </g>
    // <use xlink:href="#g1" id="use1"/>
    //
    // `use2` should be removed.
    //
    // Also, child should not reference its parent:
    // <g id="g1">
    //     <use xlink:href="#g1" id="use1"/>
    // </g>
    //
    // `use1` should be removed.
    let mut is_recursive = false;
    for link_child in link
        .descendants()
        .skip(1)
        .filter(|n| n.has_tag_name((SVG_NS, "use")))
    {
        if let Some(link2) = resolve_href(link_child, id_map) {
            if link2 == node || link2 == link {
                is_recursive = true;
                break;
            }
        }
    }

    if is_recursive {
        log::warn!(
            "Recursive 'use' detected. '{}' will be skipped.",
            node.attribute((SVG_NS, "id")).unwrap_or_default()
        );
        return None;
    }

    Some(link)
}

fn resolve_css<'a>(
    xml: &'a roxmltree::Document<'a>,
    style_sheet: Option<&'a str>,
) -> StyleSheet<'a> {
    let mut sheet = StyleSheet::new();

    // Injected style sheets do not override internal ones (we mimic the logic of rsvg-convert),
    // so we need to parse it first.
    if let Some(style_sheet) = style_sheet {
        sheet.parse_more(style_sheet);
    }

    for node in xml.descendants().filter(|n| n.has_tag_name("style")) {
        match node.attribute("type") {
            Some("text/css") => {}
            Some(_) => continue,
            None => {}
        }

        let text = match node.text() {
            Some(v) => v,
            None => continue,
        };

        sheet.parse_more(text);
    }

    sheet.prepare(xml);
    sheet
}

/// Breaks the reference cycles that would make conversion recurse endlessly.
pub(super) fn fix_recursions(doc: &mut Document) {
    fix_recursive_patterns(doc);
    fix_recursive_links(EId::ClipPath, AId::ClipPath, doc);
    fix_recursive_links(EId::Mask, AId::Mask, doc);
    fix_recursive_links(EId::Filter, AId::Filter, doc);
    fix_recursive_fe_image(doc);
}

fn fix_recursive_patterns(doc: &mut Document) {
    while let Some(node_id) = find_recursive_pattern(AId::Fill, doc) {
        let idx = doc.get(node_id).attribute_id(AId::Fill).unwrap();
        doc.attrs[idx].value = roxmltree::StringStorage::Borrowed("none");
    }

    while let Some(node_id) = find_recursive_pattern(AId::Stroke, doc) {
        let idx = doc.get(node_id).attribute_id(AId::Stroke).unwrap();
        doc.attrs[idx].value = roxmltree::StringStorage::Borrowed("none");
    }
}

fn find_recursive_pattern(aid: AId, doc: &mut Document) -> Option<NodeId> {
    for pattern_node in doc
        .root()
        .descendants()
        .filter(|n| n.tag_name() == Some(EId::Pattern))
    {
        for node in pattern_node.descendants() {
            let value = match node.attribute(aid) {
                Some(v) => v,
                None => continue,
            };

            if let Ok(svgtypes::Paint::FuncIRI(link_id, _)) = svgtypes::Paint::from_str(value) {
                if link_id == pattern_node.element_id() {
                    // If a pattern child has a link to the pattern itself
                    // then we have to replace it with `none`.
                    // Otherwise we will get endless loop/recursion and stack overflow.
                    return Some(node.id);
                } else {
                    // Check that linked node children doesn't link this pattern.
                    if let Some(linked_node) = doc.element_by_id(link_id) {
                        for node2 in linked_node.descendants() {
                            let value2 = match node2.attribute(aid) {
                                Some(v) => v,
                                None => continue,
                            };

                            if let Ok(svgtypes::Paint::FuncIRI(link_id2, _)) =
                                svgtypes::Paint::from_str(value2)
                            {
                                if link_id2 == pattern_node.element_id() {
                                    return Some(node2.id);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    None
}

fn fix_recursive_links(eid: EId, aid: AId, doc: &mut Document) {
    while let Some(node_id) = find_recursive_link(eid, aid, doc) {
        let idx = doc.get(node_id).attribute_id(aid).unwrap();
        doc.attrs[idx].value = roxmltree::StringStorage::Borrowed("none");
    }
}

fn find_recursive_link(eid: EId, aid: AId, doc: &Document) -> Option<NodeId> {
    for node in doc
        .root()
        .descendants()
        .filter(|n| n.tag_name() == Some(eid))
    {
        for child in node.descendants() {
            if let Some(link) = child.node_attribute(aid) {
                if link == node {
                    // If an element child has a link to the element itself
                    // then we have to replace it with `none`.
                    // Otherwise we will get endless loop/recursion and stack overflow.
                    return Some(child.id);
                } else {
                    // Check that linked node children doesn't link this element.
                    for node2 in link.descendants() {
                        if let Some(link2) = node2.node_attribute(aid) {
                            if link2 == node {
                                return Some(node2.id);
                            }
                        }
                    }
                }
            }
        }
    }

    None
}

/// Detects cases like:
///
/// ```xml
/// <filter id="filter1">
///   <feImage xlink:href="#rect1"/>
/// </filter>
/// <rect id="rect1" x="36" y="36" width="120" height="120" fill="green" filter="url(#filter1)"/>
/// ```
fn fix_recursive_fe_image(doc: &mut Document) {
    let mut ids = Vec::new();
    for fe_node in doc
        .root()
        .descendants()
        .filter(|n| n.tag_name() == Some(EId::FeImage))
    {
        if let Some(link) = fe_node.node_attribute(AId::Href) {
            if let Some(filter_uri) = link.attribute::<&str>(AId::Filter) {
                let filter_id = fe_node.parent().unwrap().element_id();
                for func in svgtypes::FilterValueListParser::from(filter_uri).flatten() {
                    if let svgtypes::FilterValue::Url(url) = func {
                        if url == filter_id {
                            ids.push(link.id);
                        }
                    }
                }
            }
        }
    }

    for id in ids {
        let idx = doc.get(id).attribute_id(AId::Filter).unwrap();
        doc.attrs[idx].value = roxmltree::StringStorage::Borrowed("none");
    }
}
