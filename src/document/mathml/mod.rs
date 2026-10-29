//! Native `MathML` boxes. Coordinates are points relative to the alphabetic baseline.
mod metrics;
use crate::document::{DocumentError, Paragraph};
use crate::dom::{Element, MATHML_NAMESPACE, NodeData, TreeRelation};
use crate::graphics::{BoxDecoration, BoxPaint, DisplayItem, Rect};
use crate::images::Image;
use crate::layout::{TextLine, ViewportPaint};
use crate::resources::ResourceResolver;
use crate::style::{Display, LengthPercentage, Size, Style, StyleError, Stylesheet};
use crate::text::TextContext;
use metrics::Metrics;
use std::sync::Arc;

const MAX_MATH_DEPTH: usize = 256;
const UNCONSTRAINED_LINE: f32 = f32::MAX;

enum Paint {
    Text(TextLine),
    Box(BoxPaint),
}
#[derive(Default)]
struct MathBox {
    width: f32,
    ascent: f32,
    descent: f32,
    ink_ascent: f32,
    ink_descent: f32,
    italic: f32,
    paint: Vec<Paint>,
}
impl MathBox {
    fn place(&mut self, mut child: Self, x: f32, y: f32) {
        self.width = self.width.max(x + child.width);
        self.ascent = self.ascent.max(child.ascent - y);
        self.descent = self.descent.max(child.descent + y);
        self.ink_ascent = self.ink_ascent.max(child.ink_ascent - y);
        self.ink_descent = self.ink_descent.max(child.ink_descent + y);
        for paint in &mut child.paint {
            match paint {
                Paint::Text(line) => {
                    line.x += x;
                    line.baseline += y;
                }
                Paint::Box(paint) => {
                    paint.rect.x += x;
                    paint.rect.y += y;
                }
            }
        }
        self.paint.extend(child.paint);
    }
    fn rule(&mut self, rect: Rect, style: &Style) {
        if rect.height > 0.0 && style.visibility.is_visible() {
            self.paint.push(Paint::Box(BoxPaint::whole(
                rect,
                BoxDecoration {
                    background: style.color,
                    ..Default::default()
                },
            )));
        }
    }
}
struct Builder<'a> {
    sheet: &'a Stylesheet,
    resolver: &'a dyn ResourceResolver,
    text: TextContext<'a>,
}
fn unsupported(detail: impl Into<String>) -> DocumentError {
    StyleError::Unsupported(format!("MathML {}", detail.into())).into()
}
fn text_error(error: crate::text::TextError) -> DocumentError {
    match error {
        crate::text::TextError::Font(error) => StyleError::Font(error).into(),
        other => DocumentError::Formatting(other.to_string()),
    }
}
pub(super) fn render(
    element: Element<'_>,
    style: &Style,
    sheet: &Stylesheet,
    resolver: &dyn ResourceResolver,
) -> Result<Arc<Image>, DocumentError> {
    let builder = Builder {
        sheet,
        resolver,
        text: TextContext::new(&sheet.fonts, resolver),
    };
    let content = builder.element(element, style, 0, true)?;
    let mut lines = Vec::new();
    let mut items = Vec::new();
    for paint in content.paint {
        match paint {
            Paint::Text(mut line) => {
                line.baseline += content.ascent;
                items.push(DisplayItem::Text { line: lines.len() });
                lines.push(line);
            }
            Paint::Box(mut paint) => {
                paint.rect.y += content.ascent;
                items.push(DisplayItem::Box(paint));
            }
        }
    }
    Ok(Arc::new(Image::Viewport(Arc::new(ViewportPaint {
        lines,
        items,
        fonts: sheet.fonts.clone(),
        definitions: Default::default(),
        area: Rect {
            x: 0.0,
            y: 0.0,
            width: content.width,
            height: content.ascent + content.descent,
        },
        baseline: Some(content.ascent),
    }))))
}
impl Builder<'_> {
    fn metrics(&self, style: &Style) -> Result<Metrics, DocumentError> {
        Metrics::of(&self.sheet.fonts, style, self.resolver)
            .map_err(|error| StyleError::Font(error).into())
    }
    fn token(&self, text: &str, style: &Style) -> Result<MathBox, DocumentError> {
        let Some(paragraph) = Paragraph::generated(text, style)? else {
            return Ok(MathBox::default());
        };
        let lines = self
            .text
            .layout_paragraph(&paragraph, UNCONSTRAINED_LINE, &[])
            .map_err(text_error)?;
        if lines.len() > 1 {
            return Err(unsupported("token with forced line breaks"));
        }
        let Some(line) = lines.into_iter().next() else {
            return Ok(MathBox::default());
        };
        let bounds = self
            .text
            .outline_bounds(&paragraph, &line)
            .map_err(text_error)?;
        let (ascent, descent) =
            bounds.map_or((0.0, 0.0), |bounds| (-bounds.y, bounds.y + bounds.height));
        let mut result = MathBox {
            width: line.width,
            ascent,
            descent,
            ink_ascent: ascent,
            ink_descent: descent,
            ..Default::default()
        };
        if let [run] = line.runs.as_slice()
            && let [glyph] = run.glyphs()
            && let Some(font) = self
                .sheet
                .fonts
                .face(run.font_id)
                .and_then(|font| font.data())
            && let Ok(face) = ttf_parser::Face::parse(font, 0)
        {
            result.italic = face
                .tables()
                .math
                .and_then(|table| table.glyph_info)
                .and_then(|info| info.italic_corrections)
                .and_then(|values| values.get(ttf_parser::GlyphId(glyph.id as u16)))
                .map_or(0.0, |value| {
                    f32::from(value.value) * run.font_size / f32::from(face.units_per_em())
                });
        }
        if style.visibility.is_visible() {
            result.paint.push(Paint::Text(TextLine {
                paragraph: 0,
                text: line.text,
                x: 0.0,
                baseline: 0.0,
                width: line.width,
                height: ascent + descent,
                runs: line.runs,
                font_size: style.size,
                color: style.color,
            }));
        }
        Ok(result)
    }
    fn row(
        &self,
        element: Element<'_>,
        style: &Style,
        depth: usize,
    ) -> Result<MathBox, DocumentError> {
        let mut result = MathBox::default();
        for node in element.as_node().children(TreeRelation::Formatting) {
            let child = match node.value() {
                NodeData::Text(text) if !text.trim().is_empty() => self.token(text, style)?,
                NodeData::Element(_) => self.element(
                    node.element().expect("element node"),
                    style,
                    depth + 1,
                    false,
                )?,
                _ => continue,
            };
            let x = result.width;
            result.place(child, x, 0.0);
        }
        Ok(result)
    }
    fn element(
        &self,
        element: Element<'_>,
        parent: &Style,
        depth: usize,
        root: bool,
    ) -> Result<MathBox, DocumentError> {
        if depth >= MAX_MATH_DEPTH {
            return Err(DocumentError::Formatting(format!(
                "MathML nesting exceeds {MAX_MATH_DEPTH}"
            )));
        }
        if element.namespace() != MATHML_NAMESPACE {
            return Err(unsupported(format!("foreign content {}", element.name())));
        }
        let computed;
        let style = if root {
            parent
        } else {
            computed = self
                .sheet
                .computed_without_box(element, parent, self.resolver)?;
            &computed
        };
        if style.display == Display::None {
            return Ok(MathBox::default());
        }
        if !matches!(style.display, Display::BlockMath | Display::InlineMath) {
            return Err(unsupported(format!(
                "CSS display {:?} on {}",
                style.display,
                element.name()
            )));
        }
        if style.position.is_out_of_flow()
            || style.floating().is_some()
            || style.has_transform_effects()
            || style.is_filtered()
            || style.has_backdrop_filter()
        {
            return Err(unsupported(format!(
                "positioning or effects on {}",
                element.name()
            )));
        }
        let mut content = match element.name() {
            "math" | "mrow" | "mstyle" | "merror" | "mphantom" | "semantics" | "maction" => {
                self.row(element, style, depth)?
            }
            "mi" | "mn" | "mtext" | "ms" | "annotation" => {
                let mut text = String::new();
                for node in element.as_node().children(TreeRelation::Formatting) {
                    match node.value() {
                        NodeData::Text(value) => text.push_str(value),
                        NodeData::Element(_) => {
                            return Err(unsupported(format!("non-text token {}", element.name())));
                        }
                        _ => {}
                    }
                }
                self.token(&text, style)?
            }
            "mspace" => self.space(element, style)?,
            "mpadded" => self.padded(element, style, depth)?,
            "mfrac" => self.fraction(element, style, depth)?,
            "msub" | "msup" | "msubsup" => self.scripts(element, style, depth)?,
            other => return Err(unsupported(other)),
        };
        if !root {
            content = Self::edges(content, style)?;
        }
        Ok(content)
    }
    fn children(
        &self,
        element: Element<'_>,
        style: &Style,
        depth: usize,
    ) -> Result<Vec<MathBox>, DocumentError> {
        let mut boxes = Vec::new();
        for child in element.children(TreeRelation::Formatting) {
            let computed = self
                .sheet
                .computed_without_box(child, style, self.resolver)?;
            if computed.display != Display::None {
                boxes.push(self.element(child, style, depth + 1, false)?);
            }
        }
        Ok(boxes)
    }
    fn attribute(
        &self,
        element: Element<'_>,
        name: &str,
        style: &Style,
    ) -> Result<Option<f32>, DocumentError> {
        let Some(source) = element.attr(name) else {
            return Ok(None);
        };
        Ok(
            match self
                .sheet
                .math_attribute_length(source, style, self.resolver)?
            {
                Some(LengthPercentage::Length(value)) => Some(value),
                _ => None,
            },
        )
    }
    fn space(&self, element: Element<'_>, style: &Style) -> Result<MathBox, DocumentError> {
        let ascent = self
            .attribute(element, "height", style)?
            .unwrap_or(0.0)
            .max(0.0);
        let depth = self
            .attribute(element, "depth", style)?
            .unwrap_or(0.0)
            .max(0.0);
        let width = match &style.width {
            Size::Value(LengthPercentage::Length(value)) => *value,
            Size::Auto => self
                .attribute(element, "width", style)?
                .unwrap_or(0.0)
                .max(0.0),
            _ => return Err(unsupported("intrinsic or percentage mspace width")),
        };
        let height = match &style.height {
            Size::Value(LengthPercentage::Length(value)) => *value,
            Size::Auto => ascent + depth,
            _ => return Err(unsupported("intrinsic or percentage mspace height")),
        };
        Ok(MathBox {
            width,
            ascent,
            descent: height - ascent,
            ink_ascent: ascent,
            ink_descent: height - ascent,
            ..Default::default()
        })
    }
    fn padded(
        &self,
        element: Element<'_>,
        style: &Style,
        depth: usize,
    ) -> Result<MathBox, DocumentError> {
        let inner = self.row(element, style, depth)?;
        let width = match &style.width {
            Size::Value(LengthPercentage::Length(value)) => *value,
            Size::Auto => self
                .attribute(element, "width", style)?
                .unwrap_or(inner.width)
                .max(0.0),
            _ => return Err(unsupported("intrinsic or percentage mpadded width")),
        };
        let ascent = self
            .attribute(element, "height", style)?
            .unwrap_or(inner.ascent)
            .max(0.0);
        let descent = self
            .attribute(element, "depth", style)?
            .unwrap_or(inner.ascent)
            .max(0.0);
        let x = self
            .attribute(element, "lspace", style)?
            .unwrap_or(0.0)
            .max(0.0);
        let y = self.attribute(element, "voffset", style)?.unwrap_or(0.0);
        let mut result = MathBox::default();
        result.place(inner, x, -y);
        result.width = width;
        result.ascent = ascent;
        result.descent = descent;
        result.ink_ascent = ascent;
        result.ink_descent = descent;
        Ok(result)
    }
    fn fraction(
        &self,
        element: Element<'_>,
        style: &Style,
        depth: usize,
    ) -> Result<MathBox, DocumentError> {
        let mut children = self.children(element, style, depth)?;
        if children.len() != 2 {
            return self.row(element, style, depth);
        }
        let denominator = children.pop().expect("two fraction children");
        let numerator = children.pop().expect("two fraction children");
        let metrics = self.metrics(style)?;
        let thickness = element
            .attr("linethickness")
            .map(|source| {
                self.sheet
                    .math_attribute_length(source, style, self.resolver)
            })
            .transpose()?
            .flatten()
            .map_or(metrics.rule, |value| value.resolve(metrics.rule))
            .max(0.0);
        let index = usize::from(style.math.compact);
        let (up, down) = if thickness > 0.0 {
            (
                metrics.numerator[index].max(
                    metrics.axis
                        + thickness / 2.0
                        + metrics.numerator_gap[index]
                        + numerator.ink_descent,
                ),
                metrics.denominator[index].max(
                    thickness / 2.0 + metrics.denominator_gap[index] + denominator.ink_ascent
                        - metrics.axis,
                ),
            )
        } else {
            let up = metrics.stack_top[index];
            let down = metrics.stack_bottom[index];
            let gap = up - numerator.ink_descent + down - denominator.ink_ascent;
            let adjustment = (metrics.stack_gap[index] - gap).max(0.0) / 2.0;
            (up + adjustment, down + adjustment)
        };
        let width = numerator.width.max(denominator.width);
        let mut result = MathBox::default();
        let numerator_x = (width - numerator.width) / 2.0;
        let denominator_x = (width - denominator.width) / 2.0;
        result.place(numerator, numerator_x, -up);
        result.place(denominator, denominator_x, down);
        if thickness > 0.0 {
            result.ascent = result.ascent.max(metrics.axis + thickness / 2.0);
            result.descent = result.descent.max(-metrics.axis + thickness / 2.0);
            result.ink_ascent = result.ink_ascent.max(metrics.axis + thickness / 2.0);
            result.ink_descent = result.ink_descent.max(-metrics.axis + thickness / 2.0);
        }
        result.rule(
            Rect {
                x: 0.0,
                y: -metrics.axis - thickness / 2.0,
                width,
                height: thickness,
            },
            style,
        );
        Ok(result)
    }
    fn scripts(
        &self,
        element: Element<'_>,
        style: &Style,
        depth: usize,
    ) -> Result<MathBox, DocumentError> {
        let children = self.children(element, style, depth)?;
        let paired = element.name() == "msubsup";
        if children.len() != if paired { 3 } else { 2 } {
            return self.row(element, style, depth);
        }
        let mut children = children.into_iter();
        let base = children.next().expect("script base");
        let mut sub = None;
        let mut sup = None;
        if element.name() != "msup" {
            sub = children.next();
        }
        if element.name() != "msub" {
            sup = children.next();
        }
        let metrics = self.metrics(style)?;
        let mut down = sub.as_ref().map_or(0.0, |sub| {
            metrics
                .sub_shift
                .max(sub.ink_ascent - metrics.sub_top)
                .max(metrics.sub_drop + base.ink_descent)
        });
        let mut up = sup.as_ref().map_or(0.0, |sup| {
            metrics.sup_shift[usize::from(style.math.shift)]
                .max(metrics.sup_bottom + sup.ink_descent)
                .max(base.ink_ascent - metrics.sup_drop)
        });
        if let (Some(sub), Some(sup)) = (&sub, &sup) {
            let gap = down - sub.ink_ascent + up - sup.ink_descent;
            if gap < metrics.script_gap {
                let adjustment = (metrics.sup_bottom_with_sub - (up - sup.ink_descent))
                    .max(0.0)
                    .min(metrics.script_gap - gap);
                up += adjustment;
                down += (metrics.script_gap - gap - adjustment).max(0.0);
            }
        }
        let x = base.width;
        let italic = base.italic;
        let mut result = MathBox::default();
        result.place(base, 0.0, 0.0);
        if let Some(sub) = sub {
            result.place(sub, x, down);
        }
        if let Some(sup) = sup {
            result.place(sup, x + italic, -up);
        }
        result.width += metrics.after_script;
        Ok(result)
    }
    fn edges(content: MathBox, style: &Style) -> Result<MathBox, DocumentError> {
        let length = |value: &LengthPercentage| match value {
            LengthPercentage::Length(value) => Ok(*value),
            _ => Err(unsupported("percentage edges")),
        };
        let padding: Vec<_> = style.padding.iter().map(length).collect::<Result<_, _>>()?;
        let margins: Vec<_> = style
            .margin
            .iter()
            .map(|value| match value {
                Size::Auto => Ok(0.0),
                Size::Value(value) => length(value),
                Size::Intrinsic(_) => Err(unsupported("intrinsic margins")),
            })
            .collect::<Result<_, _>>()?;
        let borders = style
            .decoration
            .borders
            .map(crate::graphics::Border::used_width);
        let top = padding[0] + borders[0];
        let right = padding[1] + borders[1];
        let bottom = padding[2] + borders[2];
        let left = padding[3] + borders[3];
        let border_rect = Rect {
            x: margins[3],
            y: -content.ascent - top,
            width: left + content.width + right,
            height: top + content.ascent + content.descent + bottom,
        };
        let mut result = MathBox {
            ascent: content.ascent + top + margins[0],
            descent: content.descent + bottom + margins[2],
            ..Default::default()
        };
        if style.visibility.is_visible() {
            let paint = BoxPaint::whole(border_rect, style.decoration.with_image_type());
            if paint.has_ink() {
                result.paint.push(Paint::Box(paint));
            }
        }
        result.place(content, margins[3] + left, 0.0);
        result.width += right + margins[1];
        Ok(result)
    }
}
