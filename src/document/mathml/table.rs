use super::{Attributes, Element, child_classes, classify};
use crate::document::{BlockBox, BlockContent, DocumentError};
use crate::resources::ResourceResolver;
use crate::style::{Display, Style, Stylesheet};

pub(in crate::document) fn is_cell(element: crate::dom::Element<'_>, display: &Display) -> bool {
    element.namespace() == crate::dom::MATHML_NAMESPACE
        && element.name() == "mtd"
        && *display == Display::TableCell
}

pub(crate) fn cell(
    style: &Style,
    children: Vec<BlockContent>,
    sheet: &Stylesheet,
    resolver: &dyn ResourceResolver,
) -> Result<Vec<BlockContent>, DocumentError> {
    let mut row = BlockBox::anonymous(style.anonymous(Display::BlockMath), children, None);
    let children = child_classes(&row);
    for child in &children {
        if let Some(operator) = &child.operator {
            operator.resolve(sheet, resolver)?;
        }
    }
    row.set_math(Element {
        principal: false,
        name: "mrow".into(),
        text: None,
        attributes: Attributes::default(),
        radical: None,
        class: classify("mrow", children),
    });
    Ok(vec![BlockContent::Block(row)])
}
