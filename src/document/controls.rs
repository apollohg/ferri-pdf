//! Form controls (HTML §15.5): the internal boxes each widget renders its initial state with.
//! Widgets with a native appearance that shows no text are laid out as replaced elements
//! (CSS UI 4 §7.2 `auto`) whose drawing is made at their used size; text-bearing widgets are
//! inline-blocks holding anonymous boxes in the element's own font, so author CSS restyles
//! them like any other box.
use crate::document::formatting::{BlockBox, ImageResources};
use crate::document::{BlockContent, DocumentError, Paragraph};
use crate::dom::{Element, GaugeRegion, HTML_NAMESPACE, InputType, TreeRelation};
use crate::images::{
    Image,
    widget::{GaugeFill, Widget, WidgetDrawing},
};
use crate::style::{
    Appearance, ContainIntrinsicSize, Display, FieldSizing, IntrinsicSizing, LengthPercentage,
    LineHeight, Overflow, POINTS_PER_PIXEL, PseudoElement, Size, Style, StyleError, Stylesheet,
    Visibility, WhiteSpace,
};
use std::sync::Arc;

/// HTML §15.5.6: text fields without a valid `size` are 20 characters wide.
const DEFAULT_TEXT_SIZE: u32 = 20;
/// HTML §4.10.11: a textarea's default character width and height.
const DEFAULT_COLS: u32 = 20;
const DEFAULT_ROWS: u32 = 2;
/// Password fields obscure each character of the value with this mask.
const PASSWORD_MASK: char = '\u{2022}';
/// Keeps a line box in a control whose label or value is empty.
const EMPTY_LINE: &str = "\u{200b}";
/// Implementation-defined button labels (HTML §15.5.11, §15.5.12).
const SUBMIT_LABEL: &str = "Submit";
const RESET_LABEL: &str = "Reset";
const FILE_BUTTON_LABEL: &str = "Choose File";
/// Natural sizes in em of the controls whose native appearance is a drawing: 13px
/// checkboxes and radio buttons, a slider and the gauges of HTML §15.5.14–§15.5.15.
const TOGGLE_SIZE_EM: [f32; 2] = [0.8125, 0.8125];
const RANGE_SIZE_EM: [f32; 2] = [8.0, 1.0];
const METER_SIZE_EM: [f32; 2] = [5.0, 1.0];
const PROGRESS_SIZE_EM: [f32; 2] = [10.0, 1.0];
const ARROW_SIZE_EM: [f32; 2] = [0.625, 0.625];
/// Space between a drop-down box's label and its button, in em.
const ARROW_GAP_EM: f32 = 0.25;
/// A colour well's swatch, in em.
const SWATCH_SIZE_EM: [f32; 2] = [2.75, 1.0];
/// The widest strings each date and time control shows, in tabular digits.
const DATE_TEMPLATE: &str = "0000-00-00";
const MONTH_TEMPLATE: &str = "0000-00";
const WEEK_TEMPLATE: &str = "0000-W00";
const TIME_TEMPLATE: &str = "00:00";
const LOCAL_DATE_TIME_TEMPLATE: &str = "0000-00-00T00:00";
/// HTML §15.5.13: a vertical marquee's natural height.
const VERTICAL_MARQUEE_HEIGHT_PX: f32 = 200.0;

/// What a form control renders in place of its children.
pub(super) struct ControlContent {
    pub image: Option<Arc<Image>>,
    pub children: Vec<BlockContent>,
}

/// Whether a widget renders natively or in its primitive appearance (CSS UI 4 §7.2).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Look {
    Native,
    Primitive,
}

fn html(element: Element<'_>, name: &str) -> bool {
    element.namespace() == HTML_NAMESPACE && element.name() == name
}

