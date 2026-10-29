//! Semantic data retained while the ordinary formatter creates `MathML` boxes.
pub(super) mod table;
use crate::document::{BlockBox, BlockContent, DocumentError, Paragraph};
use crate::dom::{Element as DomElement, MATHML_NAMESPACE, NodeData, TreeRelation};
use crate::resources::ResourceResolver;
use crate::style::{Display, LengthPercentage, Stylesheet};
use std::collections::HashMap;
use std::{cell::OnceCell, rc::Rc};

pub(crate) const MAX_DEPTH: usize = 256;

pub(crate) struct Element {
    pub principal: bool,
    pub name: Box<str>,
    pub text: Option<String>,
    pub attributes: Attributes,
    pub radical: Option<Paragraph>,
    pub class: Class,
}

const OPERATOR_LENGTHS: &[&str] = &["lspace", "rspace", "minsize", "maxsize"];

#[derive(Clone, Default)]
pub(crate) struct Class {
    pub space_like: bool,
    pub operator: Option<Rc<OperatorSource>>,
}
pub(crate) struct OperatorSource {
    pub style: Box<crate::style::Style>,
    pub text: Option<String>,
    attributes: Attributes,
    resolved: OnceCell<Attributes>,
}
impl OperatorSource {
    pub(crate) fn attributes(&self) -> &Attributes {
        self.resolved
            .get()
            .expect("native consumers resolve the core's attributes during formatting")
    }
    fn resolve(
        &self,
        sheet: &Stylesheet,
        resolver: &dyn ResourceResolver,
    ) -> Result<(), DocumentError> {
        if self.resolved.get().is_some() {
            return Ok(());
        }
        let mut attributes = self.attributes.clone();
        for &name in OPERATOR_LENGTHS {
            if let Some(value) = attributes.get(name)
                && let Some(length) = sheet.math_attribute_length(value, &self.style, resolver)?
            {
                attributes.lengths.insert(name.into(), length);
            }
        }
        self.resolved
            .set(attributes)
            .unwrap_or_else(|_| unreachable!("the formatting pass resolves a core once"));
        Ok(())
    }
}

pub(super) fn classify(name: &str, children: impl IntoIterator<Item = Class>) -> Class {
    let children: Vec<_> = children.into_iter().collect();
    match name {
        "mtext" | "mspace" | "annotation" | "annotation-xml" => Class {
            space_like: true,
            operator: None,
        },
        "mfrac" | "msub" | "msup" | "msubsup" | "munder" | "mover" | "munderover"
        | "mmultiscripts" => Class {
            space_like: false,
            operator: children.first().and_then(|child| child.operator.clone()),
        },
        "mi" | "mn" | "ms" | "msqrt" | "mroot" | "mtable" | "mtr" | "mtd" => Class::default(),
        _ => {
            let mut significant = children.iter().filter(|child| !child.space_like);
            let first = significant.next();
            Class {
                space_like: first.is_none(),
                operator: first
                    .and_then(|child| child.operator.clone())
                    .filter(|_| significant.next().is_none()),
            }
        }
    }
}

pub(crate) fn block_class(block: &BlockBox) -> Class {
    block.math().map_or_else(
        || {
            if block.is_anonymous() {
                classify("mrow", child_classes(block))
            } else {
                Class::default()
            }
        },
        |math| math.class.clone(),
    )
}

fn child_classes(block: &BlockBox) -> Vec<Class> {
    block
        .children()
        .iter()
        .flat_map(|child| match child {
            BlockContent::Block(block) => vec![block_class(block)],
            BlockContent::Inline(paragraph) => paragraph.math_children(),
            BlockContent::Events(_) | BlockContent::Positioned(_) => Vec::new(),
        })
        .collect()
}

