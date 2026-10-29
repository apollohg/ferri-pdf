//! Native `MathML` boxes. Coordinates are points relative to the alphabetic baseline.
mod metrics;
mod operator;
mod operator_data;
mod schedule;
mod stretch;
use crate::document::{DocumentError, Paragraph};
use crate::dom::{Element, MATHML_NAMESPACE, NodeData, TreeRelation};
use crate::graphics::{BoxDecoration, BoxPaint, DisplayItem, Rect};
use crate::images::Image;
use crate::layout::{TextLine, ViewportPaint};
use crate::resources::ResourceResolver;
use crate::style::{
    Display, IntrinsicWidths, LengthPercentage, Size, Style, StyleError, Stylesheet,
};
use crate::text::TextContext;
use metrics::Metrics;
use operator::{Form, Operator};
use schedule::{MathError, Requests};
use std::collections::HashMap;
use std::sync::Arc;

const MAX_MATH_DEPTH: usize = 256;
const UNCONSTRAINED_LINE: f32 = f32::MAX;

#[derive(Clone)]
enum Paint {
    Text(TextLine),
    Box(BoxPaint),
}
#[derive(Clone, Default)]
struct MathBox {
    width: f32,
    intrinsic: IntrinsicWidths,
    ascent: f32,
    descent: f32,
    ink_ascent: f32,
    ink_descent: f32,
    italic: Option<f32>,
    accent: Option<f32>,
    operator: Option<Operator>,
    space_like: bool,
    paint: Vec<Paint>,
}
impl MathBox {
    fn place(&mut self, child: Self, x: f32, y: f32) {
        self.width = self.width.max(x + child.width);
        self.ascent = self.ascent.max(child.ascent - y);
        self.descent = self.descent.max(child.descent + y);
        self.ink_ascent = self.ink_ascent.max(child.ink_ascent - y);
        self.ink_descent = self.ink_descent.max(child.ink_descent + y);
        self.position(child, x, y);
    }
    fn position(&mut self, mut child: Self, x: f32, y: f32) {
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
#[derive(Clone, Copy)]
enum Stretch {
    Inline(f32),
    Block(f32, f32),
}
#[derive(Clone, Copy, Default)]
struct MathLayout {
    form: Option<Form>,
    stretch: Option<Stretch>,
}
#[derive(Clone, Copy, PartialEq)]
enum RowKind {
    Grouping,
    Fraction,
    Scripted,
    Other,
}
fn is_scripted(name: &str) -> bool {
    matches!(
        name,
        "msub" | "msup" | "msubsup" | "munder" | "mover" | "munderover" | "mmultiscripts"
    )
}
fn script_children(
    children: Vec<MathBox>,
    below: bool,
    above: bool,
) -> (MathBox, Option<MathBox>, Option<MathBox>) {
    let mut children = children.into_iter();
    let base = children.next().expect("script base");
    let under = below.then(|| children.next()).flatten();
    let over = above.then(|| children.next()).flatten();
    (base, under, over)
}
fn script_pairs(
    children: &mut impl Iterator<Item = MathBox>,
    count: usize,
) -> Vec<(MathBox, MathBox)> {
    (0..count)
        .map(|_| {
            (
                children.next().expect("valid subscript"),
                children.next().expect("valid superscript"),
            )
        })
        .collect()
}
struct Builder<'a, 'd> {
    sheet: &'a Stylesheet,
    resolver: &'a dyn ResourceResolver,
    text: TextContext<'a>,
    jobs: Vec<schedule::Job<'d>>,
    cache: HashMap<schedule::Key, MathBox>,
}
fn unsupported<'d>(detail: impl Into<String>) -> MathError<'d> {
    StyleError::Unsupported(format!("MathML {}", detail.into())).into()
}
fn text_error<'d>(error: crate::text::TextError) -> MathError<'d> {
    match error {
        crate::text::TextError::Font(error) => StyleError::Font(error).into(),
        other => DocumentError::Formatting(other.to_string()).into(),
    }
}
pub(super) fn render(
    element: Element<'_>,
    style: &Style,
    sheet: &Stylesheet,
    resolver: &dyn ResourceResolver,
) -> Result<Arc<Image>, DocumentError> {
    let mut builder = Builder {
        sheet,
        resolver,
        text: TextContext::new(&sheet.fonts, resolver),
        jobs: Vec::new(),
        cache: HashMap::new(),
    };
    let content = builder.run(element, style)?;
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
        intrinsic_widths: Some(content.intrinsic),
    }))))
}
impl<'d> Builder<'_, 'd> {
    fn metrics(&self, style: &Style) -> Result<Metrics, MathError<'d>> {
        Metrics::of(&self.sheet.fonts, style, self.resolver)
            .map_err(|error| StyleError::Font(error).into())
    }
    fn token(&self, text: &str, style: &Style) -> Result<MathBox, MathError<'d>> {
        let Some(paragraph) = Paragraph::generated(text, style)? else {
            return Ok(MathBox::default());
        };
        self.shape_token(&paragraph, style)
    }
    fn token_element(&self, element: Element<'d>, style: &Style) -> Result<MathBox, MathError<'d>> {
        let mut collector = crate::document::inline::InlineCollector::default();
        for text in Self::text_nodes(element)? {
            collector.append(text, style)?;
        }
        let Ok(paragraph) = collector.finish(style)? else {
            return Ok(MathBox::default());
        };
        self.shape_token(&paragraph, style)
    }
    fn shape_token(&self, paragraph: &Paragraph, style: &Style) -> Result<MathBox, MathError<'d>> {
        let lines = self
            .text
            .layout_paragraph(paragraph, UNCONSTRAINED_LINE, &[])
            .map_err(text_error)?;
        if lines.len() > 1 {
            return Err(unsupported("token with forced line breaks"));
        }
        let Some(line) = lines.into_iter().next() else {
            return Ok(MathBox::default());
        };
        let bounds = self
            .text
            .outline_bounds(paragraph, &line)
            .map_err(text_error)?;
        let (ascent, descent) =
            bounds.map_or((0.0, 0.0), |bounds| (-bounds.y, bounds.y + bounds.height));
        let mut result = MathBox {
            width: line.width,
            intrinsic: IntrinsicWidths {
                min: line.width,
                max: line.width,
            },
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
                .map(|value| {
                    f32::from(value.value) * run.font_size / f32::from(face.units_per_em())
                });
            result.accent = face
                .tables()
                .math
                .and_then(|table| table.glyph_info)
                .and_then(|info| info.top_accent_attachments)
                .and_then(|values| values.get(ttf_parser::GlyphId(glyph.id as u16)))
                .map(|value| {
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
        &mut self,
        element: Element<'d>,
        style: &Arc<Style>,
        depth: usize,
        layout: MathLayout,
        kind: RowKind,
    ) -> Result<MathBox, MathError<'d>> {
        let mut children = Vec::new();
        let mut requests = Requests::default();
        for node in element.as_node().children(TreeRelation::Formatting) {
            let child = match node.value() {
                NodeData::Text(text) if !text.chars().all(crate::document::is_css_whitespace) => {
                    self.token(text, style).map(|mut child| {
                        child.space_like = true;
                        child
                    })
                }
                NodeData::Element(_) => self.element(
                    node.element().expect("element node"),
                    style,
                    depth + 1,
                    false,
                    MathLayout::default(),
                ),
                _ => continue,
            };
            if let Some(child) = requests.collect(child)? {
                children.push((node.element(), child));
            }
        }
        requests.finish()?;
        let significant: Vec<_> = children
            .iter()
            .enumerate()
            .filter(|(_, (_, child))| !child.space_like)
            .map(|(index, _)| index)
            .collect();
        let scripted = kind == RowKind::Scripted;
        let grouping = kind == RowKind::Grouping;
        let first_base = scripted || kind == RowKind::Fraction;
        let core = if first_base {
            let first = self
                .in_flow_children(element, style)?
                .first()
                .map(|child| child.id());
            children
                .iter()
                .position(|(source, _)| source.is_none_or(|source| Some(source.id()) == first))
                .filter(|&index| children[index].1.operator.is_some())
        } else if grouping {
            significant
                .first()
                .copied()
                .filter(|_| significant.len() == 1)
                .filter(|&index| children[index].1.operator.is_some())
        } else {
            None
        };
        let embellished = core.is_some();
        let mut requests = Requests::default();
        for (index, (source, child)) in children.iter_mut().enumerate() {
            let Some(operator) = &child.operator else {
                continue;
            };
            let form = if core == Some(index) {
                layout.form.unwrap_or(Form::Infix)
            } else if scripted {
                Form::Postfix
            } else if grouping && significant.len() > 1 && significant.first() == Some(&index) {
                Form::Prefix
            } else if grouping && significant.len() > 1 && significant.last() == Some(&index) {
                Form::Postfix
            } else {
                Form::Infix
            };
            let inherited_stretch = if core == Some(index) {
                layout.stretch
            } else {
                None
            };
            if ((!operator.explicit_form && operator.form != form) || inherited_stretch.is_some())
                && let Some(resolved) = requests.collect(self.element(
                    source.expect("only elements are embellished"),
                    style,
                    depth + 1,
                    false,
                    MathLayout {
                        form: Some(form),
                        stretch: inherited_stretch,
                    },
                ))?
            {
                *child = resolved;
            }
        }
        requests.finish()?;
        if layout.stretch.is_none() {
            let (ascent, descent) = children
                .iter()
                .filter(|(_, child)| {
                    !child
                        .operator
                        .as_ref()
                        .is_some_and(|operator| operator.stretchy && !operator.inline_axis)
                })
                .fold((0.0f32, 0.0f32), |(ascent, descent), (_, child)| {
                    (ascent.max(child.ink_ascent), descent.max(child.ink_descent))
                });
            let mut requests = Requests::default();
            for (source, child) in &mut children {
                if let Some(operator) = &child.operator
                    && operator.stretchy
                    && !operator.inline_axis
                    && let Some(resolved) = requests.collect(self.element(
                        source.expect("operator source"),
                        style,
                        depth + 1,
                        false,
                        MathLayout {
                            form: Some(operator.form),
                            stretch: Some(Stretch::Block(ascent, descent)),
                        },
                    ))?
                {
                    *child = resolved;
                }
            }
            requests.finish()?;
        }
        let add_space = element.name() == "math" || !embellished;
        let mut positions = Vec::with_capacity(children.len());
        let mut width = 0.0;
        let mut intrinsic = IntrinsicWidths::default();
        let mut previous_italic = 0.0;
        for (_, child) in &children {
            let slanted =
                child.operator.is_none() && child.italic.is_some_and(|value| value != 0.0);
            if !slanted {
                width += previous_italic;
                intrinsic.min += previous_italic;
                intrinsic.max += previous_italic;
            }
            if add_space && let Some(operator) = &child.operator {
                width += operator.space[0];
                intrinsic.min += operator.space[0];
                intrinsic.max += operator.space[0];
            }
            positions.push(width);
            width += child.width;
            intrinsic.min += child.intrinsic.min;
            intrinsic.max += child.intrinsic.max;
            previous_italic = if slanted {
                child.italic.unwrap_or(0.0)
            } else {
                0.0
            };
            if add_space && let Some(operator) = &child.operator {
                width += operator.space[1];
                intrinsic.min += operator.space[1];
                intrinsic.max += operator.space[1];
            }
        }
        width += previous_italic;
        intrinsic.min += previous_italic;
        intrinsic.max += previous_italic;
        let ascent = children
            .iter()
            .map(|(_, child)| child.ascent)
            .reduce(f32::max)
            .unwrap_or(0.0);
        let descent = children
            .iter()
            .map(|(_, child)| child.descent)
            .reduce(f32::max)
            .unwrap_or(0.0);
        let ink_ascent = children
            .iter()
            .map(|(_, child)| child.ink_ascent)
            .reduce(f32::max)
            .unwrap_or(0.0);
        let ink_descent = children
            .iter()
            .map(|(_, child)| child.ink_descent)
            .reduce(f32::max)
            .unwrap_or(0.0);
        let mut result = MathBox {
            width,
            intrinsic,
            ascent,
            descent,
            ink_ascent,
            ink_descent,
            italic: Some(previous_italic),
            space_like: grouping && significant.is_empty(),
            operator: core.and_then(|index| children[index].1.operator.clone()),
            ..Default::default()
        };
        for ((_, child), x) in children.into_iter().zip(positions) {
            let x = if style.rtl {
                width - x - child.width
            } else {
                x
            };
            result.place(child, x, 0.0);
        }
        Ok(result)
    }
    fn layout_element(
        &mut self,
        element: Element<'d>,
        style: &Arc<Style>,
        depth: usize,
        root: bool,
        layout: MathLayout,
    ) -> Result<MathBox, MathError<'d>> {
        if element.namespace() != MATHML_NAMESPACE {
            return Err(unsupported(format!("foreign content {}", element.name())));
        }
        if style.display == Display::None {
            return Ok(MathBox {
                space_like: true,
                ..Default::default()
            });
        }
        if !matches!(style.display, Display::BlockMath | Display::InlineMath) {
            return Err(unsupported(format!(
                "CSS display {:?} on {}",
                style.display,
                element.name()
            )));
        }
        if !root
            && (style.position.is_out_of_flow()
                || style.floating().is_some()
                || style.has_transform_effects()
                || style.is_filtered()
                || style.has_backdrop_filter())
        {
            return Err(unsupported(format!(
                "positioning or effects on {}",
                element.name()
            )));
        }
        let mut content = match element.name() {
            "math" | "mrow" | "mstyle" | "merror" | "mphantom" | "semantics" | "maction"
            | "mprescripts" => self.row(element, style, depth, layout, RowKind::Grouping)?,
            "mi" | "mn" | "mtext" | "ms" | "annotation" => self.token_element(element, style)?,
            "mo" => self.operator(element, style, layout)?,
            "mspace" => self.space(element, style)?,
            "mpadded" => self.padded(element, style, depth, layout)?,
            "mfrac" => self.fraction(element, style, depth, layout)?,
            "msqrt" | "mroot" => self.radical(element, style, depth, layout)?,
            "msub" | "msup" | "msubsup" => self.scripts(element, style, depth, layout)?,
            "munder" | "mover" | "munderover" => self.limits(element, style, depth, layout)?,
            "mmultiscripts" => self.multiscripts(element, style, depth, layout)?,
            "annotation-xml" | "mtable" | "mtr" | "mtd" => return Err(unsupported(element.name())),
            _ => self.row(element, style, depth, layout, RowKind::Grouping)?,
        };
        if matches!(
            element.name(),
            "mspace" | "mtext" | "annotation" | "annotation-xml"
        ) {
            content.space_like = true;
        }
        if !root {
            content = Self::edges(content, style)?;
        }
        Ok(content)
    }
    fn in_flow_children(
        &self,
        element: Element<'d>,
        style: &Style,
    ) -> Result<Vec<Element<'d>>, MathError<'d>> {
        let mut children = Vec::new();
        for child in element.children(TreeRelation::Formatting) {
            if self
                .sheet
                .computed_with_resolver(child, style, self.resolver)?
                .display
                != Display::None
            {
                children.push(child);
            }
        }
        Ok(children)
    }
    fn children(
        &mut self,
        element: Element<'d>,
        style: &Arc<Style>,
        depth: usize,
        layout: MathLayout,
    ) -> Result<Vec<MathBox>, MathError<'d>> {
        let mut boxes = Vec::new();
        let mut requests = Requests::default();
        for (index, child) in self
            .in_flow_children(element, style)?
            .into_iter()
            .enumerate()
        {
            let child_layout = if index == 0 {
                layout
            } else {
                MathLayout {
                    form: is_scripted(element.name()).then_some(Form::Postfix),
                    stretch: None,
                }
            };
            if let Some(child) =
                requests.collect(self.element(child, style, depth + 1, false, child_layout))?
            {
                boxes.push(child);
            }
        }
        requests.finish()?;
        Ok(boxes)
    }
    fn text_nodes(element: Element<'d>) -> Result<Vec<&'d str>, MathError<'d>> {
        let mut nodes = Vec::new();
        for node in element.as_node().children(TreeRelation::Formatting) {
            match node.value() {
                NodeData::Text(value) => nodes.push(value.as_ref()),
                NodeData::Element(_) => {
                    return Err(unsupported(format!("non-text token {}", element.name())));
                }
                _ => {}
            }
        }
        Ok(nodes)
    }
    fn stretch_character(
        &self,
        character: char,
        style: &Style,
        target: f32,
        horizontal: bool,
        variants_only: bool,
    ) -> Result<Option<MathBox>, MathError<'d>> {
        let mut visible = style.clone();
        visible.visibility = crate::style::Visibility::Visible;
        let mut normal = self.token(&character.to_string(), &visible)?;
        let Some(Paint::Text(line)) = normal.paint.first_mut() else {
            return Ok(None);
        };
        let [run] = line.runs.as_mut_slice() else {
            return Ok(None);
        };
        if run.font_size == 0.0 {
            return Ok(Some(normal));
        }
        let mut selected = None;
        for &id in style.font_candidates() {
            if let Some((instance, _)) = self
                .sheet
                .fonts
                .sized_instance(
                    id,
                    style.font_selection(),
                    &style.font_adjustments,
                    style.size,
                    self.resolver,
                )
                .map_err(StyleError::Font)?
            {
                selected = Some((id, instance));
                break;
            }
        }
        let Some((id, instance)) = selected else {
            return Ok(None);
        };
        if id != run.font_id {
            return Ok(None);
        }
        let [glyph] = run.glyphs() else {
            return Ok(None);
        };
        let Some(glyph) = u16::try_from(glyph.id).ok().map(ttf_parser::GlyphId) else {
            return Ok(None);
        };
        let Some(shape) = stretch::shape(
            &instance.face,
            glyph,
            target / run.font_size,
            horizontal,
            variants_only,
        )?
        else {
            return Ok(None);
        };
        let size = run.font_size;
        let width = shape.width * size;
        run.set_math_glyphs(shape.glyphs, width);
        line.width = width;
        line.height = (shape.ascent + shape.descent) * size;
        normal.width = width;
        normal.ascent = shape.ascent * size;
        normal.descent = shape.descent * size;
        normal.ink_ascent = normal.ascent;
        normal.ink_descent = normal.descent;
        normal.italic = shape.italic.map(|value| value * size);
        if variants_only {
            normal.intrinsic = IntrinsicWidths {
                min: width,
                max: width,
            };
        } else if !horizontal
            && let Some(preferred) = stretch::preferred_inline_size(&instance.face, glyph)
        {
            normal.intrinsic = IntrinsicWidths {
                min: preferred * size,
                max: preferred * size,
            };
        }
        if !style.visibility.is_visible() {
            normal.paint.clear();
        }
        Ok(Some(normal))
    }
    fn operator(
        &mut self,
        element: Element<'d>,
        style: &Arc<Style>,
        layout: MathLayout,
    ) -> Result<MathBox, MathError<'d>> {
        let text = Self::text_nodes(element)?.concat();
        let operator = Operator::of(
            element,
            &text,
            style,
            layout.form.unwrap_or_default(),
            self.sheet,
            self.resolver,
        )?;
        let mut result = self.token_element(element, style)?;
        let mut chars = text.chars();
        if let Some(character) = chars.next().filter(|_| chars.next().is_none()) {
            let metrics = self.metrics(style)?;
            if operator.stretchy {
                if !operator.inline_axis
                    && let Some(normal) =
                        self.stretch_character(character, style, 0.0, false, false)?
                {
                    result.intrinsic = normal.intrinsic;
                }
                match layout.stretch {
                    Some(Stretch::Inline(size)) if operator.inline_axis => {
                        if let Some(stretched) =
                            self.stretch_character(character, style, size, true, false)?
                        {
                            result = stretched;
                        }
                    }
                    Some(Stretch::Block(ascent, descent)) if !operator.inline_axis => {
                        let (mut ascent, mut descent) = if operator.symmetric {
                            let extent = (ascent - metrics.axis).max(descent + metrics.axis);
                            (extent + metrics.axis, extent - metrics.axis)
                        } else {
                            (ascent, descent)
                        };
                        let height = result.ascent + result.descent;
                        let minimum = operator.min.resolve(height).max(0.0);
                        let maximum = operator
                            .max
                            .as_ref()
                            .map_or(f32::INFINITY, |value| value.resolve(height))
                            .max(minimum);
                        let target = ascent + descent;
                        if target <= 0.0 {
                            ascent = minimum / 2.0 + metrics.axis;
                            descent = minimum - ascent;
                        } else if target < minimum || target > maximum {
                            let size = target.clamp(minimum, maximum);
                            ascent =
                                ((ascent - metrics.axis) * size / target + metrics.axis).max(0.0);
                            descent = size - ascent;
                        }
                        if let Some(stretched) = self.stretch_character(
                            character,
                            style,
                            ascent + descent,
                            false,
                            false,
                        )? {
                            let delta =
                                ((stretched.ascent - stretched.descent) - (ascent - descent)) / 2.0;
                            let (width, top, bottom, italic) = (
                                stretched.width,
                                stretched.ascent - delta,
                                stretched.descent + delta,
                                stretched.italic,
                            );
                            let mut centred = MathBox {
                                intrinsic: stretched.intrinsic,
                                ..Default::default()
                            };
                            centred.place(stretched, 0.0, delta);
                            centred.width = width;
                            centred.ascent = top;
                            centred.descent = bottom;
                            centred.ink_ascent = top;
                            centred.ink_descent = bottom;
                            centred.italic = italic;
                            result = centred;
                        }
                    }
                    _ => {}
                }
            } else if operator.large
                && !style.math.compact
                && let Some(stretched) =
                    self.stretch_character(character, style, metrics.display_operator, false, true)?
            {
                result = stretched;
            }
        }
        result.operator = Some(operator);
        Ok(result)
    }
    fn attribute(
        &self,
        element: Element<'d>,
        name: &str,
        style: &Style,
    ) -> Result<Option<f32>, MathError<'d>> {
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
    fn space(&self, element: Element<'d>, style: &Style) -> Result<MathBox, MathError<'d>> {
        let ascent = self
            .attribute(element, "height", style)?
            .unwrap_or(0.0)
            .max(0.0);
        let width = match &style.width {
            Size::Value(LengthPercentage::Length(value)) => *value,
            Size::Auto => 0.0,
            _ => return Err(unsupported("intrinsic or percentage mspace width")),
        };
        let height = match &style.height {
            Size::Value(LengthPercentage::Length(value)) => *value,
            Size::Auto => 0.0,
            _ => return Err(unsupported("intrinsic or percentage mspace height")),
        };
        Ok(MathBox {
            width,
            intrinsic: IntrinsicWidths {
                min: width,
                max: width,
            },
            ascent,
            descent: height - ascent,
            ink_ascent: ascent,
            ink_descent: height - ascent,
            ..Default::default()
        })
    }
    fn padded(
        &mut self,
        element: Element<'d>,
        style: &Arc<Style>,
        depth: usize,
        layout: MathLayout,
    ) -> Result<MathBox, MathError<'d>> {
        let inner = self.row(element, style, depth, layout, RowKind::Grouping)?;
        let width = match &style.width {
            Size::Value(LengthPercentage::Length(value)) => *value,
            Size::Auto => inner.width,
            _ => return Err(unsupported("intrinsic or percentage mpadded width")),
        };
        let intrinsic = if style.width == Size::Auto {
            inner.intrinsic
        } else {
            IntrinsicWidths {
                min: width,
                max: width,
            }
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
        let mut result = MathBox {
            operator: inner.operator.clone(),
            space_like: inner.space_like,
            ..Default::default()
        };
        result.place(inner, x, -y);
        result.width = width;
        result.intrinsic = intrinsic;
        result.ascent = ascent;
        result.descent = descent;
        result.ink_ascent = ascent;
        result.ink_descent = descent;
        Ok(result)
    }
    fn radical(
        &mut self,
        element: Element<'d>,
        style: &Arc<Style>,
        depth: usize,
        layout: MathLayout,
    ) -> Result<MathBox, MathError<'d>> {
        let (base, index) = if element.name() == "mroot" {
            let mut children = self.children(element, style, depth, MathLayout::default())?;
            if children.len() != 2 {
                return self.row(element, style, depth, layout, RowKind::Other);
            }
            let index = children.pop().expect("root index");
            (children.pop().expect("root base"), Some(index))
        } else {
            (
                self.row(
                    element,
                    style,
                    depth,
                    MathLayout::default(),
                    RowKind::Grouping,
                )?,
                None,
            )
        };
        let metrics = self.metrics(style)?;
        let gap = metrics.radical_gap[usize::from(style.math.compact)];
        let target = metrics.radical_rule + gap + base.ink_ascent + base.ink_descent;
        let surd = self
            .stretch_character('√', style, target, false, false)?
            .map_or_else(|| self.token("√", style), Ok)?;
        let width = surd.width + base.width;
        let intrinsic = IntrinsicWidths {
            min: surd.intrinsic.min + base.intrinsic.min,
            max: surd.intrinsic.max + base.intrinsic.max,
        };
        let ascent = base
            .ascent
            .max(base.ink_ascent + gap + metrics.radical_rule + metrics.radical_extra);
        let descent = base
            .descent
            .max(surd.ascent + surd.descent + metrics.radical_extra - ascent);
        let surd_y = -ascent + metrics.radical_extra + surd.ascent;
        let base_x = surd.width;
        let base_width = base.width;
        let mut result = MathBox {
            width,
            intrinsic,
            ascent,
            descent,
            ink_ascent: ascent,
            ink_descent: descent,
            ..Default::default()
        };
        result.place(base, if style.rtl { 0.0 } else { base_x }, 0.0);
        let content_end = result.paint.len();
        result.rule(
            Rect {
                x: if style.rtl { 0.0 } else { base_x },
                y: -ascent + metrics.radical_extra,
                width: base_width,
                height: metrics.radical_rule,
            },
            style,
        );
        result.place(surd, if style.rtl { width - base_x } else { 0.0 }, surd_y);
        if let Some(index) = index {
            let graphics = MathBox {
                paint: result.paint.split_off(content_end),
                ..Default::default()
            };
            let before = metrics.radical_kern[0].max(0.0);
            let after = metrics.radical_kern[1].max(-index.width);
            let base_offset = before + index.width + after;
            let total_width = width + base_offset;
            let index_y = descent - metrics.radical_raise * (ascent + descent) - index.descent;
            let top =
                ascent.max(-descent + metrics.radical_raise * (ascent + descent) + index.ascent);
            let bottom =
                descent.max(descent - metrics.radical_raise * (ascent + descent) + index.ascent);
            let intrinsic = IntrinsicWidths {
                min: result.intrinsic.min
                    + before
                    + index.intrinsic.min
                    + metrics.radical_kern[1].max(-index.intrinsic.min),
                max: result.intrinsic.max
                    + before
                    + index.intrinsic.max
                    + metrics.radical_kern[1].max(-index.intrinsic.max),
            };
            let mut with_index = MathBox {
                intrinsic,
                ..Default::default()
            };
            with_index.place(result, if style.rtl { 0.0 } else { base_offset }, 0.0);
            let index_x = if style.rtl {
                total_width - before - index.width
            } else {
                before
            };
            with_index.place(index, index_x, index_y);
            with_index.position(graphics, if style.rtl { 0.0 } else { base_offset }, 0.0);
            with_index.width = total_width;
            with_index.ascent = top;
            with_index.descent = bottom;
            with_index.ink_ascent = top;
            with_index.ink_descent = bottom;
            result = with_index;
        }
        Ok(result)
    }
    fn fraction(
        &mut self,
        element: Element<'d>,
        style: &Arc<Style>,
        depth: usize,
        layout: MathLayout,
    ) -> Result<MathBox, MathError<'d>> {
        let mut children = self.children(element, style, depth, layout)?;
        if children.len() != 2 {
            return self.row(element, style, depth, layout, RowKind::Fraction);
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
        let mut result = MathBox {
            intrinsic: IntrinsicWidths {
                min: numerator.intrinsic.min.max(denominator.intrinsic.min),
                max: numerator.intrinsic.max.max(denominator.intrinsic.max),
            },
            operator: numerator.operator.clone(),
            ..Default::default()
        };
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
        &mut self,
        element: Element<'d>,
        style: &Arc<Style>,
        depth: usize,
        layout: MathLayout,
    ) -> Result<MathBox, MathError<'d>> {
        let children = self.children(element, style, depth, layout)?;
        let paired = element.name() == "msubsup";
        if children.len() != if paired { 3 } else { 2 } {
            return self.row(element, style, depth, layout, RowKind::Scripted);
        }
        let (base, sub, sup) =
            script_children(children, element.name() != "msup", element.name() != "msub");
        self.script_boxes(base, sub, sup, style)
    }
    fn script_boxes(
        &self,
        base: MathBox,
        sub: Option<MathBox>,
        sup: Option<MathBox>,
        style: &Style,
    ) -> Result<MathBox, MathError<'d>> {
        let metrics = self.metrics(style)?;
        let [down, up] = Self::script_shifts(&base, sub.as_ref(), sup.as_ref(), style, &metrics);
        let x = base.width;
        let large = base
            .operator
            .as_ref()
            .is_some_and(|operator| operator.large);
        let italic = if large {
            0.0
        } else {
            base.italic.unwrap_or(0.0)
        };
        let sub_correction = if large {
            base.italic.unwrap_or(0.0)
        } else {
            0.0
        };
        let sub_x = x - sub_correction;
        let inline_size = |base: f32, sub: Option<f32>, sup: Option<f32>| {
            base.max(sub.map_or(0.0, |sub| base - sub_correction + sub))
                .max(sup.map_or(0.0, |sup| base + italic + sup))
                + metrics.after_script
        };
        let width = inline_size(
            base.width,
            sub.as_ref().map(|child| child.width),
            sup.as_ref().map(|child| child.width),
        );
        let intrinsic = IntrinsicWidths {
            min: inline_size(
                base.intrinsic.min,
                sub.as_ref().map(|child| child.intrinsic.min),
                sup.as_ref().map(|child| child.intrinsic.min),
            ),
            max: inline_size(
                base.intrinsic.max,
                sub.as_ref().map(|child| child.intrinsic.max),
                sup.as_ref().map(|child| child.intrinsic.max),
            ),
        };
        let physical = |x, size| if style.rtl { width - x - size } else { x };
        let mut result = MathBox {
            width,
            intrinsic,
            ascent: base.ascent,
            descent: base.descent,
            ink_ascent: base.ink_ascent,
            ink_descent: base.ink_descent,
            operator: base.operator.clone(),
            ..Default::default()
        };
        let base_x = physical(0.0, base.width);
        result.place(base, base_x, 0.0);
        if let Some(sub) = sub {
            let sub_x = physical(sub_x, sub.width);
            result.place(sub, sub_x, down);
        }
        if let Some(sup) = sup {
            let sup_x = physical(x + italic, sup.width);
            result.place(sup, sup_x, -up);
        }
        Ok(result)
    }
    fn script_shifts(
        base: &MathBox,
        sub: Option<&MathBox>,
        sup: Option<&MathBox>,
        style: &Style,
        metrics: &Metrics,
    ) -> [f32; 2] {
        let mut down = sub.map_or(0.0, |sub| {
            metrics
                .sub_shift
                .max(sub.ink_ascent - metrics.sub_top)
                .max(metrics.sub_drop + base.ink_descent)
        });
        let mut up = sup.map_or(0.0, |sup| {
            metrics.sup_shift[usize::from(style.math.shift)]
                .max(metrics.sup_bottom + sup.ink_descent)
                .max(base.ink_ascent - metrics.sup_drop)
        });
        if let (Some(sub), Some(sup)) = (sub, sup) {
            let gap = down - sub.ink_ascent + up - sup.ink_descent;
            if gap < metrics.script_gap {
                let adjustment = (metrics.sup_bottom_with_sub - (up - sup.ink_descent))
                    .max(0.0)
                    .min(metrics.script_gap - gap);
                up += adjustment;
                down += (metrics.script_gap - gap - adjustment).max(0.0);
            }
        }
        [down, up]
    }
    fn multiscripts(
        &mut self,
        element: Element<'d>,
        style: &Arc<Style>,
        depth: usize,
        layout: MathLayout,
    ) -> Result<MathBox, MathError<'d>> {
        let sources = self.in_flow_children(element, style)?;
        if sources
            .first()
            .is_none_or(|child| child.name() == "mprescripts")
        {
            return self.row(element, style, depth, layout, RowKind::Scripted);
        }
        let separators: Vec<_> = sources
            .iter()
            .enumerate()
            .filter(|(_, child)| child.name() == "mprescripts")
            .map(|(index, _)| index)
            .collect();
        if separators.len() > 1 {
            return self.row(element, style, depth, layout, RowKind::Scripted);
        }
        let post_count = separators.first().copied().unwrap_or(sources.len()) - 1;
        let pre_count = separators
            .first()
            .map_or(0, |index| sources.len() - index - 1);
        if post_count % 2 != 0 || pre_count % 2 != 0 {
            return self.row(element, style, depth, layout, RowKind::Scripted);
        }
        let mut children = self.children(element, style, depth, layout)?.into_iter();
        let base = children.next().expect("valid tensor base");
        let post = script_pairs(&mut children, post_count / 2);
        let marker = separators
            .first()
            .map(|_| children.next().expect("prescripts separator"));
        let pre = script_pairs(&mut children, pre_count / 2);
        let metrics = self.metrics(style)?;
        let [down, up] = post
            .iter()
            .chain(&pre)
            .fold([0.0f32, 0.0f32], |shifts, (sub, sup)| {
                let pair = Self::script_shifts(&base, Some(sub), Some(sup), style, &metrics);
                [shifts[0].max(pair[0]), shifts[1].max(pair[1])]
            });
        let large = base
            .operator
            .as_ref()
            .is_some_and(|operator| operator.large);
        let sub_italic = if large {
            base.italic.unwrap_or(0.0)
        } else {
            0.0
        };
        let sup_italic = if large {
            0.0
        } else {
            base.italic.unwrap_or(0.0)
        };
        let has_posts = !post.is_empty();
        let mut nodes = Vec::new();
        let mut offset = 0.0;
        let mut intrinsic_offset = IntrinsicWidths::default();
        let pre_start = 1 + post_count + usize::from(marker.is_some());
        for (index, (sub, sup)) in pre.into_iter().enumerate() {
            offset += metrics.after_script;
            intrinsic_offset.min += metrics.after_script + sub.intrinsic.min.max(sup.intrinsic.min);
            intrinsic_offset.max += metrics.after_script + sub.intrinsic.max.max(sup.intrinsic.max);
            let width = sub.width.max(sup.width);
            let sub_x = offset + width - sub.width;
            let sup_x = offset + width - sup.width;
            nodes.push((pre_start + 2 * index, sub, sub_x, down));
            nodes.push((pre_start + 2 * index + 1, sup, sup_x, -up));
            offset += width;
        }
        let operator = base.operator.clone();
        let (ascent, descent) = (base.ascent, base.descent);
        if let Some(marker) = marker {
            nodes.push((1 + post_count, marker, offset, 0.0));
        }
        let base_x = offset;
        offset += base.width;
        intrinsic_offset.min += base.intrinsic.min;
        intrinsic_offset.max += base.intrinsic.max;
        let mut intrinsic = intrinsic_offset;
        nodes.push((0, base, base_x, 0.0));
        for (index, (sub, sup)) in post.into_iter().enumerate() {
            intrinsic.min = intrinsic
                .min
                .max(intrinsic_offset.min - sub_italic + sub.intrinsic.min)
                .max(intrinsic_offset.min + sup_italic + sup.intrinsic.min);
            intrinsic.max = intrinsic
                .max
                .max(intrinsic_offset.max - sub_italic + sub.intrinsic.max)
                .max(intrinsic_offset.max + sup_italic + sup.intrinsic.max);
            intrinsic_offset.min += sub.intrinsic.min.max(sup.intrinsic.min) + metrics.after_script;
            intrinsic_offset.max += sub.intrinsic.max.max(sup.intrinsic.max) + metrics.after_script;
            let width = sub.width.max(sup.width);
            nodes.push((1 + 2 * index, sub, offset - sub_italic, down));
            nodes.push((2 + 2 * index, sup, offset + sup_italic, -up));
            offset += width + metrics.after_script;
        }
        let width = nodes
            .iter()
            .filter(|(index, _, _, _)| Some(index) != separators.first())
            .map(|(_, child, x, _)| x + child.width)
            .fold(0.0, f32::max)
            + if has_posts { metrics.after_script } else { 0.0 };
        if has_posts {
            intrinsic.min += metrics.after_script;
            intrinsic.max += metrics.after_script;
        }
        let mut result = MathBox {
            width,
            intrinsic,
            ascent,
            descent,
            operator,
            ..Default::default()
        };
        nodes.sort_by_key(|(index, _, _, _)| *index);
        for (_, child, x, y) in nodes {
            let x = if style.rtl {
                width - x - child.width
            } else {
                x
            };
            result.place(child, x, y);
        }
        result.width = width;
        result.ink_ascent = result.ascent;
        result.ink_descent = result.descent;
        Ok(result)
    }
    fn limits(
        &mut self,
        element: Element<'d>,
        style: &Arc<Style>,
        depth: usize,
        layout: MathLayout,
    ) -> Result<MathBox, MathError<'d>> {
        let mut children = self.children(element, style, depth, layout)?;
        let paired = element.name() == "munderover";
        if children.len() != if paired { 3 } else { 2 } {
            return self.row(element, style, depth, layout, RowKind::Scripted);
        }
        let movable = children[0]
            .operator
            .as_ref()
            .is_some_and(|operator| operator.movable);
        if style.math.compact && movable {
            let (base, under, over) = script_children(
                children,
                element.name() != "mover",
                element.name() != "munder",
            );
            return self.script_boxes(base, under, over, style);
        }
        let target = children
            .iter()
            .enumerate()
            .filter(|(index, child)| {
                (*index == 0 && layout.stretch.is_some())
                    || !child
                        .operator
                        .as_ref()
                        .is_some_and(|operator| operator.stretchy && operator.inline_axis)
            })
            .map(|(_, child)| child.width)
            .fold(0.0, f32::max);
        let sources = self.in_flow_children(element, style)?;
        let mut requests = Requests::default();
        for (index, child) in children.iter_mut().enumerate() {
            if index == 0 && layout.stretch.is_some() {
                continue;
            }
            if let Some(operator) = &child.operator
                && operator.stretchy
                && operator.inline_axis
                && let Some(resolved) = requests.collect(self.element(
                    sources[index],
                    style,
                    depth + 1,
                    false,
                    MathLayout {
                        form: Some(operator.form),
                        stretch: Some(Stretch::Inline(target)),
                    },
                ))?
            {
                *child = resolved;
            }
        }
        requests.finish()?;
        let (base, under, over) = script_children(
            children,
            element.name() != "mover",
            element.name() != "munder",
        );
        let large = base
            .operator
            .as_ref()
            .is_some_and(|operator| operator.large);
        let inline_stretchy = base
            .operator
            .as_ref()
            .is_some_and(|operator| operator.stretchy && operator.inline_axis);
        let italic = if large {
            base.italic.unwrap_or(0.0)
        } else {
            0.0
        };
        let inline_bounds = |base: f32, under: Option<f32>, over: Option<(f32, Option<f32>)>| {
            let mut minimum = -base / 2.0;
            let mut maximum = base / 2.0;
            if let Some(under) = under {
                minimum = minimum.min(-under / 2.0 - italic / 2.0);
                maximum = maximum.max(under / 2.0 - italic / 2.0);
            }
            if let Some((width, attachment)) = over {
                let attachment = attachment.unwrap_or(width / 2.0);
                minimum = minimum.min(-attachment + italic / 2.0);
                maximum = maximum.max(width - attachment + italic / 2.0);
            }
            [minimum, maximum]
        };
        let [minimum, maximum] = inline_bounds(
            base.width,
            under.as_ref().map(|child| child.width),
            over.as_ref().map(|child| (child.width, child.accent)),
        );
        let [min_start, min_end] = inline_bounds(
            base.intrinsic.min,
            under.as_ref().map(|child| child.intrinsic.min),
            over.as_ref()
                .map(|child| (child.intrinsic.min, child.accent)),
        );
        let [max_start, max_end] = inline_bounds(
            base.intrinsic.max,
            under.as_ref().map(|child| child.intrinsic.max),
            over.as_ref()
                .map(|child| (child.intrinsic.max, child.accent)),
        );
        let width = maximum - minimum;
        let intrinsic = IntrinsicWidths {
            min: min_end - min_start,
            max: max_end - max_start,
        };
        let metrics = self.metrics(style)?;
        let mut result = MathBox {
            width,
            intrinsic,
            ascent: base.ascent,
            descent: base.descent,
            operator: base.operator.clone(),
            ..Default::default()
        };
        let under_layout = under.map(|under| {
            let (shift, extra) = if large {
                (
                    metrics.limit_shift[1].max(metrics.limit_gap[1] + under.ascent),
                    0.0,
                )
            } else if inline_stretchy {
                (
                    metrics.stretch_shift[1].max(metrics.stretch_gap[1] + under.ascent),
                    0.0,
                )
            } else {
                (
                    if element
                        .attr("accentunder")
                        .is_some_and(|value| value.eq_ignore_ascii_case("true"))
                    {
                        0.0
                    } else {
                        metrics.bar_gap[1]
                    },
                    metrics.bar_extra[1],
                )
            };
            let x = -minimum - under.width / 2.0 - italic / 2.0;
            let y = base.ink_descent + shift;
            result.ascent = result.ascent.max(under.ascent - shift);
            result.descent = result.descent.max(under.descent + shift + extra);
            (under, x, y)
        });
        let over_layout = over.map(|over| {
            let (shift, extra) = if large {
                (
                    metrics.limit_shift[0].max(metrics.limit_gap[0] + over.descent),
                    0.0,
                )
            } else if inline_stretchy {
                (
                    metrics.stretch_shift[0].max(metrics.stretch_gap[0] + over.descent),
                    0.0,
                )
            } else {
                (
                    if element
                        .attr("accent")
                        .is_some_and(|value| value.eq_ignore_ascii_case("true"))
                    {
                        (metrics.accent_base - base.ascent).max(0.0)
                    } else {
                        metrics.bar_gap[0]
                    },
                    metrics.bar_extra[0],
                )
            };
            let x = -minimum - over.width / 2.0 + italic / 2.0;
            let y = -base.ink_ascent - shift;
            result.ascent = result.ascent.max(over.ascent + shift + extra);
            result.descent = result.descent.max(over.descent - shift);
            (over, x, y)
        });
        let base_x = -minimum - base.width / 2.0;
        let physical = |x, size| if style.rtl { width - x - size } else { x };
        let base_x = physical(base_x, base.width);
        result.position(base, base_x, 0.0);
        for (child, x, y) in under_layout.into_iter().chain(over_layout) {
            let x = physical(x, child.width);
            result.position(child, x, y);
        }
        result.ink_ascent = result.ascent;
        result.ink_descent = result.descent;
        Ok(result)
    }
    fn edges(content: MathBox, style: &Style) -> Result<MathBox, MathError<'d>> {
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
        let inline_edges = left + right + margins[3] + margins[1];
        let mut result = MathBox {
            intrinsic: IntrinsicWidths {
                min: content.intrinsic.min + inline_edges,
                max: content.intrinsic.max + inline_edges,
            },
            operator: content.operator.clone(),
            space_like: content.space_like,
            ink_ascent: if borders[0] > 0.0 {
                content.ascent + top
            } else {
                content.ink_ascent
            },
            ink_descent: if borders[2] > 0.0 {
                content.descent + bottom
            } else {
                content.ink_descent
            },
            italic: content.italic.map(|value| {
                value
                    + if style.rtl {
                        left + margins[3]
                    } else {
                        right + margins[1]
                    }
            }),
            accent: content.accent.map(|attachment| {
                attachment
                    + if style.rtl {
                        right + margins[1]
                    } else {
                        left + margins[3]
                    }
            }),
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