/// HTML §15.5.1 and CSS UI 4 §7.2.1: checkboxes, radio buttons and sliders never devolve.
fn look(element: Element<'_>, style: &Style) -> Look {
    let devolvable = !matches!(
        element.input_type(),
        Some(InputType::Checkbox | InputType::Radio | InputType::Range)
    );
    match style.widget.appearance {
        Appearance::None => Look::Primitive,
        // Widgets without a base appearance render as for `auto`; a `select` with base
        // appearance renders its internal shadow tree instead.
        Appearance::Auto
        | Appearance::Base
        | Appearance::BaseSelect
        | Appearance::Textfield
        | Appearance::MenulistButton => {
            if devolvable && style.widget.author_decorated {
                Look::Primitive
            } else {
                Look::Native
            }
        }
    }
}

/// HTML §15.3.10, §15.5.3, §15.3.12: how a control's computed `display` and inline size act.
/// Inputs and other widgets always have a flow-root inner display; buttons and fieldsets
/// keep flex and grid layouts; inline ones behave as inline-blocks. Buttons and rendered
/// legends with an `auto` inline size use the fit-content size, which also keeps an
/// absolutely positioned button as wide as a replaced element would be.
pub(super) fn used_style(element: Element<'_>, style: &mut Style) {
    if element.namespace() != HTML_NAMESPACE {
        return;
    }
    let display = &mut style.display;
    match element.name() {
        // HTML §15.5.16: base appearance takes the display of its user-agent styles.
        "select" if element.is_base_select() => {}
        "input" | "select" | "textarea" | "meter" | "progress" => {
            *display = match display {
                Display::None | Display::Contents => return,
                _ if display.is_inline_level() => Display::InlineBlock,
                _ => Display::FlowRoot,
            };
        }
        "button" | "fieldset" => button_display(display),
        "marquee"
            if element.attr("direction").is_some_and(|direction| {
                direction.eq_ignore_ascii_case("up") || direction.eq_ignore_ascii_case("down")
            }) =>
        {
            style.height = style
                .height
                .with_automatic(Size::Value(LengthPercentage::Length(
                    VERTICAL_MARQUEE_HEIGHT_PX * POINTS_PER_PIXEL,
                )));
        }
        _ => {}
    }
    if is_rendered_legend(element, style) {
        style.display = style.display.blockified();
    }
    let button_layout = html(element, "button")
        || matches!(
            element.input_type(),
            Some(InputType::Submit | InputType::Reset | InputType::Button | InputType::Color)
        )
        || is_rendered_legend(element, style);
    if button_layout {
        style.width = style
            .width
            .with_automatic(Size::Intrinsic(IntrinsicSizing::FitContent));
    }
}

/// HTML §15.5.3: a button keeps flex and grid layouts, and an inline one behaves as an
/// inline-block.
fn button_display(display: &mut Display) {
    if display.is_inline_level() && !matches!(display, Display::InlineFlex | Display::InlineGrid) {
        *display = Display::InlineBlock;
    }
}

/// HTML §15.3.12: a fieldset's first legend child that neither floats nor is absolutely
/// positioned.
pub(super) fn is_rendered_legend(element: Element<'_>, style: &Style) -> bool {
    html(element, "legend")
        && style.floating().is_none()
        && !style.position.is_out_of_flow()
        && element
            .parent(TreeRelation::Formatting)
            .is_some_and(|parent| {
                html(parent, "fieldset")
                    && parent
                        .children(TreeRelation::Formatting)
                        .find(|child| html(*child, "legend"))
                        == Some(element)
            })
}

