//! Standalone XML for `<svg>` elements embedded in HTML, so they share the SVG image pipeline.
//! The document's cascade styles that content (SVG 2 §6.4): each SVG element carries its
//! computed presentation properties, and each `use` element its own styled instance (§5.5.3).
mod presentation;
#[cfg(test)]
mod tests;

use crate::document::DocumentError;
use crate::dom::{
    DocumentTree, Element, ElementId, Node, NodeData, SVG_NAMESPACE, TreeRelation, XLINK_NAMESPACE,
    XML_NAMESPACE,
};
use crate::resources::ResourceResolver;
use crate::style::{Style, Stylesheet, svg_values::InitialOnly};
use std::{
    collections::{HashMap, HashSet},
    fmt::Write,
    sync::Arc,
};

const ID_ATTRIBUTE: &str = "id";
/// `lang` and `xml:lang` (SVG 2 §2.6.2).
const LANG_ATTRIBUTE: &str = "lang";

const HREF_ATTRIBUTE: &str = "href";
const STYLE_ATTRIBUTE: &str = "style";
const CLASS_ATTRIBUTE: &str = "class";
const SVG_ELEMENT: &str = "svg";
const USE_ELEMENT: &str = "use";
/// The cascade has applied the style sheets of these elements already.
const STYLE_ELEMENT: &str = "style";
const DEFINITIONS_ELEMENT: &str = "defs";
/// The root's geometry sizes its replaced box, which the document lays out; the renderer
/// takes that box as its viewport.
const ROOT_GEOMETRY: [&str; 4] = ["x", "y", "width", "height"];
/// Ids naming the styled clones that `use` elements reference in the written XML.
const INSTANCE_ID_PREFIX: &str = "ferri-pdf-instance-";

/// The style sheets and fonts that style inline SVG content.
pub(crate) struct Styling<'a> {
    pub(crate) sheet: &'a Stylesheet,
    pub(crate) resolver: &'a dyn ResourceResolver,
}

/// Loads the CSS mask layers of an SVG element with the computed style, and the documents
/// they reference.
pub(crate) type MaskLoader<'a> =
    dyn FnMut(&Style) -> Result<Arc<crate::graphics::LoadedMasks>, DocumentError> + 'a;

/// An SVG document written from styled SVG content, and the CSS mask layers its `mask`
/// values name.
pub(crate) struct SerializedSvg {
    pub(crate) xml: String,
    pub(crate) layer_masks: crate::images::LayerMaskTable,
}

/// Serializes the subtree of an `<svg>` element whose computed style is `style` as an SVG
/// document with explicit namespaces. Elements and attributes with undeclared prefixes (editor
/// metadata) are omitted.
pub(crate) fn serialize(
    element: Element<'_>,
    style: &Style,
    styling: &Styling<'_>,
    load_masks: &mut MaskLoader<'_>,
) -> Result<SerializedSvg, DocumentError> {
    Serializer::new(element, styling, load_masks).document(element, style)
}