pub(super) fn inline_class(
    element: DomElement<'_>,
    style: &crate::style::Style,
    children: Vec<Class>,
) -> Option<Class> {
    if element.namespace() != MATHML_NAMESPACE {
        return None;
    }
    Some(if element.name() == "mo" {
        let mut attributes = Attributes::default();
        for name in OPERATOR_LENGTHS.iter().copied().chain([
            "form",
            "stretchy",
            "symmetric",
            "largeop",
            "movablelimits",
        ]) {
            if let Some(value) = element.attr(name) {
                attributes.values.insert(name.into(), value.into());
            }
        }
        Class {
            space_like: false,
            operator: Some(Rc::new(OperatorSource {
                style: Box::new(style.clone()),
                text: text_content(element),
                attributes,
                resolved: OnceCell::new(),
            })),
        }
    } else {
        classify(element.name(), children)
    })
}

fn text_content(element: DomElement<'_>) -> Option<String> {
    let mut text = String::new();
    for node in element.as_node().children(TreeRelation::Formatting) {
        match node.value() {
            NodeData::Text(value) => text.push_str(value),
            NodeData::Element(_) => return None,
            _ => {}
        }
    }
    Some(text)
}

#[derive(Clone, Default)]
pub(crate) struct Attributes {
    values: HashMap<Box<str>, Box<str>>,
    lengths: HashMap<Box<str>, LengthPercentage>,
}
impl Attributes {
    pub(crate) fn get(&self, name: &str) -> Option<&str> {
        self.values.get(name).map(AsRef::as_ref)
    }
    pub(crate) fn length(&self, name: &str) -> Option<&LengthPercentage> {
        self.lengths.get(name)
    }
}

pub(super) fn prepare(
    element: DomElement<'_>,
    block: &mut BlockBox,
    sheet: &Stylesheet,
    resolver: &dyn ResourceResolver,
) -> Result<(), DocumentError> {
    if element.namespace() != MATHML_NAMESPACE {
        return Ok(());
    }
    let name = element.name();
    let text = text_content(element);
    let native = matches!(
        block.style().display,
        Display::BlockMath | Display::InlineMath
    );
    let arity = block
        .children()
        .iter()
        .filter(|child| matches!(child, BlockContent::Block(_) | BlockContent::Inline(_)))
        .count();
    let lengths: &[&str] = if native {
        match name {
            "mo" => OPERATOR_LENGTHS,
            "mspace" => &["height"],
            "mpadded" => &["height", "depth", "lspace", "voffset"],
            "mfrac" if arity == 2 => &["linethickness"],
            _ => &[],
        }
    } else {
        &[]
    };
    let mut attributes = Attributes::default();
    for name in lengths
        .iter()
        .copied()
        .chain(
            (if name == "mo" { OPERATOR_LENGTHS } else { &[] })
                .iter()
                .copied(),
        )
        .chain([
            "form",
            "stretchy",
            "symmetric",
            "largeop",
            "movablelimits",
            "accent",
            "accentunder",
        ])
    {
        if let Some(value) = element.attr(name) {
            attributes.values.insert(name.into(), value.into());
        }
    }
    for &name in lengths {
        if let Some(value) = element.attr(name)
            && let Some(length) = sheet.math_attribute_length(value, block.style(), resolver)?
        {
            attributes.lengths.insert(name.into(), length);
        }
    }
    let radical = if native && matches!(name, "msqrt" | "mroot") {
        Paragraph::generated("√", block.style())?
    } else {
        None
    };
    let children = child_classes(block);
    let class = if name == "mo" {
        Class {
            space_like: false,
            operator: Some(Rc::new(OperatorSource {
                style: Box::new(block.style().clone()),
                text: text.clone(),
                attributes: attributes.clone(),
                resolved: OnceCell::new(),
            })),
        }
    } else {
        classify(name, children.clone())
    };
    if native {
        for child in children.iter().chain(std::iter::once(&class)) {
            if let Some(operator) = &child.operator {
                operator.resolve(sheet, resolver)?;
            }
        }
    }
    block.set_math(Element {
        principal: true,
        name: name.into(),
        text,
        attributes,
        radical,
        class,
    });
    Ok(())
}