/// HTML §15.3.12: a fieldset holds its rendered legend and an anonymous fieldset content box
/// with everything else. The fieldset is a block formatting context whose flex, grid,
/// multicolumn and overflow behaviour, block padding and `unicode-bidi` pass to the content
/// box; its inline padding stays, which positions the legend as if the fieldset had it and
/// gives the content box the same inline extent.
pub(super) fn fieldset(
    element: Element<'_>,
    mut style: Style,
    children: Vec<BlockContent>,
    page_scope: Option<&Arc<str>>,
) -> (Style, Vec<BlockContent>) {
    if !html(element, "fieldset") || style.display == Display::Contents {
        return (style, children);
    }
    let mut content = style.anonymous(match style.display {
        Display::Grid | Display::InlineGrid => Display::Grid,
        Display::Flex | Display::InlineFlex => Display::Flex,
        _ => Display::FlowRoot,
    });
    content.flex = style.flex.clone();
    content.grid = style.grid.clone();
    content.multicol = style.multicol.clone();
    content.overflow = style.overflow;
    content.text_overflow = style.text_overflow;
    content.unicode_bidi = style.unicode_bidi;
    content.decoration.radii = style.decoration.radii.clone();
    content.decoration.corner_shapes = style.decoration.corner_shapes.clone();
    for side in [0, 2] {
        content.padding[side] = std::mem::replace(&mut style.padding[side], LengthPercentage::ZERO);
    }
    style.display = if style.display.is_inline_level() {
        Display::InlineBlock
    } else if style.display.is_list_item() {
        Display::FlowRootListItem
    } else {
        Display::FlowRoot
    };
    style.flex = Default::default();
    style.grid = Default::default();
    style.multicol = Default::default();
    style.overflow = Default::default();
    let mut legend = None;
    let mut rest = Vec::new();
    for child in children {
        match child {
            BlockContent::Block(block) if legend.is_none() && block.is_rendered_legend() => {
                legend = Some(BlockContent::Block(block));
            }
            child => rest.push(child),
        }
    }
    let content = BlockContent::Block(BlockBox::anonymous(content, rest, page_scope.cloned()));
    (style, legend.into_iter().chain([content]).collect())
}

struct Builder<'b, 'a> {
    element: Element<'b>,
    style: &'b Style,
    sheet: &'a Stylesheet,
    resources: &'b mut ImageResources<'a>,
    page_scope: Option<&'b Arc<str>>,
}

/// The content of a form control that renders its state rather than its children, or `None`
/// for other elements.
pub(super) fn content<'a>(
    element: Element<'_>,
    style: &Style,
    sheet: &'a Stylesheet,
    resources: &mut ImageResources<'a>,
    page_scope: Option<&Arc<str>>,
) -> Result<Option<ControlContent>, DocumentError> {
    if element.namespace() != HTML_NAMESPACE {
        return Ok(None);
    }
    let mut builder = Builder {
        element,
        style,
        sheet,
        resources,
        page_scope,
    };
    let content = match element.name() {
        "input" => builder.input()?,
        "textarea" => builder.textarea()?,
        "select" if element.is_base_select() => return Ok(None),
        "select" => builder.select()?,
        "meter" => {
            let gauge = element.meter_gauge();
            let span = gauge.maximum - gauge.minimum;
            let position = if span > 0.0 {
                (gauge.value - gauge.minimum) / span
            } else {
                0.0
            };
            let fill = match gauge.region() {
                GaugeRegion::Optimum => GaugeFill::Optimum,
                GaugeRegion::Suboptimal => GaugeFill::Suboptimal,
                GaugeRegion::EvenLessGood => GaugeFill::EvenLessGood,
            };
            let track = look(element, style) == Look::Native;
            builder.drawn(
                METER_SIZE_EM,
                WidgetDrawing::Meter {
                    position: position as f32,
                    fill,
                    track,
                },
            )
        }
        "progress" => {
            let track = look(element, style) == Look::Native;
            builder.drawn(
                PROGRESS_SIZE_EM,
                WidgetDrawing::Progress {
                    position: element.progress_position().map(|position| position as f32),
                    track,
                },
            )
        }
        _ => return Ok(None),
    };
    Ok(Some(content))
}

