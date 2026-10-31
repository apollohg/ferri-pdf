//! The `::first-line` pseudo-element (CSS Pseudo-Elements 4 §2.1): the contents of the first
//! formatted line inherit through it, so their styles are computed again for that line.
use crate::document::{DocumentError, Paragraph, formatting::ImageResources};
use crate::dom::{DocumentTree, ElementId};
use crate::style::{Display, OriginNode, PseudoElement, Size, Style, StyleOrigin, Stylesheet};
use std::collections::HashMap;

/// The copy of `paragraph`, the untransformed first formatted line of `origins` (outermost
/// first, ending with its own block), that their `::first-line` pseudo-elements style; `None`
/// where no rule selects one.
pub(super) fn variant(
    paragraph: &mut Paragraph,
    origins: &[ElementId],
    sheet: &Stylesheet,
    document: &DocumentTree,
    resources: &mut ImageResources<'_>,
) -> Result<Option<Paragraph>, DocumentError> {
    // The annotation boxes take their first-line boxes while the block's style restyles.
    let block_style = paragraph.style().clone();
    let Some(FirstLineStyles { mut restyle, lines }) =
        FirstLineStyles::of(&block_style, origins, sheet, document, resources)?
    else {
        return Ok(None);
    };
    let spans = paragraph
        .spans()
        .iter()
        .map(|span| restyle.style(&span.style))
        .collect::<Result<Vec<_>, _>>()?;
    let box_styles = paragraph
        .boxes()
        .iter()
        .map(|inline_box| restyle.style(&inline_box.style))
        .collect::<Result<Vec<_>, _>>()?;
    // A ruby annotation takes the style its annotation box takes on the line.
    let atomics = paragraph
        .atomics()
        .iter()
        .map(|atomic| atomic.block.style())
        .chain(
            (0..paragraph.annotations().len()).map(|index| paragraph.annotation_box_style(index)),
        )
        .map(|style| restyle.style(style))
        .collect::<Result<Vec<_>, _>>()?;
    let mut boxes = Vec::with_capacity(box_styles.len());
    for style in box_styles {
        let decoration = resources.backgrounds(&style);
        boxes.push((style, decoration));
    }
    // Each ruby annotation box's contents inherit through the box from `::first-line`.
    for index in 0..paragraph.annotations().len() {
        let Some(source) = paragraph
            .annotation_block_mut(index)
            .take_annotation_source()
        else {
            continue;
        };
        let root = annotation_root(paragraph, index, &boxes, &mut restyle)?;
        let contents_spans = source
            .spans()
            .iter()
            .map(|span| restyle.style(&span.style))
            .collect::<Result<Vec<_>, _>>()?;
        let mut contents_boxes = Vec::with_capacity(source.boxes().len());
        for inline_box in source.boxes() {
            let style = restyle.style(&inline_box.style)?;
            let decoration = resources.backgrounds(&style);
            contents_boxes.push((style, decoration));
        }
        let contents = source.restyled(root.clone(), contents_spans, contents_boxes)?;
        paragraph.annotation_block_mut(index).set_first_line_box(
            crate::document::BlockBox::anonymous(
                root,
                vec![crate::document::BlockContent::Inline(contents)],
                None,
            ),
        );
    }
    Ok(Some(paragraph.first_line_copy(
        restyle.root,
        lines,
        spans,
        boxes,
        atomics,
    )))
}

/// The style of the block laying out ruby annotation box `index`'s contents on the first
/// line, from its annotation container and box there; anonymous ones inherit from the boxes
/// around them, whose first-line styles are `boxes` (CSS Ruby 1 §2.1.1).
fn annotation_root(
    paragraph: &Paragraph,
    index: usize,
    boxes: &[(Style, crate::graphics::BoxDecoration)],
    restyle: &mut Restyle<'_>,
) -> Result<Style, DocumentError> {
    use crate::document::ruby;
    let (container, level, annotation) = paragraph
        .rubies()
        .iter()
        .flat_map(|ruby| {
            ruby.segments
                .iter()
                .flat_map(|segment| &segment.levels)
                .map(move |level| (ruby.container, level))
        })
        .find_map(|(container, level)| {
            level
                .annotations
                .iter()
                .find(|annotation| annotation.block == Some(index))
                .map(|annotation| (container, level, annotation))
        })
        .expect("each annotation box belongs to a ruby annotation");
    let level_style = if level.style.origin.node().is_some() {
        restyle.style(&level.style)?
    } else {
        ruby::level_style(&boxes[container].0)
    };
    let annotation_style = if annotation.style.origin.node().is_some() {
        restyle.style(&annotation.style)?
    } else {
        ruby::anonymous(&level_style, Display::RubyText)
    };
    Ok(ruby::annotation_wrapper(
        &level_style,
        &annotation_style,
        level.inter_character,
    ))
}

