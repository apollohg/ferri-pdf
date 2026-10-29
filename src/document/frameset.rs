//! Frames and framesets (HTML §15.6): a frameset fills the viewport with a grid of
//! rectangles, each showing a frame's content navigable or a nested frameset's grid.
use crate::document::BlockContent;
use crate::document::controls::ControlContent;
use crate::document::embedded::{Embedded, EmbeddedContent};
use crate::document::formatting::BlockBox;
use crate::dom::{Element, HTML_NAMESPACE, TreeRelation};
use crate::graphics::{Border, BorderStyle, Color};
use crate::images::Image;
use crate::style::{
    BoxSizing, Display, LengthPercentage, POINTS_PER_PIXEL, Position, Size, Style, Stylesheet,
};
use std::sync::Arc;

/// The implementation-defined width of a frame border, in CSS pixels.
const FRAME_BORDER_PX: f32 = 2.0;
/// The implementation-defined frame border colour.
const FRAME_BORDER_COLOR: Color = Color {
    rgb: [0x76, 0x76, 0x76],
    alpha: 1.0,
};
const PERCENT: f32 = 100.0;
const DECIMAL_RADIX: u32 = 10;

fn html(element: Element<'_>, name: &str) -> bool {
    element.namespace() == HTML_NAMESPACE && element.name() == name
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Unit {
    Percentage,
    Relative,
    Absolute,
}

/// HTML §2.3.4.6 rules for parsing a list of dimensions.
fn dimensions(raw: &str) -> Vec<(f32, Unit)> {
    let raw = raw.strip_suffix(',').unwrap_or(raw);
    raw.split(',')
        .map(|token| {
            let mut rest = token;
            if rest.is_empty() {
                return (0.0, Unit::Relative);
            }
            let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
            let mut value = rest[..digits].bytes().fold(0.0f32, |value, digit| {
                value * DECIMAL_RADIX as f32 + f32::from(digit - b'0')
            });
            rest = &rest[digits..];
            if let Some(after) = rest.strip_prefix('.') {
                let length = after
                    .bytes()
                    .take_while(|ch| ch.is_ascii_digit() || ch.is_ascii_whitespace())
                    .count();
                let fraction: String = after[..length]
                    .chars()
                    .filter(char::is_ascii_digit)
                    .collect();
                if !fraction.is_empty() {
                    value += format!("0.{fraction}").parse::<f32>().unwrap_or(0.0);
                }
                rest = &after[length..];
            }
            rest = rest.trim_start_matches(|ch: char| ch.is_ascii_whitespace());
            let unit = match rest.chars().next() {
                Some('%') => Unit::Percentage,
                Some('*') => Unit::Relative,
                _ => Unit::Absolute,
            };
            (value, unit)
        })
        .collect()
}

/// HTML §15.6 "convert a list of dimensions to a list of pixel values", in points.
fn sizes(list: &[(f32, Unit)], input: f32) -> Vec<f32> {
    let total = |unit| -> f32 {
        list.iter()
            .filter(|(_, candidate)| *candidate == unit)
            .map(|(value, _)| value)
            .sum()
    };
    let (percentage, relative, absolute) = (
        total(Unit::Percentage),
        total(Unit::Relative),
        total(Unit::Absolute) * POINTS_PER_PIXEL,
    );
    let mut output = vec![0.0; list.len()];
    let mut remaining = input;
    let each = |output: &mut Vec<f32>, unit, size: &dyn Fn(f32) -> f32| {
        for (slot, (value, candidate)) in output.iter_mut().zip(list) {
            if *candidate == unit {
                *slot = size(*value);
            }
        }
    };
    if absolute > remaining {
        each(&mut output, Unit::Absolute, &|value| {
            value * POINTS_PER_PIXEL * remaining / absolute
        });
        remaining = 0.0;
    } else {
        each(&mut output, Unit::Absolute, &|value| {
            value * POINTS_PER_PIXEL
        });
        remaining -= absolute;
    }
    if percentage * input / PERCENT > remaining {
        each(&mut output, Unit::Percentage, &|value| {
            value * remaining / percentage
        });
        remaining = 0.0;
    } else {
        each(&mut output, Unit::Percentage, &|value| {
            value * input / PERCENT
        });
        remaining -= percentage * input / PERCENT;
    }
    if relative > 0.0 {
        each(&mut output, Unit::Relative, &|value| {
            value * remaining / relative
        });
    }
    output
}

/// A frameset's tracks along one axis; relative entries of zero count as one, and an empty
/// list is one relative track (HTML §15.6).
fn tracks(frameset: Element<'_>, attribute: &str, input: f32) -> Vec<f32> {
    let mut list = frameset.attr(attribute).map(dimensions).unwrap_or_default();
    for (value, unit) in &mut list {
        if *unit == Unit::Relative && *value == 0.0 {
            *value = 1.0;
        }
    }
    if list.is_empty() {
        list.push((1.0, Unit::Relative));
    }
    sizes(&list, input)
}

/// HTML §15.6: whether a frame or frameset has a border.
fn has_border(element: Element<'_>) -> bool {
    match element.attr("frameborder") {
        Some(value) => value.starts_with(['1', 'y', 'Y']),
        None => element
            .parent(TreeRelation::Formatting)
            .filter(|parent| html(*parent, "frameset"))
            .is_none_or(has_border),
    }
}

/// HTML §15.6: a frame or frameset's frame border colour.
fn border_color(element: Element<'_>) -> Color {
    element
        .attr("bordercolor")
        .and_then(crate::style::legacy_color)
        .and_then(|color| crate::style::parse_color_value(&color))
        .or_else(|| {
            element
                .parent(TreeRelation::Formatting)
                .filter(|parent| html(*parent, "frameset"))
                .map(border_color)
        })
        .unwrap_or(FRAME_BORDER_COLOR)
}

/// The used style of a frameset box: as large as the viewport, and the containing block of
/// its frames.
pub(super) fn used_style(element: Element<'_>, style: &mut Style, sheet: &Stylesheet) {
    if !html(element, "frameset") || style.display == Display::None {
        return;
    }
    let [width, height] = sheet.page_area();
    style.display = Display::Block;
    style.box_sizing = BoxSizing::BorderBox;
    style.width = Size::Value(LengthPercentage::Length(width));
    style.height = Size::Value(LengthPercentage::Length(height));
    if style.position == Position::Static {
        style.position = Position::Relative;
    }
}

struct Layout<'a> {
    style: &'a Style,
    sheet: &'a Stylesheet,
    embedded: &'a EmbeddedContent,
    page_scope: Option<&'a Arc<str>>,
    boxes: Vec<BlockContent>,
}

