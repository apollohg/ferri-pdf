use crate::{DomError, HTML_NAMESPACE, NodeData, state};
use ego_tree::NodeId;
use html5ever::{
    Attribute, QualName,
    tendril::StrTendril,
    tokenizer::{TagKind, Token, TokenSink, TokenSinkResult},
    tree_builder::{ElementFlags, NodeOrText, QuirksMode, TreeBuilder, TreeSink},
};
use scraper::{Html, HtmlTreeSink};
use std::{
    borrow::Cow,
    cell::{Ref, RefCell},
    collections::{HashMap, HashSet},
};

pub const MAX_SHADOW_DEPTH: usize = 64;
const SHADOW_DEPTH: &str = "shadow tree nesting";
const SHADOW_HOST_NAMES: &[&str] = &[
    "article",
    "aside",
    "blockquote",
    "body",
    "div",
    "footer",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "header",
    "main",
    "nav",
    "p",
    "section",
    "span",
];
const SLOT_ELEMENT: &str = "slot";
const SLOT_ATTRIBUTE: &str = "slot";
const SLOT_NAME: &str = "name";
const EXPORTPARTS_ATTRIBUTE: &str = "exportparts";
const TEMPLATE_ELEMENT: &str = "template";
const SHADOW_ROOT_MODE_ATTRIBUTE: &str = "shadowrootmode";
const SHADOW_ROOT_MODES: [&str; 2] = ["open", "closed"];

#[derive(Clone, Copy, Debug)]
pub(crate) struct ShadowRoot {
    pub fragment: NodeId,
    pub depth: usize,
}

#[derive(Default, Debug)]
pub(crate) struct ShadowState {
    pub roots: HashMap<NodeId, ShadowRoot>,
    pub hosts: HashMap<NodeId, NodeId>,
    pub scopes: HashMap<NodeId, NodeId>,
    pub context_order: HashMap<NodeId, usize>,
    pub assigned: HashMap<NodeId, NodeId>,
    pub slots: HashMap<NodeId, Vec<NodeId>>,
    pub fallback_slots: HashMap<NodeId, NodeId>,
    pub populated_slots: HashSet<NodeId>,
    pub exports: HashMap<NodeId, HashMap<String, Vec<String>>>,
    fragments: Vec<NodeId>,
}
impl ShadowState {
    pub(crate) fn order_contexts(&mut self, connected: impl IntoIterator<Item = NodeId>) {
        for fragment in connected.into_iter().chain(self.fragments.iter().copied()) {
            let next = self.context_order.len() + 1;
            self.context_order.entry(fragment).or_insert(next);
        }
    }
    pub(crate) fn alias_slot(&self, node: NodeId) -> Option<NodeId> {
        self.assigned
            .get(&node)
            .or_else(|| self.fallback_slots.get(&node))
            .copied()
    }

    pub(crate) fn finish(&mut self, html: &Html) {
        for &fragment in &self.fragments {
            let mut pending = vec![fragment];
            while let Some(id) = pending.pop() {
                self.scopes.insert(id, fragment);
                pending.extend(
                    html.tree
                        .get(id)
                        .unwrap()
                        .children()
                        .filter(|node| !node.value().is_fragment())
                        .map(|node| node.id()),
                );
            }
        }
        if self.roots.is_empty() {
            return;
        }
        for (&host, root) in &self.roots {
            if let Some(source) = html
                .tree
                .get(host)
                .unwrap()
                .value()
                .as_element()
                .and_then(|element| element.attr(EXPORTPARTS_ATTRIBUTE))
            {
                let mut names: HashMap<String, Vec<String>> = HashMap::new();
                for mapping in source.split(',') {
                    let mapping = mapping.trim_matches(state::is_html_whitespace);
                    let (inner, outer) = mapping.split_once(':').unwrap_or((mapping, mapping));
                    let inner = inner.trim_matches(state::is_html_whitespace);
                    let outer = outer.trim_matches(state::is_html_whitespace);
                    let valid = |name: &str| {
                        !name.is_empty()
                            && !name
                                .chars()
                                .any(|c| c == ':' || state::is_html_whitespace(c))
                    };
                    if valid(inner) && valid(outer) {
                        let aliases = names.entry(outer.to_owned()).or_default();
                        if !aliases.iter().any(|name| name == inner) {
                            aliases.push(inner.to_owned());
                        }
                    }
                }
                self.exports.insert(host, names);
            }
            let mut names = HashMap::new();
            let mut pending: Vec<_> = Self::children(html, root.fragment)
                .into_iter()
                .rev()
                .collect();
            while let Some(id) = pending.pop() {
                let node = html.tree.get(id).unwrap();
                if let Some(element) = node.value().as_element()
                    && element.name.ns.as_ref() == HTML_NAMESPACE
                    && element.name() == SLOT_ELEMENT
                {
                    self.slots.insert(id, Vec::new());
                    names
                        .entry(element.attr(SLOT_NAME).unwrap_or_default().to_owned())
                        .or_insert(id);
                }
                pending.extend(Self::children(html, id).into_iter().rev());
            }
            for child in html.tree.get(host).unwrap().children() {
                let name = match child.value() {
                    NodeData::Element(element) => element.attr(SLOT_ATTRIBUTE).unwrap_or_default(),
                    NodeData::Text(_) => "",
                    _ => continue,
                };
                if let Some(&slot) = names.get(name) {
                    self.assigned.insert(child.id(), slot);
                    self.slots.get_mut(&slot).unwrap().push(child.id());
                }
            }
        }
        for (&slot, assigned) in &self.slots {
            if assigned.is_empty() {
                for child in html.tree.get(slot).unwrap().children() {
                    if matches!(child.value(), NodeData::Element(_) | NodeData::Text(_)) {
                        self.fallback_slots.insert(child.id(), slot);
                    }
                }
            }
        }
        let mut populated = HashSet::new();
        for (&node, &slot) in self.assigned.iter().chain(&self.fallback_slots) {
            if self.slots.contains_key(&node) {
                continue;
            }
            let mut current = Some(slot);
            while let Some(slot) = current {
                if !populated.insert(slot) {
                    break;
                }
                current = self.alias_slot(slot);
            }
        }
        self.populated_slots = populated;
    }