/// The styles a block's first formatted line gives its contents.
pub(super) struct FirstLineStyles<'a> {
    restyle: Restyle<'a>,
    /// The `::first-line` boxes, outermost first, styled and decorated.
    lines: Vec<(Style, crate::graphics::BoxDecoration)>,
}
impl<'a> FirstLineStyles<'a> {
    /// The first formatted line of `origins` (outermost first, ending with the block styled
    /// `block`); `None` where no rule selects `::first-line`.
    pub(super) fn of<'r: 'a>(
        block: &'a Style,
        origins: &[ElementId],
        sheet: &'a Stylesheet,
        document: &'a DocumentTree,
        resources: &mut ImageResources<'r>,
    ) -> Result<Option<Self>, DocumentError> {
        if matches!(block.display, Display::InlineMath | Display::BlockMath)
            || !sheet.has_pseudo_rules(PseudoElement::FirstLine)
        {
            return Ok(None);
        }
        let mut parent = block.clone();
        let mut lines = Vec::with_capacity(origins.len());
        for &origin in origins {
            let line = sheet
                .typographic_pseudo_with_resolver(
                    element(document, origin),
                    &parent,
                    PseudoElement::FirstLine,
                    resources.resolver,
                )?
                .used();
            let mut boxed = line.clone();
            // Margins, borders and padding do not apply to `::first-line` (§2.1.2).
            boxed.margin = [Size::ZERO; 4];
            boxed.padding = [crate::style::LengthPercentage::ZERO; 4];
            boxed.decoration = crate::document::formatting::background_only(&line.decoration);
            let decoration = resources.backgrounds(&boxed);
            lines.push((boxed, decoration));
            parent = line;
        }
        Ok(Some(Self {
            restyle: Restyle {
                sheet,
                document,
                resolver: resources.resolver,
                block: &block.origin,
                root: Style::through_first_line(block, &parent),
                styles: HashMap::new(),
            },
            lines,
        }))
    }
    /// `style` on the first line, keeping its box's display.
    pub(super) fn style(&mut self, style: &Style) -> Result<Style, DocumentError> {
        self.restyle.style(style)
    }
}

fn element(document: &DocumentTree, id: ElementId) -> crate::dom::Element<'_> {
    document
        .element(id)
        .expect("first-line origins belong to the formatted document")
}

/// Recomputes styles below the block with the first line's root inline box style `root`.
struct Restyle<'a> {
    sheet: &'a Stylesheet,
    document: &'a DocumentTree,
    resolver: &'a dyn crate::resources::ResourceResolver,
    block: &'a StyleOrigin,
    root: Style,
    styles: HashMap<*const OriginNode, Option<Style>>,
}
impl Restyle<'_> {
    /// `style` on the first line, keeping its box's display.
    fn style(&mut self, style: &Style) -> Result<Style, DocumentError> {
        if matches!(style.display, Display::InlineMath | Display::BlockMath) {
            return Ok(style.clone());
        }
        Ok(match self.origin(&style.origin)? {
            Some(mut restyled) => {
                restyled.display = style.display.clone();
                restyled
            }
            None => style.clone(),
        })
    }
    /// The style the box of `origin` takes on the first line; `None` for a box outside the
    /// block, or one whose inheritance through `::first-line` is undefined: `::marker`
    /// (CSS Pseudo-Elements 4 §4.2, issue 4506).
    fn origin(&mut self, origin: &StyleOrigin) -> Result<Option<Style>, DocumentError> {
        if origin.same(self.block) {
            return Ok(Some(self.root.clone()));
        }
        let Some(node) = origin.node() else {
            return Ok(None);
        };
        let key = std::sync::Arc::as_ptr(node);
        if let Some(style) = self.styles.get(&key) {
            return Ok(style.clone());
        }
        let style = match self.origin(&node.parent)? {
            Some(parent) => {
                let element = element(self.document, node.element);
                match node.pseudo {
                    None => Some(self.sheet.computed_with_resolver(
                        element,
                        &parent,
                        self.resolver,
                    )?),
                    Some(
                        pseudo @ (PseudoElement::Before
                        | PseudoElement::After
                        | PseudoElement::FootnoteCall
                        | PseudoElement::FootnoteMarker
                        | PseudoElement::Checkmark
                        | PseudoElement::PickerIcon),
                    ) => self
                        .sheet
                        .pseudo_with_resolver(element, &parent, pseudo, self.resolver)?
                        .map(|(style, _)| style),
                    Some(
                        pseudo @ (PseudoElement::FirstLetter
                        | PseudoElement::FirstLetterPrefix
                        | PseudoElement::FirstLetterSuffix),
                    ) => Some(crate::document::formatting::inline_pseudo_style(
                        self.sheet.typographic_pseudo_with_resolver(
                            element,
                            &parent,
                            pseudo,
                            self.resolver,
                        )?,
                    )),
                    // Control internals are atomic inlines, which no first line reaches, and
                    // it reaches `::details-content` as its backing element. The scroll
                    // marker pseudo-elements inherit from originating elements outside the
                    // line their boxes may lie on.
                    Some(
                        PseudoElement::ScrollMarkerGroup
                        | PseudoElement::ScrollMarker
                        | PseudoElement::Marker
                        | PseudoElement::FirstLine
                        | PseudoElement::Placeholder
                        | PseudoElement::FileSelectorButton
                        | PseudoElement::DetailsContent
                        | PseudoElement::Absent(_),
                    ) => None,
                }
            }
            None => None,
        };
        let style = style.map(|style| style.used());
        self.styles.insert(key, style.clone());
        Ok(style)
    }
}

#[cfg(test)]
mod tests;
