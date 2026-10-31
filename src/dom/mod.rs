//! Shared static HTML tree, selector matching and explicit navigation relationships.
mod attributes;
mod control_values;
mod controls;
pub(crate) use control_values::{parse_float, stripped_newlines, valid_float};
pub(crate) use controls::{GaugeRegion, InputAttribute, InputType, non_negative_integer};
mod language;
mod selector;
mod shadow;
#[cfg(test)]
pub(crate) use shadow::MAX_SHADOW_DEPTH;
mod state;
mod tree;
pub(crate) use attributes::{integer_digits, non_negative_integer_digits};
pub(crate) use selector::{
    GeneratedPseudoElement, NamespaceMap, PseudoElement, ScopeRoot, Selector, SelectorMode,
    SelectorOrigin, SelectorScope, TreeContext, parse_namespace,
};
pub(crate) use state::{
    Directionality, InitialState, ScrollMarker, ScrollMarkerStates, TargetPosition,
};

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum DomError {
    #[error("{resource} exceeds {limit}")]
    TreeLimit {
        resource: &'static str,
        limit: usize,
    },
}

pub(crate) use html5ever::tree_builder::QuirksMode;
use html5ever::{
    TokenizerResult,
    tokenizer::{BufferQueue, Tokenizer},
    tree_builder::{TreeBuilder, TreeBuilderOpts, TreeSink},
};
pub(crate) use scraper::Node as NodeData;
use std::ops::ControlFlow;
pub(crate) use tree::table::{column_span, row_span};
pub(crate) use tree::{
    ChildNodes, DOCUMENT_CONTEXT_ORDER, DocumentTree, Element, ElementId, Node, TreeRelation,
    TreeScopeId, XmlTree,
};

/// Shared budget for nested CSS blocks and functions.
pub(crate) const MAX_COMPONENT_DEPTH: usize = 256;
pub(crate) const HTML_NAMESPACE: &str = "http://www.w3.org/1999/xhtml";
pub(crate) const SVG_NAMESPACE: &str = "http://www.w3.org/2000/svg";
pub(crate) const MATHML_NAMESPACE: &str = "http://www.w3.org/1998/Math/MathML";
pub(crate) const XML_NAMESPACE: &str = "http://www.w3.org/XML/1998/namespace";
pub(crate) const XLINK_NAMESPACE: &str = "http://www.w3.org/1999/xlink";
const META_ELEMENT: &str = "meta";
const CHARSET_ATTRIBUTE: &str = "charset";
const HTTP_EQUIV_ATTRIBUTE: &str = "http-equiv";
const CONTENT_ATTRIBUTE: &str = "content";
const CONTENT_TYPE_PRAGMA: &str = "content-type";

/// A declaration encountered by the real HTML parser, before any encoding restart.
#[derive(Clone, Copy, Debug)]
pub(crate) struct EncodingDeclaration<'a> {
    pub charset: Option<&'a str>,
    pub content_type: Option<&'a str>,
}

impl DocumentTree {
    pub(crate) fn parse(html: &str) -> Self {
        match Self::parse_with_encoding(html, |_| {
            ControlFlow::<std::convert::Infallible>::Continue(())
        }) {
            ControlFlow::Continue(tree) => tree,
            ControlFlow::Break(never) => match never {},
        }
    }

    /// Stop only when the caller needs to re-decode the original buffered bytes.
    pub(crate) fn parse_with_encoding<T>(
        html: &str,
        mut on_encoding: impl FnMut(EncodingDeclaration<'_>) -> ControlFlow<T>,
    ) -> ControlFlow<T, Self> {
        let options = TreeBuilderOpts {
            scripting_enabled: false,
            ..Default::default()
        };
        let tokenizer = Tokenizer::new(
            shadow::DocumentTokens(TreeBuilder::new(shadow::DocumentSink::new(), options)),
            Default::default(),
        );
        let input = BufferQueue::default();
        input.push_back(html.into());
        loop {
            match tokenizer.feed(&input) {
                TokenizerResult::Done => break,
                TokenizerResult::EncodingIndicator(_) => {
                    // Upstream also emits this for link/base attributes; only meta can change encoding.
                    let document = tokenizer.sink.0.sink.html.0.borrow();
                    if let Some(element) = document
                        .tree
                        .nodes()
                        .next_back()
                        .and_then(|node| node.value().as_element())
                        && element.name() == META_ELEMENT
                        && element.name.ns.as_ref() == HTML_NAMESPACE
                    {
                        let declaration = EncodingDeclaration {
                            charset: element.attr(CHARSET_ATTRIBUTE),
                            content_type: element
                                .attr(HTTP_EQUIV_ATTRIBUTE)
                                .filter(|value| value.eq_ignore_ascii_case(CONTENT_TYPE_PRAGMA))
                                .and_then(|_| element.attr(CONTENT_ATTRIBUTE)),
                        };
                        if let ControlFlow::Break(value) = on_encoding(declaration) {
                            return ControlFlow::Break(value);
                        }
                    }
                }
                TokenizerResult::Script(_) => {}
            }
        }
        tokenizer.end();
        let (html, shadow, limit) = tokenizer.sink.0.sink.finish();
        let mut tree = Self::from_parsed(html, shadow);
        tree.limits.extend(limit);
        tree.update_selectedcontents();
        tree.attach_use_instances();
        tree.attach_details_shadow_trees();
        tree.attach_select_shadow_trees();
        ControlFlow::Continue(tree)
    }
}

#[cfg(test)]
mod tests;
