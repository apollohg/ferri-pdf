//! Shared static HTML tree, selector matching and explicit navigation relationships.
mod attributes;
mod controls;
pub use controls::InputType;
mod language;
mod selector;
mod shadow;
mod state;
mod tree;
pub use attributes::{integer_digits, non_negative_integer_digits};
pub use selector::{
    GeneratedPseudoElement, NamespaceMap, ScopeRoot, Selector, SelectorError, SelectorMode,
    SelectorScope, TreeContext, parse_namespace,
};
pub use shadow::MAX_SHADOW_DEPTH;
pub use state::{Directionality, InitialState};

#[derive(Debug, thiserror::Error)]
pub enum DomError {
    #[error("{resource} exceeds {limit}")]
    TreeLimit {
        resource: &'static str,
        limit: usize,
    },
}

pub use html5ever::tree_builder::QuirksMode;
use html5ever::{
    TokenizerResult,
    driver::{ParseOpts, parse_document},
    tendril::TendrilSink,
    tree_builder::TreeBuilderOpts,
};
pub use scraper::Node as NodeData;
use std::ops::ControlFlow;
pub use tree::{
    ChildNodes, DOCUMENT_CONTEXT_ORDER, DocumentTree, Element, ElementId, Node, TreeRelation,
    TreeScopeId, XmlTree,
};

pub const HTML_NAMESPACE: &str = "http://www.w3.org/1999/xhtml";
pub const SVG_NAMESPACE: &str = "http://www.w3.org/2000/svg";
pub const MATHML_NAMESPACE: &str = "http://www.w3.org/1998/Math/MathML";
pub const XML_NAMESPACE: &str = "http://www.w3.org/XML/1998/namespace";
pub const XLINK_NAMESPACE: &str = "http://www.w3.org/1999/xlink";
const META_ELEMENT: &str = "meta";
const CHARSET_ATTRIBUTE: &str = "charset";
const HTTP_EQUIV_ATTRIBUTE: &str = "http-equiv";
const CONTENT_ATTRIBUTE: &str = "content";
const CONTENT_TYPE_PRAGMA: &str = "content-type";

/// A declaration encountered by the real HTML parser, before any encoding restart.
#[derive(Clone, Copy, Debug)]
pub struct EncodingDeclaration<'a> {
    pub charset: Option<&'a str>,
    pub content_type: Option<&'a str>,
}

impl DocumentTree {
    pub fn parse(html: &str) -> Result<Self, DomError> {
        match Self::parse_with_encoding(html, |_| {
            ControlFlow::<std::convert::Infallible>::Continue(())
        })? {
            ControlFlow::Continue(tree) => Ok(tree),
            ControlFlow::Break(never) => match never {},
        }
    }

    /// Stop only when the caller needs to re-decode the original buffered bytes.
    pub fn parse_with_encoding<T>(
        html: &str,
        mut on_encoding: impl FnMut(EncodingDeclaration<'_>) -> ControlFlow<T>,
    ) -> Result<ControlFlow<T, Self>, DomError> {
        let options = ParseOpts {
            tree_builder: TreeBuilderOpts {
                scripting_enabled: false,
                ..Default::default()
            },
            ..Default::default()
        };
        let parser = parse_document(shadow::DocumentSink::new(), options);
        parser.input_buffer.push_back(html.into());
        loop {
            match parser.tokenizer.feed(&parser.input_buffer) {
                TokenizerResult::Done => break,
                TokenizerResult::EncodingIndicator(_) => {
                    // Upstream also emits this for link/base attributes; only meta can change encoding.
                    let document = parser.tokenizer.sink.sink.html.0.borrow();
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
                            return Ok(ControlFlow::Break(value));
                        }
                    }
                }
                TokenizerResult::Script(_) => {}
            }
        }
        let (html, shadow) = parser.finish()?;
        Ok(ControlFlow::Continue(Self::from_parsed(html, shadow)))
    }
}