impl<'b> Builder<'b, '_> {
    fn widget(&self, size_em: [f32; 2], drawing: WidgetDrawing) -> Arc<Image> {
        let scheme = self.sheet.used_color_scheme(&self.style.color_scheme);
        let vertical = self.style.writing_mode.vertical();
        let mut natural = size_em.map(|em| em * self.style.size / POINTS_PER_PIXEL);
        if vertical && drawing.runs_inline() {
            natural.reverse();
        }
        Arc::new(Image::Widget(Arc::new(Widget {
            natural,
            drawing,
            palette: crate::style::widget_palette(self.style.accent_color, scheme),
            rtl: self.style.rtl,
            vertical,
            disabled: self.element.initial_state().disabled == Some(true),
        })))
    }

    fn drawn(&self, size_em: [f32; 2], drawing: WidgetDrawing) -> ControlContent {
        ControlContent {
            image: Some(self.widget(size_em, drawing)),
            children: Vec::new(),
        }
    }

    fn empty() -> ControlContent {
        ControlContent {
            image: None,
            children: Vec::new(),
        }
    }

    fn input(&mut self) -> Result<ControlContent, DocumentError> {
        let element = self.element;
        let kind = element
            .input_type()
            .expect("HTML input has an initial type");
        let look = look(element, self.style);
        Ok(match kind {
            InputType::Hidden => Self::empty(),
            InputType::Checkbox | InputType::Radio => {
                if look == Look::Primitive {
                    return Ok(Self::empty());
                }
                let checked = element.initially_checked();
                self.drawn(
                    TOGGLE_SIZE_EM,
                    if kind == InputType::Checkbox {
                        WidgetDrawing::Checkbox { checked }
                    } else {
                        WidgetDrawing::Radio { checked }
                    },
                )
            }
            InputType::Range => {
                let range = element
                    .numeric_range()
                    .expect("range controls have a numeric range");
                let minimum = range.minimum.expect("range controls have a minimum");
                let maximum = range.maximum.expect("range controls have a maximum");
                let span = maximum - minimum;
                let at = |value: f64| {
                    if span > 0.0 {
                        ((value - minimum) / span) as f32
                    } else {
                        0.0
                    }
                };
                let ticks = self
                    .suggestions()
                    .into_iter()
                    .filter(|value| *value >= minimum && *value <= maximum)
                    .map(at)
                    .collect();
                self.drawn(
                    RANGE_SIZE_EM,
                    WidgetDrawing::Range {
                        position: at(element.range_number()),
                        ticks,
                        track: look == Look::Native,
                    },
                )
            }
            InputType::Color => self.color_well(),
            InputType::Submit | InputType::Reset | InputType::Button => {
                let label = element.attr("value").unwrap_or(match kind {
                    InputType::Submit => SUBMIT_LABEL,
                    InputType::Reset => RESET_LABEL,
                    _ => "",
                });
                ControlContent {
                    image: None,
                    children: Self::label(label, self.style)?,
                }
            }
            InputType::Image => self.image_button()?,
            InputType::File => self.file_button()?,
            _ => self.text_field(kind)?,
        })
    }

    /// HTML §4.10.5.3.9: the valid floating-point values of the options of the input's
    /// suggestions source element.
    fn suggestions(&self) -> Vec<f64> {
        let Some(id) = self.element.attr("list").filter(|id| !id.is_empty()) else {
            return Vec::new();
        };
        let Some(list) = self
            .element
            .ancestors(TreeRelation::Selector)
            .last()
            .into_iter()
            .flat_map(|root| root.as_node().inclusive_descendants(TreeRelation::Selector))
            .filter_map(crate::dom::Node::element)
            .find(|candidate| candidate.attr("id") == Some(id))
            .filter(|candidate| html(*candidate, "datalist"))
        else {
            return Vec::new();
        };
        list.as_node()
            .inclusive_descendants(TreeRelation::Selector)
            .filter_map(crate::dom::Node::element)
            .filter(|option| {
                html(*option, "option") && option.initial_state().disabled != Some(true)
            })
            .filter_map(Element::option_value)
            .filter(|value| crate::dom::valid_float(value))
            .filter_map(|value| crate::dom::parse_float(&value))
            .collect()
    }

    /// A line of text in `style`, which keeps a line box when it is empty.
    fn label(text: &str, style: &Style) -> Result<Vec<BlockContent>, DocumentError> {
        let text = if text.is_empty() { EMPTY_LINE } else { text };
        Ok(Paragraph::generated(text, style)?
            .map(|mut paragraph| {
                paragraph.set_formatted_lines(true, false);
                BlockContent::Inline(paragraph)
            })
            .into_iter()
            .collect())
    }

    /// A box with its own decoration, such as an option's.
    fn styled_block(&mut self, style: Style, children: Vec<BlockContent>) -> BlockContent {
        let decoration = self.resources.backgrounds(&style);
        BlockContent::Block(BlockBox::with_decoration(
            style,
            children,
            decoration,
            self.page_scope.cloned(),
        ))
    }

    fn block(&self, style: Style, children: Vec<BlockContent>) -> BlockContent {
        BlockContent::Block(BlockBox::anonymous(
            style,
            children,
            self.page_scope.cloned(),
        ))
    }

    /// The first available font's average and maximum character widths, and its normal
    /// line height, in points.
    fn character_metrics(&self) -> Result<[f32; 3], DocumentError> {
        let style = self.style;
        let (metrics, scale) = self
            .sheet
            .fonts
            .first_available_metrics(
                style.font_candidates(),
                style.font_selection(),
                &style.font_adjustments,
                style.size,
                self.resources.resolver,
            )
            .map_err(StyleError::from)?;
        let em = style.size * scale;
        Ok([
            metrics.average_advance * em,
            metrics.maximum_advance * em,
            metrics.normal_line_height() * em,
        ])
    }

    /// A line height no smaller than `line-height: normal` (HTML §15.5.6).
    fn at_least_normal(style: &mut Style, normal: f32) {
        if style
            .line_height
            .resolve(style.size)
            .is_some_and(|height| height < normal)
        {
            style.line_height = LineHeight::Normal;
        }
    }

    /// HTML §15.5.6–§15.5.7: one-line text, number, date and time fields. The value shows,
    /// or the placeholder while it is empty, in a line that never wraps.
    fn text_field(&mut self, kind: InputType) -> Result<ControlContent, DocumentError> {
        let element = self.element;
        let [average, maximum, normal] = self.character_metrics()?;
        let mut editor = self.style.anonymous(Display::Block);
        editor.white_space = WhiteSpace::PRE;
        // CSS UI 4 §7.3.1: `text-overflow` applies to the line regardless of `overflow`, as
        // the control clips its content.
        editor.text_overflow = self.style.text_overflow;
        editor.overflow[0] = Overflow::Clip;
        Self::at_least_normal(&mut editor, normal);
        let value = element.input_value();
        let mut lines = Vec::new();
        if let Some(placeholder) = element.shown_placeholder() {
            let mut style = self
                .sheet
                .element_backed_with_resolver(
                    element,
                    self.style,
                    PseudoElement::Placeholder,
                    self.resources.resolver,
                )?
                .used();
            style.white_space = WhiteSpace::PRE;
            let placeholder = crate::dom::stripped_newlines(placeholder);
            lines.push(self.styled_line(&placeholder, &editor, &style)?);
        } else if kind == InputType::Password {
            let masked: String =
                std::iter::repeat_n(PASSWORD_MASK, value.chars().count()).collect();
            lines.push(self.block(editor.clone(), Self::label(&masked, &editor)?));
        } else {
            lines.push(self.block(editor.clone(), Self::label(&value, &editor)?));
        }
        let template = match kind {
            InputType::Date => Some(DATE_TEMPLATE),
            InputType::Month => Some(MONTH_TEMPLATE),
            InputType::Week => Some(WEEK_TEMPLATE),
            InputType::Time => Some(TIME_TEMPLATE),
            InputType::DateTimeLocal => Some(LOCAL_DATE_TIME_TEMPLATE),
            _ => None,
        };
        let bounds: Vec<String> = if kind == InputType::Number {
            ["min", "max"]
                .iter()
                .filter_map(|name| element.attr(name))
                .filter(|value| crate::dom::valid_float(value))
                .map(str::to_owned)
                .collect()
        } else {
            Vec::new()
        };
        let mut container = self.style.anonymous(Display::Block);
        if self.style.widget.field_sizing == FieldSizing::Fixed {
            // HTML §15.5.7: date, time and bounded number fields are as wide as their widest
            // value; other fields measure `size` characters (HTML §15.5.6).
            let widest: Vec<&str> = match template {
                Some(template) => vec![template],
                None if bounds.len() == 2 => bounds.iter().map(String::as_str).collect(),
                None => Vec::new(),
            };
            if widest.is_empty() {
                let size = element
                    .attr("size")
                    .filter(|_| kind.applies(crate::dom::InputAttribute::Size))
                    .and_then(crate::dom::non_negative_integer)
                    .filter(|size| *size > 0)
                    .unwrap_or(DEFAULT_TEXT_SIZE);
                let width = (size - 1) as f32 * average + maximum;
                container.containment.contain.inline_size = true;
                container.containment.intrinsic[0] = ContainIntrinsicSize {
                    auto: false,
                    length: Some(width),
                };
            }
            // The widening lines precede the shown one, whose line gives the baseline.
            let hidden = Self::collapsed(&editor);
            for text in widest.into_iter().rev() {
                lines.insert(0, self.block(hidden.clone(), Self::label(text, &hidden)?));
            }
        }
        container.overflow = [Overflow::Clip; 2];
        Ok(ControlContent {
            image: None,
            children: vec![self.block(container, lines)],
        })
    }

    /// A line whose text is styled by a pseudo-element of the control.
    fn styled_line(
        &mut self,
        text: &str,
        line: &Style,
        text_style: &Style,
    ) -> Result<BlockContent, DocumentError> {
        let mut inline = crate::document::inline::InlineCollector::default();
        let decoration = self.resources.backgrounds(text_style);
        inline.open_box(text_style, decoration, Default::default(), false);
        inline.append(if text.is_empty() { EMPTY_LINE } else { text }, text_style)?;
        inline.close_box();
        let children = match inline.finish(line)? {
            Ok(mut paragraph) => {
                paragraph.set_formatted_lines(true, false);
                vec![BlockContent::Inline(paragraph)]
            }
            Err(stray) => stray,
        };
        Ok(self.block(line.clone(), children))
    }

    /// A line that only widens its container: no height and invisible.
    fn collapsed(style: &Style) -> Style {
        let mut style = style.clone();
        style.height = Size::Value(LengthPercentage::Length(0.0));
        style.visibility = Visibility::Hidden;
        style
    }

    /// HTML §15.5.17: a multiline text control `cols` characters wide and `rows` lines high,
    /// showing its raw value or placeholder with white space preserved.
    fn textarea(&mut self) -> Result<ControlContent, DocumentError> {
        let element = self.element;
        let [average, _, normal] = self.character_metrics()?;
        let mut editor = self.style.anonymous(Display::Block);
        editor.overflow = [Overflow::Clip; 2];
        editor.text_overflow = self.style.text_overflow;
        let dimension = |name: &str, default: u32| {
            element
                .attr(name)
                .and_then(crate::dom::non_negative_integer)
                .filter(|value| *value > 0)
                .unwrap_or(default)
        };
        if self.style.widget.field_sizing == FieldSizing::Fixed {
            let line = self
                .style
                .line_height
                .resolve(self.style.size)
                .unwrap_or(normal);
            editor.containment.contain.size = true;
            editor.containment.intrinsic = [
                ContainIntrinsicSize {
                    auto: false,
                    length: Some(dimension("cols", DEFAULT_COLS) as f32 * average),
                },
                ContainIntrinsicSize {
                    auto: false,
                    length: Some(dimension("rows", DEFAULT_ROWS) as f32 * line),
                },
            ];
        }
        let line = if let Some(placeholder) = element.shown_placeholder() {
            let style = self
                .sheet
                .element_backed_with_resolver(
                    element,
                    self.style,
                    PseudoElement::Placeholder,
                    self.resources.resolver,
                )?
                .used();
            self.styled_line(placeholder, &editor, &style)?
        } else {
            self.block(
                editor.clone(),
                Self::label(&element.textarea_value(), &editor)?,
            )
        };
        Ok(ControlContent {
            image: None,
            children: vec![line],
        })
    }

    /// HTML §15.5.16: a drop-down box shows its selected option's label beside a drop-down
    /// button and is as wide as its widest label; a list box shows its display size of rows.
    fn select(&mut self) -> Result<ControlContent, DocumentError> {
        let element = self.element;
        let selected = element.initial_selected_options();
        let list_box = element.is_list_box();
        // A drop-down box's other labels only widen it, ahead of the shown label so that its
        // line is the box's last and gives its baseline.
        let mut rows = Vec::new();
        let mut shown_row = None;
        for (item, mut style) in self.select_items()? {
            style.display = Display::Block;
            style.white_space = WhiteSpace::PRE;
            let shown = list_box || (shown_row.is_none() && selected.first() == Some(&item));
            // HTML §15.5.16: with `field-sizing: content` only the shown label sizes the box.
            if !shown && self.style.widget.field_sizing == FieldSizing::Content {
                continue;
            }
            let style = if shown {
                style
            } else {
                Self::collapsed(&style)
            };
            let label = Self::label(&item.option_label(), &style)?;
            let row = self.styled_block(style, label);
            if shown && !list_box {
                shown_row = Some(row);
            } else {
                rows.push(row);
            }
        }
        if list_box {
            let [_, _, normal] = self.character_metrics()?;
            let line = self
                .style
                .line_height
                .resolve(self.style.size)
                .unwrap_or(normal);
            let mut list = self.style.anonymous(Display::Block);
            list.overflow = [Overflow::Clip; 2];
            list.height = Size::Value(LengthPercentage::Length(
                if self.style.widget.field_sizing == FieldSizing::Content {
                    rows.len() as f32
                } else {
                    element.display_size() as f32
                } * line,
            ));
            return Ok(ControlContent {
                image: None,
                children: vec![self.block(list, rows)],
            });
        }
        let shown_row = if let Some(row) = shown_row {
            row
        } else {
            let line = self.style.anonymous(Display::Block);
            self.block(line.clone(), Self::label("", &line)?)
        };
        rows.push(shown_row);
        let mut labels = self.style.anonymous(Display::Block);
        labels.flex.grow = 1.0;
        labels.overflow = [Overflow::Clip; 2];
        let mut children = vec![self.block(labels, rows)];
        // A devolved drop-down box keeps its button; only primitive appearance drops it.
        if self.style.widget.appearance != Appearance::None {
            let mut arrow = self.style.anonymous(Display::Block);
            arrow.margin[if self.style.rtl { 3 } else { 1 }] =
                Size::Value(LengthPercentage::Length(0.0));
            arrow.margin[if self.style.rtl { 1 } else { 3 }] =
                Size::Value(LengthPercentage::Length(ARROW_GAP_EM * self.style.size));
            children.push(BlockContent::Block(BlockBox::anonymous_replaced(
                arrow,
                self.widget(ARROW_SIZE_EM, WidgetDrawing::DropDownArrow),
                self.page_scope.cloned(),
            )));
        }
        let mut row = self.style.anonymous(Display::Flex);
        row.flex.align_items = crate::style::Alignment::Center;
        Ok(ControlContent {
            image: None,
            children: vec![self.block(row, children)],
        })
    }

    /// A select's items in tree order, option group headers and their options, each in its
    /// element's computed style.
    fn select_items(&mut self) -> Result<Vec<(Element<'b>, Style)>, DocumentError> {
        let mut items = Vec::new();
        let mut pending: Vec<_> = self
            .element
            .children(TreeRelation::Selector)
            .rev()
            .map(|child| (child, self.style.clone(), false))
            .collect();
        while let Some((child, parent, grouped)) = pending.pop() {
            if child.namespace() != HTML_NAMESPACE || !matches!(child.name(), "option" | "optgroup")
            {
                continue;
            }
            let style = self
                .sheet
                .computed_with_resolver(child, &parent, self.resources.resolver)?
                .used();
            if style.display == Display::None {
                continue;
            }
            if child.name() == "optgroup" && !grouped {
                pending.extend(
                    child
                        .children(TreeRelation::Selector)
                        .rev()
                        .map(|option| (option, style.clone(), true)),
                );
            }
            items.push((child, style));
        }
        Ok(items)
    }

    /// HTML §15.5.9: a colour well uses button layout around a block filled with its colour.
    fn color_well(&self) -> ControlContent {
        let color = crate::style::parse_color_value(&self.element.input_value())
            .expect("a colour well's value parses");
        let mut swatch = self.style.anonymous(Display::Block);
        swatch.decoration.background = color;
        swatch.containment.contain.size = true;
        swatch.containment.intrinsic = SWATCH_SIZE_EM.map(|em| ContainIntrinsicSize {
            auto: false,
            length: Some(em * self.style.size),
        });
        ControlContent {
            image: None,
            children: vec![self.block(swatch, Vec::new())],
        }
    }

    /// HTML §4.10.5.1.20: an image button shows its image, or its alternative text as a
    /// button label while the image is unavailable.
    fn image_button(&mut self) -> Result<ControlContent, DocumentError> {
        let image = self
            .element
            .attr("src")
            .filter(|source| !source.is_empty())
            .and_then(|source| {
                self.resources.cache.load_optional(
                    source,
                    self.resources.base,
                    self.resources.resolver,
                    self.style.element_font_namespace(),
                )
            })
            .map(|image| crate::images::content_image(image, self.style, None));
        Ok(match image {
            Some(image) => ControlContent {
                image: Some(image),
                children: Vec::new(),
            },
            None => ControlContent {
                image: None,
                children: Self::label(
                    self.element.attr("alt").unwrap_or(SUBMIT_LABEL),
                    self.style,
                )?,
            },
        })
    }

    /// HTML §15.5.11: no file is selected, so the control holds only its
    /// `::file-selector-button`.
    fn file_button(&mut self) -> Result<ControlContent, DocumentError> {
        let mut style = self
            .sheet
            .element_backed_with_resolver(
                self.element,
                self.style,
                PseudoElement::FileSelectorButton,
                self.resources.resolver,
            )?
            .used();
        match style.display {
            Display::None => return Ok(Self::empty()),
            Display::Contents => {
                return Ok(ControlContent {
                    image: None,
                    children: Self::label(FILE_BUTTON_LABEL, &style)?,
                });
            }
            _ => button_display(&mut style.display),
        }
        style.width = style
            .width
            .with_automatic(Size::Intrinsic(IntrinsicSizing::FitContent));
        let decoration = self.resources.backgrounds(&style);
        let button = BlockBox::with_decoration(
            style.clone(),
            Self::label(FILE_BUTTON_LABEL, &style)?,
            decoration,
            self.page_scope.cloned(),
        );
        let mut inline = crate::document::inline::InlineCollector::default();
        if style.position.is_out_of_flow() || style.floating().is_some() {
            inline.anchor(button);
        } else if style.display.is_inline_level() {
            inline.atomic(button, self.style)?;
        } else {
            return Ok(ControlContent {
                image: None,
                children: vec![BlockContent::Block(button)],
            });
        }
        Ok(ControlContent {
            image: None,
            children: match inline.finish(self.style)? {
                Ok(mut paragraph) => {
                    paragraph.set_formatted_lines(true, false);
                    vec![BlockContent::Inline(paragraph)]
                }
                Err(stray) => stray,
            },
        })
    }
}