    fn children(html: &Html, id: NodeId) -> Vec<NodeId> {
        html.tree
            .get(id)
            .unwrap()
            .children()
            .filter(|node| !node.value().is_fragment())
            .map(|node| node.id())
            .collect()
    }
}

pub(crate) struct DocumentSink {
    pub html: HtmlTreeSink,
    shadow: RefCell<ShadowState>,
    error: RefCell<Option<DomError>>,
    /// Source spelling of the `shadowrootmode` keyword canonicalised for the current token.
    shadow_root_mode: RefCell<Option<StrTendril>>,
}
impl DocumentSink {
    pub(crate) fn new() -> Self {
        Self {
            html: HtmlTreeSink::new(Html::new_document()),
            shadow: RefCell::default(),
            error: RefCell::default(),
            shadow_root_mode: RefCell::default(),
        }
    }
}

/// The tree builder compares the enumerated `shadowrootmode` keywords case-sensitively,
/// so template start tags reach it in canonical case; created elements keep the source value.
pub(crate) struct DocumentTokens(pub TreeBuilder<NodeId, DocumentSink>);
impl TokenSink for DocumentTokens {
    type Handle = NodeId;

    fn process_token(&self, mut token: Token, line_number: u64) -> TokenSinkResult<NodeId> {
        if let Token::TagToken(tag) = &mut token
            && tag.kind == TagKind::StartTag
            && tag.name.as_ref() == TEMPLATE_ELEMENT
            && let Some(attr) = tag
                .attrs
                .iter_mut()
                .find(|attr| attr.name.local.as_ref() == SHADOW_ROOT_MODE_ATTRIBUTE)
            && let Some(mode) = SHADOW_ROOT_MODES
                .into_iter()
                .find(|mode| attr.value.eq_ignore_ascii_case(mode))
            && attr.value.as_ref() != mode
        {
            let source = std::mem::replace(&mut attr.value, StrTendril::from_slice(mode));
            *self.0.sink.shadow_root_mode.borrow_mut() = Some(source);
            let result = self.0.process_token(token, line_number);
            self.0.sink.shadow_root_mode.take();
            return result;
        }
        self.0.process_token(token, line_number)
    }
    fn end(&self) {
        self.0.end();
    }
    fn adjusted_current_node_present_but_not_in_html_namespace(&self) -> bool {
        self.0
            .adjusted_current_node_present_but_not_in_html_namespace()
    }
}
impl TreeSink for DocumentSink {
    type Handle = NodeId;
    type Output = Result<(Html, ShadowState), DomError>;
    type ElemName<'a> = Ref<'a, QualName>;