/// The frames of a frameset, absolutely positioned in its grid of rectangles.
pub(super) fn content(
    element: Element<'_>,
    style: &Style,
    sheet: &Stylesheet,
    embedded: &EmbeddedContent,
    page_scope: Option<&Arc<str>>,
) -> Option<ControlContent> {
    if !html(element, "frameset") {
        return None;
    }
    let [width, height] = sheet.page_area();
    let mut layout = Layout {
        style,
        sheet,
        embedded,
        page_scope,
        boxes: Vec::new(),
    };
    layout.frameset(element, [0.0, 0.0, width, height]);
    Some(ControlContent {
        image: None,
        children: layout.boxes,
    })
}

impl Layout<'_> {
    /// Lays out `frameset` into the `[x, y, width, height]` rectangle.
    fn frameset(&mut self, frameset: Element<'_>, [x, y, width, height]: [f32; 4]) {
        let columns = tracks(frameset, "cols", width);
        let rows = tracks(frameset, "rows", height);
        let mut children = frameset
            .children(TreeRelation::Formatting)
            .filter(|child| html(*child, "frame") || html(*child, "frameset"));
        let mut top = y;
        for row in &rows {
            let mut left = x;
            for column in &columns {
                let Some(child) = children.next() else {
                    return;
                };
                let rectangle = [left, top, *column, *row];
                if child.name() == "frameset" {
                    self.frameset(child, rectangle);
                } else {
                    self.frame(child, rectangle);
                }
                left += column;
            }
            top += row;
        }
    }

    fn frame(&mut self, frame: Element<'_>, [x, y, width, height]: [f32; 4]) {
        let mut style = self.style.anonymous(Display::Block);
        style.position = Position::Absolute;
        style.box_sizing = BoxSizing::BorderBox;
        style.inset = [
            Size::Value(LengthPercentage::Length(y)),
            Size::Auto,
            Size::Auto,
            Size::Value(LengthPercentage::Length(x)),
        ];
        style.width = Size::Value(LengthPercentage::Length(width));
        style.height = Size::Value(LengthPercentage::Length(height));
        if has_border(frame) {
            let border = Border {
                width: FRAME_BORDER_PX * POINTS_PER_PIXEL,
                style: BorderStyle::Solid,
                color: border_color(frame),
            };
            style.decoration.borders = [border; 4];
        }
        let image = match self.embedded.content(frame.id()) {
            Some(Embedded::Navigable(navigable)) => Arc::new(Image::Frame(
                navigable.clone(),
                self.sheet.used_color_scheme(&self.style.color_scheme),
            )),
            _ => Arc::new(Image::Transparent),
        };
        self.boxes
            .push(BlockContent::Positioned(BlockBox::anonymous_replaced(
                style,
                image,
                self.page_scope.cloned(),
            )));
    }
}