enum Step<'d> {
    /// A node, and whether it is written as parsed, outside the SVG content's styling.
    Open(Node<'d>, bool),
    /// An element's end tag, and whether its start pushed a style.
    Close(&'d str, bool),
}

struct Serializer<'a, 'd> {
    styling: &'a Styling<'a>,
    load_masks: &'a mut MaskLoader<'a>,
    /// The mask layers that `mask` values written so far name, by index.
    layer_masks: Vec<Arc<crate::graphics::LoadedMasks>>,
    initial_only: InitialOnly,
    xml: String,
    /// Ids written so far; a clone repeating one omits it.
    written: HashSet<String>,
    /// The source's ids, which instance ids avoid.
    taken: HashSet<&'d str>,
    next_instance: usize,
    /// Instance roots with their ids and their `use` element's style, to write in `defs`.
    instances: Vec<(String, Element<'d>, Style)>,
}

impl<'a, 'd> Serializer<'a, 'd> {
    fn new(
        root: Element<'d>,
        styling: &'a Styling<'a>,
        load_masks: &'a mut MaskLoader<'a>,
    ) -> Self {
        Self {
            styling,
            load_masks,
            layer_masks: Vec::new(),
            initial_only: InitialOnly::new(),
            xml: String::new(),
            written: HashSet::new(),
            taken: root
                .as_node()
                .inclusive_descendants(TreeRelation::Selector)
                .filter_map(|node| node.element()?.attr(ID_ATTRIBUTE))
                .collect(),
            next_instance: 0,
            instances: Vec::new(),
        }
    }

    fn document(
        mut self,
        root: Element<'d>,
        style: &Style,
    ) -> Result<SerializedSvg, DocumentError> {
        let style = style.canonical();
        let _ = write!(
            self.xml,
            "<{} xmlns=\"{SVG_NAMESPACE}\" xmlns:xlink=\"{XLINK_NAMESPACE}\"",
            root.name()
        );
        for (name, value) in &root.value().attrs {
            let Some(prefix) = prefix(&name.ns, &name.local) else {
                continue;
            };
            let local = &*name.local;
            if prefix.is_empty() {
                if local == STYLE_ATTRIBUTE
                    || local == CLASS_ATTRIBUTE
                    || (crate::style::svg_values::presentation_property(root.name(), local)
                        .is_some()
                        && !ROOT_GEOMETRY.contains(&local))
                {
                    continue;
                }
                if local == ID_ATTRIBUTE {
                    self.written.insert(value.to_string());
                }
            }
            attribute(&mut self.xml, prefix, local, value);
        }
        attribute(
            &mut self.xml,
            "",
            STYLE_ATTRIBUTE,
            &presentation::declarations(root.name(), style, None)?,
        );
        self.xml.push('>');
        for child in root.as_node().children(TreeRelation::Selector) {
            self.subtree(child, style, None)?;
        }
        if !self.instances.is_empty() {
            let _ = write!(self.xml, "<{DEFINITIONS_ELEMENT}>");
            while let Some((id, instance, host)) = self.instances.pop() {
                self.subtree(instance.as_node(), &host, Some(id))?;
            }
            let _ = write!(self.xml, "</{DEFINITIONS_ELEMENT}>");
        }
        let _ = write!(self.xml, "</{}>", root.name());
        Ok(SerializedSvg {
            xml: self.xml,
            layer_masks: self.layer_masks.into(),
        })
    }

    /// Writes `node` and its descendants, whose parent's style is `parent`; `instance` is the
    /// id of a `use` element's clone that `node` roots.
    fn subtree(
        &mut self,
        node: Node<'d>,
        parent: &Style,
        mut instance: Option<String>,
    ) -> Result<(), DocumentError> {
        let cloned = instance.is_some();
        let mut styles: Vec<Style> = Vec::new();
        let mut stack = vec![Step::Open(node, false)];
        while let Some(step) = stack.pop() {
            let (node, raw) = match step {
                Step::Close(name, styled) => {
                    let _ = write!(self.xml, "</{name}>");
                    if styled {
                        styles.pop();
                    }
                    continue;
                }
                Step::Open(node, raw) => (node, raw),
            };
            let element = match node.value() {
                NodeData::Text(text) => {
                    escape(&mut self.xml, text, false);
                    continue;
                }
                NodeData::Element(element) if !element.name().contains(':') => {
                    node.element().expect("element node")
                }
                _ => continue,
            };
            let raw = raw || element.namespace() != SVG_NAMESPACE;
            if !raw && element.name() == STYLE_ELEMENT {
                continue;
            }
            let style = if raw {
                None
            } else {
                let style = self.styling.sheet.computed_without_box(
                    element,
                    styles.last().unwrap_or(parent),
                    self.styling.resolver,
                )?;
                self.initial_only
                    .check(&style, presentation::is_text_content(element.name()))?;
                Some(style)
            };
            let reference = match &style {
                Some(style) if element.name() == USE_ELEMENT => {
                    let Some(target) = element.shadow_root().and_then(|root| {
                        root.children(TreeRelation::Selector)
                            .find_map(Node::element)
                    }) else {
                        // §5.5.1: a `use` element without an instance renders nothing.
                        continue;
                    };
                    let id = self.instance_id();
                    self.instances.push((id.clone(), target, style.clone()));
                    Some(id)
                }
                _ => None,
            };
            let _ = write!(self.xml, "<{}", element.name());
            match &style {
                Some(style) => {
                    self.styled_attributes(element, style, instance.take(), cloned, reference)?;
                }
                None => {
                    for (name, value) in &element.value().attrs {
                        if let Some(prefix) = prefix(&name.ns, &name.local) {
                            attribute(&mut self.xml, prefix, &name.local, value);
                        }
                    }
                }
            }
            self.xml.push('>');
            stack.push(Step::Close(element.name(), style.is_some()));
            stack.extend(
                node.children(TreeRelation::Selector)
                    .rev()
                    .map(|child| Step::Open(child, raw)),
            );
            if let Some(style) = style {
                styles.push(style);
            }
        }
        Ok(())
    }

    fn instance_id(&mut self) -> String {
        loop {
            let id = format!("{INSTANCE_ID_PREFIX}{}", self.next_instance);
            self.next_instance += 1;
            if !self.taken.contains(id.as_str()) && self.written.insert(id.clone()) {
                return id;
            }
        }
    }

    /// The attributes of a styled SVG element: those the cascade took become its computed
    /// geometry and `style` declarations. A clone's root takes the `instance` id; another
    /// clone omits an id already written.
    fn styled_attributes(
        &mut self,
        element: Element<'d>,
        style: &Style,
        instance: Option<String>,
        cloned: bool,
        reference: Option<String>,
    ) -> Result<(), DocumentError> {
        let tag = element.name();
        for (name, value) in &element.value().attrs {
            let Some(prefix) = prefix(&name.ns, &name.local) else {
                continue;
            };
            let local = &*name.local;
            if reference.is_some() && local == HREF_ATTRIBUTE {
                continue;
            }
            if prefix.is_empty() {
                if local == STYLE_ATTRIBUTE
                    || local == CLASS_ATTRIBUTE
                    || crate::style::svg_values::presentation_property(tag, local).is_some()
                {
                    continue;
                }
                if local == ID_ATTRIBUTE {
                    if instance.is_some() || (cloned && self.written.contains(&**value)) {
                        continue;
                    }
                    self.written.insert(value.to_string());
                }
            }
            attribute(&mut self.xml, prefix, local, value);
        }
        if let Some(id) = instance {
            attribute(&mut self.xml, "", ID_ATTRIBUTE, &id);
        }
        if let Some(id) = reference {
            attribute(&mut self.xml, "", HREF_ATTRIBUTE, &format!("#{id}"));
        }
        // Text takes the language it inherits from the document, which cases its capitals.
        if presentation::is_text_content(tag)
            && !style.language.is_empty()
            && !element
                .value()
                .attrs
                .iter()
                .any(|(name, _)| &*name.local == LANG_ATTRIBUTE)
        {
            attribute(&mut self.xml, "xml:", LANG_ATTRIBUTE, &style.language);
        }
        for (name, value) in presentation::geometry(tag, style)?
            .into_iter()
            .chain(presentation::transform(tag, style)?)
        {
            attribute(&mut self.xml, "", name, &value);
        }
        let mask = if let Some(mask) = presentation::mask(&style.decoration.masks) {
            mask
        } else {
            self.layer_masks.push((self.load_masks)(style)?);
            crate::svg::layer_mask_value(self.layer_masks.len() - 1)
        };
        attribute(
            &mut self.xml,
            "",
            STYLE_ATTRIBUTE,
            &presentation::declarations(tag, style, Some(&mask))?,
        );
        Ok(())
    }
}

/// The written prefix of an attribute, or `None` for one with an undeclared prefix.
fn prefix(namespace: &str, local: &str) -> Option<&'static str> {
    match namespace {
        "" if !local.contains(':') => Some(""),
        XLINK_NAMESPACE => Some("xlink:"),
        XML_NAMESPACE => Some("xml:"),
        _ => None,
    }
}

fn attribute(xml: &mut String, prefix: &str, name: &str, value: &str) {
    let _ = write!(xml, " {prefix}{name}=\"");
    escape(xml, value, true);
    xml.push('"');
}

/// SVG elements that CSS references select: Filter Effects 1 §5 `filter`, CSS Masking 1
/// §6.1 `clipPath` and §7.1 `mask` elements, motion path shapes, and the paint servers that
/// `element()` uses as paint sources (CSS Images 4 §2.7.1).
const REFERENCED_ELEMENTS: [&str; 13] = [
    "filter",
    "clipPath",
    "mask",
    "path",
    "rect",
    "circle",
    "ellipse",
    "line",
    "polyline",
    "polygon",
    "linearGradient",
    "radialGradient",
    "pattern",
];

/// The document's referenceable SVG resources by `id`, each with the standalone XML of the
/// outermost `<svg>` holding it, so that references inside that SVG resolve too.
#[derive(Clone, Default)]
pub(crate) struct SvgDefinitions {
    entries: HashMap<String, SvgDefinition>,
    document: Option<crate::resources::Url>,
    /// The other documents that references name, as formatting loaded them.
    linked: crate::images::LinkedDocuments,
}

#[derive(Clone)]
pub(crate) struct SvgDefinition {
    /// The element's name.
    pub(crate) element: &'static str,
    pub(crate) xml: Arc<str>,
    pub(crate) layer_masks: crate::images::LayerMaskTable,
}

impl SvgDefinitions {
    /// `root` is the document root element's computed style, which the outermost `<svg>`
    /// elements inherit through their ancestors'.
    pub(crate) fn collect(
        tree: &DocumentTree,
        root: &Style,
        styling: &Styling<'_>,
        linked: crate::images::LinkedDocuments,
        load_masks: &mut MaskLoader<'_>,
    ) -> Result<Self, DocumentError> {
        let mut roots: HashMap<ElementId, (Arc<str>, crate::images::LayerMaskTable)> =
            HashMap::new();
        let mut definitions = HashMap::new();
        let mut seen = HashSet::new();
        for element in tree.elements() {
            let Some(id) = element.attr(ID_ATTRIBUTE).filter(|id| !id.is_empty()) else {
                continue;
            };
            if !seen.insert(id) {
                continue;
            }
            let Some(name) = REFERENCED_ELEMENTS
                .into_iter()
                .find(|name| element.namespace() == SVG_NAMESPACE && element.name() == *name)
            else {
                continue;
            };
            let Some(svg) = element
                .ancestors(TreeRelation::Selector)
                .filter(|ancestor| {
                    ancestor.namespace() == SVG_NAMESPACE && ancestor.name() == SVG_ELEMENT
                })
                .last()
            else {
                continue;
            };
            let (xml, layer_masks) = if let Some(serialized) = roots.get(&svg.id()) {
                serialized.clone()
            } else {
                let style = computed_in_place(svg, root, styling)?;
                let serialized = serialize(svg, &style, styling, load_masks)?;
                let serialized = (serialized.xml.into(), serialized.layer_masks);
                roots.insert(svg.id(), serialized.clone());
                serialized
            };
            definitions.entry(id.to_owned()).or_insert(SvgDefinition {
                element: name,
                xml,
                layer_masks,
            });
        }
        Ok(Self {
            entries: definitions,
            document: tree.document_url.clone(),
            linked,
        })
    }
    pub(crate) fn motion(&self, reference: &str) -> Option<&SvgDefinition> {
        let (document, id) = reference.rsplit_once('#')?;
        if !document.is_empty() {
            let mut current = self.document.clone()?;
            current.set_fragment(None);
            if crate::resources::Url::parse(document).ok()? != current {
                return None;
            }
        }
        self.get_decoded(id)
    }
    /// A standalone SVG document without natural dimensions whose viewport the paint server
    /// with ID `id` fills: its `objectBoundingBox` units span the viewport, and its user
    /// units are CSS pixels (CSS Images 4 §2.7.1).
    pub(crate) fn paint_server(&self, id: &str) -> Option<(String, crate::images::LayerMaskTable)> {
        let definition = self.entries.get(id).filter(|definition| {
            matches!(
                definition.element,
                "linearGradient" | "radialGradient" | "pattern"
            )
        })?;
        let mut url = String::new();
        cssparser::serialize_string(&format!("#{id}"), &mut url)
            .expect("writing a String cannot fail");
        let mut fill = String::new();
        escape(&mut fill, &format!("url({url})"), true);
        Some((
            format!(
                "<svg xmlns=\"http://www.w3.org/2000/svg\"><defs>{}</defs>\
                 <rect width=\"100%\" height=\"100%\" fill=\"{fill}\"/></svg>",
                definition.xml
            ),
            definition.layer_masks.clone(),
        ))
    }
    fn get_decoded(&self, id: &str) -> Option<&SvgDefinition> {
        let id = percent_encoding::percent_decode_str(id)
            .decode_utf8()
            .ok()?;
        self.entries.get(id.as_ref())
    }
    /// The tree in which a white rectangle spanning `bounds` references, through `property`,
    /// the `element` element that `reference` names, in this document or another that
    /// formatting loaded; `None` when it names no such element.
    pub(crate) fn reference_tree(
        &self,
        element: &str,
        property: &str,
        reference: &str,
        bounds: Option<crate::graphics::Rect>,
    ) -> Option<crate::svg::usvg::Tree> {
        let documents = |url: &str| {
            self.linked
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .get(url)
                .cloned()
                .flatten()
        };
        let Some(id) = reference.strip_prefix('#') else {
            let (url, id) = reference.rsplit_once('#')?;
            let linked = documents(url)?;
            let id = percent_encoding::percent_decode_str(id)
                .decode_utf8()
                .ok()?;
            let named = roxmltree::Document::parse_with_options(
                &linked.text,
                crate::svg::usvg::xml_options(),
            )
            .ok()?
            .descendants()
            .find(|node| node.attribute(ID_ATTRIBUTE) == Some(&*id))
            .is_some_and(|node| node.has_tag_name((SVG_NAMESPACE, element)));
            return named
                .then(|| {
                    crate::svg::reference_tree(
                        "",
                        crate::images::LayerMaskTable::default(),
                        property,
                        reference,
                        bounds,
                        &documents,
                    )
                })
                .flatten();
        };
        let definition = self
            .entries
            .get(id)
            .filter(|definition| definition.element == element)?;
        crate::svg::reference_tree(
            &definition.xml,
            definition.layer_masks.clone(),
            property,
            reference,
            bounds,
            &documents,
        )
    }
}

/// The computed style of `element`, from its inheritance ancestors' down to its own; the
/// document root's is `root`. Hidden ancestors keep every value: their SVG content can still
/// be referenced (SVG 2 §3.2).
fn computed_in_place(
    element: Element<'_>,
    root: &Style,
    styling: &Styling<'_>,
) -> Result<Style, DocumentError> {
    let mut chain: Vec<_> = std::iter::successors(Some(element), |element| {
        element.parent(TreeRelation::Inheritance)
    })
    .collect();
    chain.pop();
    let mut style = root.clone();
    for element in chain.into_iter().rev() {
        style = styling
            .sheet
            .computed_without_box(element, &style, styling.resolver)?;
    }
    Ok(style)
}

pub(crate) fn escape(xml: &mut String, text: &str, attribute: bool) {
    for ch in text.chars() {
        match ch {
            '&' => xml.push_str("&amp;"),
            '<' => xml.push_str("&lt;"),
            '>' => xml.push_str("&gt;"),
            '"' if attribute => xml.push_str("&quot;"),
            ch => xml.push(ch),
        }
    }
}