    fn finish(self) -> Self::Output {
        if let Some(error) = self.error.into_inner() {
            return Err(error);
        }
        let html = self.html.finish();
        let mut shadow = self.shadow.into_inner();
        shadow.finish(&html);
        Ok((html, shadow))
    }
    fn parse_error(&self, message: Cow<'static, str>) {
        self.html.parse_error(message);
    }
    fn get_document(&self) -> Self::Handle {
        self.html.get_document()
    }
    fn elem_name<'a>(&'a self, target: &'a Self::Handle) -> Self::ElemName<'a> {
        self.html.elem_name(target)
    }
    fn create_element(
        &self,
        name: QualName,
        mut attrs: Vec<Attribute>,
        flags: ElementFlags,
    ) -> Self::Handle {
        if let Some(source) = self.shadow_root_mode.borrow().as_ref()
            && let Some(attr) = attrs
                .iter_mut()
                .find(|attr| attr.name.local.as_ref() == SHADOW_ROOT_MODE_ATTRIBUTE)
        {
            attr.value = source.clone();
        }
        let template = flags.template;
        let node = self.html.create_element(name, attrs, flags);
        if template {
            self.shadow
                .borrow_mut()
                .fragments
                .push(self.html.get_template_contents(&node));
        }
        node
    }
    fn create_comment(&self, text: StrTendril) -> Self::Handle {
        self.html.create_comment(text)
    }
    fn create_pi(&self, target: StrTendril, data: StrTendril) -> Self::Handle {
        self.html.create_pi(target, data)
    }
    fn append(&self, parent: &Self::Handle, child: NodeOrText<Self::Handle>) {
        self.html.append(parent, child);
    }
    fn append_based_on_parent_node(
        &self,
        element: &Self::Handle,
        previous: &Self::Handle,
        child: NodeOrText<Self::Handle>,
    ) {
        self.html
            .append_based_on_parent_node(element, previous, child);
    }
    fn append_doctype_to_document(&self, name: StrTendril, public: StrTendril, system: StrTendril) {
        self.html.append_doctype_to_document(name, public, system);
    }
    fn get_template_contents(&self, target: &Self::Handle) -> Self::Handle {
        self.html.get_template_contents(target)
    }
    fn same_node(&self, x: &Self::Handle, y: &Self::Handle) -> bool {
        self.html.same_node(x, y)
    }
    fn set_quirks_mode(&self, mode: QuirksMode) {
        self.html.set_quirks_mode(mode);
    }
    fn append_before_sibling(&self, sibling: &Self::Handle, node: NodeOrText<Self::Handle>) {
        self.html.append_before_sibling(sibling, node);
    }
    fn add_attrs_if_missing(&self, target: &Self::Handle, attrs: Vec<Attribute>) {
        self.html.add_attrs_if_missing(target, attrs);
    }
    fn remove_from_parent(&self, target: &Self::Handle) {
        self.html.remove_from_parent(target);
    }
    fn reparent_children(&self, node: &Self::Handle, parent: &Self::Handle) {
        self.html.reparent_children(node, parent);
    }
    fn mark_script_already_started(&self, node: &Self::Handle) {
        self.html.mark_script_already_started(node);
    }
    fn associate_with_form(
        &self,
        target: &Self::Handle,
        form: &Self::Handle,
        nodes: (&Self::Handle, Option<&Self::Handle>),
    ) {
        self.html.associate_with_form(target, form, nodes);
    }
    fn is_mathml_annotation_xml_integration_point(&self, node: &Self::Handle) -> bool {
        self.html.is_mathml_annotation_xml_integration_point(node)
    }

    fn attach_declarative_shadow(
        &self,
        host: &Self::Handle,
        template: &Self::Handle,
        _: &[Attribute],
    ) -> bool {
        let html = self.html.0.borrow();
        let host_node = html.tree.get(*host).unwrap();
        let Some(element) = host_node.value().as_element() else {
            return false;
        };
        if element.name.ns.as_ref() != HTML_NAMESPACE
            || !(SHADOW_HOST_NAMES.contains(&element.name())
                || state::is_custom_element_name(element.name()))
        {
            return false;
        }
        let mut shadow = self.shadow.borrow_mut();
        if shadow.roots.contains_key(host) {
            return false;
        }
        let mut depth = 1;
        for ancestor in host_node.ancestors() {
            if ancestor.value().is_fragment() {
                if let Some(parent_host) = shadow.hosts.get(&ancestor.id()) {
                    depth = shadow.roots[parent_host].depth + 1;
                }
                break;
            }
        }
        if depth > MAX_SHADOW_DEPTH {
            self.error.borrow_mut().get_or_insert(DomError::TreeLimit {
                resource: SHADOW_DEPTH,
                limit: MAX_SHADOW_DEPTH,
            });
            return false;
        }
        let fragment = self.html.get_template_contents(template);
        shadow.roots.insert(*host, ShadowRoot { fragment, depth });
        shadow.hosts.insert(fragment, *host);
        true
    }
}
