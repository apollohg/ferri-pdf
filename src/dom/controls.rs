//! Form controls' initial state (HTML §4.10): values after sanitization, checkedness and
//! selectedness, and the constraint validation states the static pseudo-classes expose.
use crate::dom::control_values::{self as values, MS_PER_DAY, MS_PER_SECOND, MS_PER_WEEK};
use crate::dom::state::is_html_whitespace;
use crate::dom::{
    Element, HTML_NAMESPACE, InitialState, SVG_NAMESPACE, TreeRelation, non_negative_integer_digits,
};

const SINGLE_SELECT_SIZE: u32 = 1;
const MULTIPLE_SELECT_SIZE: u32 = 4;
const DECIMAL_RADIX: u32 = 10;
/// The value of a checkbox or radio button without a `value` attribute (HTML §4.10.5.4).
const DEFAULT_ON: &str = "on";
/// HTML §4.10.5.1.14: a colour well without a parseable value shows opaque black.
pub(crate) const DEFAULT_COLOR: &str = "#000000";
const STEP_ANY: &str = "any";
const RANGE_DEFAULT_MINIMUM: f64 = 0.0;
const RANGE_DEFAULT_MAXIMUM: f64 = 100.0;
const METER_DEFAULT_MAXIMUM: f64 = 1.0;
const PROGRESS_DEFAULT_MAXIMUM: f64 = 1.0;
/// HTML §4.10.5.1.10: the default step base of a week control, Monday 1969-12-29.
const WEEK_STEP_BASE: f64 = -259_200_000.0;
const TIME_DEFAULT_STEP: f64 = 60.0;
/// Values are decimal strings, so a step multiple that binary floating point misses only by
/// rounding error is still a multiple.
const STEP_TOLERANCE: f64 = 1e-9;
const WIDGET_ELEMENTS: [&str; 6] = ["button", "input", "meter", "progress", "select", "textarea"];
const BUTTON_TYPES: [&str; 3] = ["submit", "reset", "button"];
const CONTENT_EDITABLE_TRUE: [&str; 2] = ["", "true"];
const CONTENT_EDITABLE_PLAINTEXT: &str = "plaintext-only";
const CONTENT_EDITABLE_FALSE: &str = "false";
/// HTML §4.10.5.3.6 compiles patterns with RegExpCreate(pattern, "v").
const PATTERN_FLAGS: &str = "v";

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ControlAncestry {
    fieldset_inherited: bool,
    fieldset_descendants: bool,
    first_legend: Option<ego_tree::NodeId>,
    select_disabled: bool,
    option_disabled: bool,
    option_children_disabled: bool,
}

pub(crate) fn initial_disabled(
    element: Element<'_>,
    parent: Option<&InitialState>,
) -> (Option<bool>, ControlAncestry) {
    let parent = parent.map_or_else(ControlAncestry::default, |state| state.control_ancestry);
    let inherited = if parent.first_legend == Some(element.raw.id()) {
        parent.fieldset_inherited
    } else {
        parent.fieldset_descendants
    };
    let html = element.namespace() == HTML_NAMESPACE;
    let own = html && element.attr("disabled").is_some();
    let option_disabled =
        html_element(element, "option") && (own || parent.option_children_disabled);
    let disabled = if html {
        match element.name() {
            "button" | "input" | "select" | "textarea" | "fieldset" => Some(own || inherited),
            "optgroup" => Some(own || parent.select_disabled),
            "option" => Some(option_disabled || parent.select_disabled),
            _ => None,
        }
    } else {
        None
    };
    let fieldset = html && element.name() == "fieldset";
    let first_legend = fieldset
        .then(|| {
            element
                .children(TreeRelation::Selector)
                .find(|child| html_element(*child, "legend"))
                .map(|child| child.raw.id())
        })
        .flatten();
    let ancestry = ControlAncestry {
        fieldset_inherited: inherited,
        fieldset_descendants: inherited || (fieldset && own),
        first_legend,
        select_disabled: if html && element.name() == "select" {
            disabled == Some(true)
        } else {
            parent.select_disabled
        },
        option_disabled,
        option_children_disabled: if html {
            match element.name() {
                "optgroup" => own,
                "select" | "hr" | "datalist" | "option" => false,
                _ => parent.option_children_disabled,
            }
        } else {
            parent.option_children_disabled
        },
    };
    (disabled, ancestry)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum InputType {
    Hidden,
    Text,
    Search,
    Tel,
    Url,
    Email,
    Password,
    Date,
    Month,
    Week,
    Time,
    DateTimeLocal,
    Number,
    Range,
    Color,
    Checkbox,
    Radio,
    File,
    Submit,
    Image,
    Reset,
    Button,
}

/// `input` content attributes whose applicability depends on the `type` state (HTML §4.10.5,
/// the summary table of content attributes).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum InputAttribute {
    Checked,
    MinMaxStep,
    Multiple,
    Pattern,
    Placeholder,
    ReadOnly,
    Required,
    Size,
}

impl InputType {
    pub(crate) fn of(element: Element<'_>) -> Self {
        match element
            .attr("type")
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str()
        {
            "hidden" => Self::Hidden,
            "search" => Self::Search,
            "tel" => Self::Tel,
            "url" => Self::Url,
            "email" => Self::Email,
            "password" => Self::Password,
            "date" => Self::Date,
            "month" => Self::Month,
            "week" => Self::Week,
            "time" => Self::Time,
            "datetime-local" => Self::DateTimeLocal,
            "number" => Self::Number,
            "range" => Self::Range,
            "color" => Self::Color,
            "checkbox" => Self::Checkbox,
            "radio" => Self::Radio,
            "file" => Self::File,
            "submit" => Self::Submit,
            "image" => Self::Image,
            "reset" => Self::Reset,
            "button" => Self::Button,
            _ => Self::Text,
        }
    }
    /// The states whose value is one line of user text.
    pub(crate) fn is_text_field(self) -> bool {
        matches!(
            self,
            Self::Text | Self::Search | Self::Tel | Self::Url | Self::Email | Self::Password
        )
    }
    pub(crate) fn is_temporal(self) -> bool {
        matches!(
            self,
            Self::Date | Self::Month | Self::Week | Self::Time | Self::DateTimeLocal
        )
    }
    pub(crate) fn applies(self, attribute: InputAttribute) -> bool {
        let text = self.is_text_field();
        let temporal = self.is_temporal();
        match attribute {
            InputAttribute::Checked => matches!(self, Self::Checkbox | Self::Radio),
            InputAttribute::MinMaxStep => temporal || matches!(self, Self::Number | Self::Range),
            InputAttribute::Multiple => matches!(self, Self::Email | Self::File),
            InputAttribute::Pattern | InputAttribute::Size => text,
            InputAttribute::Placeholder => text || self == Self::Number,
            InputAttribute::ReadOnly => text || temporal || self == Self::Number,
            InputAttribute::Required => {
                text || temporal
                    || matches!(
                        self,
                        Self::Number | Self::Checkbox | Self::Radio | Self::File
                    )
            }
        }
    }
    pub(crate) fn auto_direction_value(self, element: Element<'_>) -> Option<String> {
        let value = element.attr("value").unwrap_or_default();
        match self {
            Self::Text | Self::Search | Self::Tel | Self::Password | Self::Url | Self::Email => {
                let value = values::stripped_newlines(value);
                Some(if matches!(self, Self::Url | Self::Email) {
                    value.trim_matches(is_html_whitespace).to_owned()
                } else {
                    value
                })
            }
            Self::Hidden | Self::Submit | Self::Reset | Self::Button => Some(value.to_owned()),
            _ => None,
        }
    }
    /// HTML §4.10.5.1 "convert a string to a number", for the states with a numeric domain.
    fn number(self, value: &str) -> Option<f64> {
        match self {
            Self::Number | Self::Range => values::parse_float(value),
            Self::Date => values::date_number(value),
            Self::Month => values::month_number(value),
            Self::Week => values::week_number(value),
            Self::Time => values::time_number(value),
            Self::DateTimeLocal => values::local_date_time_number(value),
            _ => None,
        }
    }
    /// The default step multiplied by the step scale factor, and the default step base.
    fn step_defaults(self) -> (f64, f64) {
        match self {
            Self::Date => (MS_PER_DAY, 0.0),
            Self::Week => (MS_PER_WEEK, WEEK_STEP_BASE),
            Self::Time | Self::DateTimeLocal => (TIME_DEFAULT_STEP * MS_PER_SECOND, 0.0),
            _ => (1.0, 0.0),
        }
    }
    fn step_scale(self) -> f64 {
        match self {
            Self::Date => MS_PER_DAY,
            Self::Week => MS_PER_WEEK,
            Self::Time | Self::DateTimeLocal => MS_PER_SECOND,
            _ => 1.0,
        }
    }
}

/// An input's numeric range: its minimum, maximum, allowed value step and step base
/// (HTML §4.10.5.3.7–8).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct NumericRange {
    pub minimum: Option<f64>,
    pub maximum: Option<f64>,
    pub step: Option<f64>,
    pub base: f64,
}
impl NumericRange {
    /// Time controls have a periodic domain, where a maximum below the minimum spans midnight.
    fn reversed(&self, kind: InputType) -> bool {
        kind == InputType::Time
            && matches!((self.minimum, self.maximum), (Some(min), Some(max)) if max < min)
    }
    fn mismatches_step(&self, number: f64) -> bool {
        self.step.is_some_and(|step| {
            let steps = (number - self.base) / step;
            (steps - steps.round()).abs() > STEP_TOLERANCE * steps.abs().max(1.0)
        })
    }
}

/// The six points of a `meter` gauge (HTML §4.10.14), in document order of evaluation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct MeterGauge {
    pub minimum: f64,
    pub maximum: f64,
    pub value: f64,
    pub low: f64,
    pub high: f64,
    pub optimum: f64,
}
/// Which region of a meter's gauge its value falls in.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum GaugeRegion {
    Optimum,
    Suboptimal,
    EvenLessGood,
}
impl MeterGauge {
    pub(crate) fn region(&self) -> GaugeRegion {
        let Self {
            value,
            low,
            high,
            optimum,
            ..
        } = *self;
        if optimum >= low && optimum <= high {
            if value >= low && value <= high {
                GaugeRegion::Optimum
            } else {
                GaugeRegion::Suboptimal
            }
        } else if optimum < low {
            if value <= low {
                GaugeRegion::Optimum
            } else if value <= high {
                GaugeRegion::Suboptimal
            } else {
                GaugeRegion::EvenLessGood
            }
        } else if value >= high {
            GaugeRegion::Optimum
        } else if value >= low {
            GaugeRegion::Suboptimal
        } else {
            GaugeRegion::EvenLessGood
        }
    }
}

fn html_element(element: Element<'_>, name: &str) -> bool {
    element.namespace() == HTML_NAMESPACE && element.name() == name
}

pub(crate) fn non_negative_integer(value: &str) -> Option<u32> {
    non_negative_integer_digits(value).map(|digits| {
        digits.bytes().fold(0u32, |value, digit| {
            value
                .saturating_mul(DECIMAL_RADIX)
                .saturating_add(u32::from(digit - b'0'))
        })
    })
}

impl<'a> Element<'a> {
    /// HTML §15.5.1: the elements that can have a native appearance.
    pub(crate) fn is_widget(self) -> bool {
        self.namespace() == HTML_NAMESPACE && WIDGET_ELEMENTS.contains(&self.name())
    }
    pub(crate) fn input_type(self) -> Option<InputType> {
        self.initial_state().input_type
    }
    fn input_applies(self, attribute: InputAttribute) -> bool {
        self.input_type()
            .is_some_and(|kind| kind.applies(attribute))
    }
    fn tree_elements(self) -> impl Iterator<Item = Self> {
        let mut root = self.as_node();
        while let Some(parent) = root.parent(TreeRelation::Selector) {
            root = parent;
        }
        root.inclusive_descendants(TreeRelation::Selector)
            .filter_map(super::tree::Node::element)
    }
    fn form_owner(self) -> Option<Self> {
        if let Some(id) = self.attr("form") {
            if id.is_empty() {
                return None;
            }
            return self
                .tree_elements()
                .find(|element| element.attr("id") == Some(id))
                .filter(|element| html_element(*element, "form"));
        }
        self.ancestors(TreeRelation::Selector)
            .find(|element| html_element(*element, "form"))
    }
    fn disabled(self) -> bool {
        self.initial_state().disabled == Some(true)
    }

    /// HTML §4.10.5.1.17: the radio buttons sharing a named group with this one, itself
    /// included; an unnamed radio button is alone in its group.
    fn radio_group(self) -> Vec<Self> {
        let Some(name) = self.attr("name").filter(|name| !name.is_empty()) else {
            return vec![self];
        };
        let owner = self.form_owner();
        self.tree_elements()
            .filter(|element| {
                html_element(*element, "input")
                    && element.input_type() == Some(InputType::Radio)
                    && element.attr("name") == Some(name)
                    && element.form_owner() == owner
            })
            .collect()
    }

    pub(crate) fn initially_checked(self) -> bool {
        if !html_element(self, "input") || self.attr("checked").is_none() {
            return false;
        }
        match self.input_type() {
            Some(InputType::Checkbox) => true,
            Some(InputType::Radio) => self
                .radio_group()
                .into_iter()
                .filter(|element| element.attr("checked").is_some())
                .last()
                .is_some_and(|element| element == self),
            _ => false,
        }
    }

    /// The `:checked` state: checked checkboxes and radio buttons and selected options.
    pub(crate) fn is_checked(self) -> bool {
        if html_element(self, "option") {
            return self.option_selected();
        }
        self.initially_checked()
    }

    /// Checkboxes, radio buttons and options: the elements `:checked` can match.
    pub(crate) fn is_checkable(self) -> bool {
        html_element(self, "option") || self.input_applies(InputAttribute::Checked)
    }

    fn option_selected(self) -> bool {
        match self
            .ancestors(TreeRelation::Selector)
            .find(|ancestor| html_element(*ancestor, "select"))
            .filter(|select| select.select_options().contains(&self))
        {
            Some(select) => select.initial_selected_options().contains(&self),
            None => self.attr("selected").is_some(),
        }
    }

    /// The `:indeterminate` state: radio buttons whose group has none checked, and progress
    /// bars without a value. Checkbox indeterminateness is only set by script.
    pub(crate) fn is_indeterminate(self) -> bool {
        if html_element(self, "progress") {
            return self.attr("value").is_none();
        }
        html_element(self, "input")
            && self.input_type() == Some(InputType::Radio)
            && !self
                .radio_group()
                .into_iter()
                .any(Element::initially_checked)
    }

    /// The `:default` state: each form's default button, inputs with a `checked` attribute
    /// that applies, and options with a `selected` attribute.
    pub(crate) fn is_default(self) -> bool {
        if html_element(self, "option") {
            return self.attr("selected").is_some();
        }
        if self.input_applies(InputAttribute::Checked) {
            return self.attr("checked").is_some();
        }
        if !self.is_submit_button() {
            return false;
        }
        let Some(owner) = self.form_owner() else {
            return false;
        };
        self.tree_elements()
            .find(|element| element.is_submit_button() && element.form_owner() == Some(owner))
            == Some(self)
    }

    fn is_submit_button(self) -> bool {
        (html_element(self, "input")
            && matches!(
                self.input_type(),
                Some(InputType::Submit | InputType::Image)
            ))
            || (html_element(self, "button") && self.button_type() == ButtonType::Submit)
    }

    pub(crate) fn button_type(self) -> ButtonType {
        match self
            .attr("type")
            .map(str::to_ascii_lowercase)
            .as_deref()
            .filter(|kind| BUTTON_TYPES.contains(kind))
        {
            Some("reset") => ButtonType::Reset,
            Some("button") => ButtonType::Button,
            _ => ButtonType::Submit,
        }
    }

    /// The placeholder text a control presents while its value is empty.
    pub(crate) fn shown_placeholder(self) -> Option<&'a str> {
        let applies = html_element(self, "textarea")
            || (html_element(self, "input") && self.input_applies(InputAttribute::Placeholder));
        let placeholder = self.attr("placeholder").filter(|text| !text.is_empty())?;
        (applies && self.control_value().is_some_and(|value| value.is_empty()))
            .then_some(placeholder)
    }

    /// `Some(true)` for `:required` and `Some(false)` for `:optional` elements.
    pub(crate) fn required_state(self) -> Option<bool> {
        if html_element(self, "select") || html_element(self, "textarea") {
            return Some(self.attr("required").is_some());
        }
        (html_element(self, "input") && self.input_applies(InputAttribute::Required))
            .then(|| self.attr("required").is_some())
    }

    /// The `:read-write` state: mutable text controls and editable content.
    pub(crate) fn is_read_write(self) -> bool {
        if self.namespace() != HTML_NAMESPACE {
            return false;
        }
        match self.name() {
            "input" => {
                self.input_applies(InputAttribute::ReadOnly)
                    && self.attr("readonly").is_none()
                    && !self.disabled()
            }
            "textarea" => self.attr("readonly").is_none() && !self.disabled(),
            _ => self.is_editable(),
        }
    }

    /// HTML §6.8.1: editing hosts and the descendants they make editable.
    fn is_editable(self) -> bool {
        for element in std::iter::once(self).chain(self.ancestors(TreeRelation::Selector)) {
            if element.namespace() != HTML_NAMESPACE {
                continue;
            }
            if let Some(state) = element.attr("contenteditable") {
                let state = state.to_ascii_lowercase();
                if CONTENT_EDITABLE_TRUE.contains(&state.as_str())
                    || state == CONTENT_EDITABLE_PLAINTEXT
                {
                    return true;
                }
                if state == CONTENT_EDITABLE_FALSE {
                    return false;
                }
            }
        }
        false
    }

    /// The `:open` state: `details` and `dialog` elements with an `open` attribute. Pickers
    /// and drop-down boxes only open on user interaction.
    pub(crate) fn is_open(self) -> bool {
        (html_element(self, "details") || html_element(self, "dialog"))
            && self.attr("open").is_some()
    }

    pub(crate) fn initial_selected_options(self) -> Vec<Self> {
        if !html_element(self, "select") {
            return Vec::new();
        }
        let options = self.select_options();
        let mut selected: Vec<_> = options
            .iter()
            .copied()
            .filter(|option| option.attr("selected").is_some())
            .collect();
        if self.attr("multiple").is_some() {
            return selected;
        }
        if let Some(last) = selected.pop() {
            return vec![last];
        }
        if self.display_size() != SINGLE_SELECT_SIZE {
            return Vec::new();
        }
        options
            .into_iter()
            .find(|option| !option.initial_state().control_ancestry.option_disabled)
            .into_iter()
            .collect()
    }

    /// HTML §4.10.7: a `select` element's display size.
    pub(crate) fn display_size(self) -> u32 {
        self.attr("size")
            .and_then(non_negative_integer)
            .filter(|size| *size != 0)
            .unwrap_or(if self.attr("multiple").is_some() {
                MULTIPLE_SELECT_SIZE
            } else {
                SINGLE_SELECT_SIZE
            })
    }

    /// HTML §4.10.7: whether a `select` renders as a list box rather than a drop-down box.
    pub(crate) fn is_list_box(self) -> bool {
        self.attr("multiple").is_some() || self.display_size() > SINGLE_SELECT_SIZE
    }

    /// A `select` element's list of options, in tree order.
    pub(crate) fn select_options(self) -> Vec<Self> {
        let mut pending: Vec<_> = self
            .children(TreeRelation::Selector)
            .rev()
            .map(|element| (element, false))
            .collect();
        let mut options = Vec::new();
        while let Some((element, in_group)) = pending.pop() {
            let html = element.namespace() == HTML_NAMESPACE;
            let group = html && element.name() == "optgroup";
            if html && element.name() == "option" {
                options.push(element);
            }
            if (html && matches!(element.name(), "select" | "hr" | "option" | "datalist"))
                || (group && in_group)
            {
                continue;
            }
            pending.extend(
                element
                    .children(TreeRelation::Selector)
                    .rev()
                    .map(|child| (child, in_group || group)),
            );
        }
        options
    }

    /// An `option` element's text: its descendant text outside scripts and images, with
    /// white space stripped and collapsed.
    fn option_text(self) -> String {
        let text: String = self
            .as_node()
            .inclusive_descendants(TreeRelation::Selector)
            .filter(|node| {
                !node
                    .parent(TreeRelation::Selector)
                    .into_iter()
                    .filter_map(super::tree::Node::element)
                    .flat_map(|parent| {
                        std::iter::once(parent).chain(parent.ancestors(TreeRelation::Selector))
                    })
                    .take_while(|ancestor| *ancestor != self)
                    .any(|ancestor| {
                        (matches!(ancestor.namespace(), HTML_NAMESPACE | SVG_NAMESPACE)
                            && ancestor.name() == "script")
                            || html_element(ancestor, "img")
                    })
            })
            .filter_map(|node| node.value().as_text().map(|text| &**text))
            .collect();
        text.split(is_html_whitespace)
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join(" ")
    }

    pub(crate) fn option_value(self) -> Option<String> {
        if !html_element(self, "option") {
            return None;
        }
        Some(
            self.attr("value")
                .map_or_else(|| self.option_text(), str::to_owned),
        )
    }

    /// An `option` or `optgroup` element's label.
    pub(crate) fn option_label(self) -> String {
        if let Some(label) = self.attr("label").filter(|label| !label.is_empty()) {
            return label.to_owned();
        }
        if html_element(self, "option") {
            self.option_text()
        } else {
            String::new()
        }
    }

    /// HTML §4.10.7: the select's placeholder label option.
    fn placeholder_label_option(self) -> Option<Self> {
        if self.attr("required").is_none()
            || self.attr("multiple").is_some()
            || self.display_size() != SINGLE_SELECT_SIZE
        {
            return None;
        }
        self.select_options().into_iter().next().filter(|option| {
            option.option_value().is_some_and(|value| value.is_empty())
                && option.parent(TreeRelation::Selector) == Some(self)
        })
    }

    /// A `textarea` element's raw value: its child text content.
    pub(crate) fn textarea_value(self) -> String {
        self.child_nodes(TreeRelation::Selector)
            .filter_map(|node| node.value().as_text().map(|text| &**text))
            .collect()
    }

    /// The value a text-like control holds: an input's sanitized value or a textarea's raw
    /// value.
    pub(crate) fn control_value(self) -> Option<String> {
        if html_element(self, "textarea") {
            return Some(self.textarea_value());
        }
        html_element(self, "input").then(|| self.input_value())
    }

    /// An input's value after the type state's value sanitization algorithm (HTML §4.10.5.1),
    /// in the IDL attribute's value mode.
    pub(crate) fn input_value(self) -> String {
        let kind = self.input_type().expect("HTML input has an initial type");
        let raw = self.attr("value");
        let value = raw.unwrap_or_default();
        match kind {
            InputType::Hidden
            | InputType::Submit
            | InputType::Image
            | InputType::Reset
            | InputType::Button => value.to_owned(),
            InputType::Checkbox | InputType::Radio => raw.unwrap_or(DEFAULT_ON).to_owned(),
            InputType::File => String::new(),
            InputType::Text | InputType::Search | InputType::Tel | InputType::Password => {
                values::stripped_newlines(value)
            }
            InputType::Url => values::trimmed_line(value),
            InputType::Email if self.attr("multiple").is_some() => value
                .split(',')
                .map(|address| address.trim_matches(is_html_whitespace))
                .collect::<Vec<_>>()
                .join(","),
            InputType::Email => values::trimmed_line(value),
            InputType::Number => Self::valid_or_empty(value, values::valid_float),
            InputType::Date => Self::valid_or_empty(value, values::valid_date),
            InputType::Month => Self::valid_or_empty(value, values::valid_month),
            InputType::Week => Self::valid_or_empty(value, values::valid_week),
            InputType::Time => Self::valid_or_empty(value, values::valid_time),
            InputType::DateTimeLocal => {
                values::normalized_local_date_time(value).unwrap_or_default()
            }
            InputType::Range => self.range_number().to_string(),
            InputType::Color => self.color_value(),
        }
    }

    fn valid_or_empty(value: &str, valid: fn(&str) -> bool) -> String {
        if valid(value) {
            value.to_owned()
        } else {
            String::new()
        }
    }

    /// HTML §4.10.5.1.14: the colour a colour well's value attribute parses to, or opaque
    /// black. The well is drawn from the parsed colour, so the value is kept as written.
    fn color_value(self) -> String {
        self.attr("value")
            .filter(|value| crate::style::parse_color_value(value).is_some())
            .map_or_else(|| DEFAULT_COLOR.to_owned(), str::to_owned)
    }

    /// An input's minimum, maximum, step and step base, for the states they apply to.
    pub(crate) fn numeric_range(self) -> Option<NumericRange> {
        let kind = self.input_type()?;
        if !kind.applies(InputAttribute::MinMaxStep) {
            return None;
        }
        let parse = |name: &str| self.attr(name).and_then(|value| kind.number(value));
        let (default_minimum, default_maximum) = if kind == InputType::Range {
            (Some(RANGE_DEFAULT_MINIMUM), Some(RANGE_DEFAULT_MAXIMUM))
        } else {
            (None, None)
        };
        let (default_step, default_base) = kind.step_defaults();
        let step = match self.attr("step") {
            Some(step) if step.eq_ignore_ascii_case(STEP_ANY) => None,
            Some(step) => Some(
                values::parse_float(step)
                    .filter(|step| *step > 0.0)
                    .map_or(default_step, |step| step * kind.step_scale()),
            ),
            None => Some(default_step),
        };
        Some(NumericRange {
            minimum: parse("min").or(default_minimum),
            maximum: parse("max").or(default_maximum),
            step,
            base: parse("min")
                .or_else(|| parse("value"))
                .unwrap_or(default_base),
        })
    }

    /// HTML §4.10.5.1.13: a range control's value after sanitization, clamped to its range
    /// and rounded to its step.
    pub(crate) fn range_number(self) -> f64 {
        let range = self
            .numeric_range()
            .expect("range controls have a numeric range");
        let minimum = range
            .minimum
            .expect("range controls have a default minimum");
        let maximum = range
            .maximum
            .expect("range controls have a default maximum");
        let reversed = maximum < minimum;
        let mut number = self
            .attr("value")
            .filter(|value| values::valid_float(value))
            .and_then(values::parse_float)
            .unwrap_or(if reversed {
                minimum
            } else {
                minimum + (maximum - minimum) / 2.0
            });
        if number < minimum {
            number = minimum;
        } else if !reversed && number > maximum {
            number = maximum;
        }
        if let Some(step) = range.step
            && range.mismatches_step(number)
        {
            let below = range.base + ((number - range.base) / step).floor() * step;
            let above = below + step;
            let fits = |candidate: f64| candidate >= minimum && (reversed || candidate <= maximum);
            number = match (fits(below), fits(above)) {
                (true, true) if number - below < above - number => below,
                (_, true) => above,
                (true, false) => below,
                (false, false) => number,
            };
        }
        number
    }

    /// HTML §4.10.14: a meter's gauge points.
    pub(crate) fn meter_gauge(self) -> MeterGauge {
        let parse = |name: &str| self.attr(name).and_then(values::parse_float);
        let minimum = parse("min").unwrap_or(0.0);
        let maximum = parse("max").unwrap_or(METER_DEFAULT_MAXIMUM).max(minimum);
        let value = parse("value").unwrap_or(0.0).clamp(minimum, maximum);
        let low = parse("low").unwrap_or(minimum).clamp(minimum, maximum);
        let high = parse("high").unwrap_or(maximum).clamp(low, maximum);
        let optimum = parse("optimum")
            .unwrap_or(minimum + (maximum - minimum) / 2.0)
            .clamp(minimum, maximum);
        MeterGauge {
            minimum,
            maximum,
            value,
            low,
            high,
            optimum,
        }
    }

    /// HTML §4.10.13: a determinate progress bar's position from zero to one, or `None` for
    /// an indeterminate one.
    pub(crate) fn progress_position(self) -> Option<f64> {
        let value = self.attr("value")?;
        let maximum = self
            .attr("max")
            .and_then(values::parse_float)
            .filter(|maximum| *maximum > 0.0)
            .unwrap_or(PROGRESS_DEFAULT_MAXIMUM);
        let value = values::parse_float(value)
            .unwrap_or(0.0)
            .clamp(0.0, maximum);
        Some(value / maximum)
    }

    pub(crate) fn is_blank_control(self) -> bool {
        if self.namespace() != HTML_NAMESPACE {
            return false;
        }
        match self.name() {
            "textarea" => self.textarea_value().is_empty(),
            "select" => self
                .initial_selected_options()
                .into_iter()
                .all(|option| option.option_value().is_none_or(|value| value.is_empty())),
            "input" => match self.input_type().expect("HTML input has an initial type") {
                InputType::Hidden
                | InputType::Range
                | InputType::Color
                | InputType::Submit
                | InputType::Image
                | InputType::Reset
                | InputType::Button => false,
                InputType::Checkbox | InputType::Radio => !self.initially_checked(),
                _ => self.input_value().is_empty(),
            },
            _ => false,
        }
    }

    /// HTML §4.10.20.2: a submittable element that nothing bars from constraint validation.
    fn is_validation_candidate(self) -> bool {
        if self.namespace() != HTML_NAMESPACE || self.disabled() {
            return false;
        }
        let submittable = match self.name() {
            "input" => {
                let barred_type = matches!(
                    self.input_type(),
                    Some(InputType::Hidden | InputType::Reset | InputType::Button)
                );
                let read_only =
                    self.input_applies(InputAttribute::ReadOnly) && self.attr("readonly").is_some();
                !barred_type && !read_only
            }
            "textarea" => self.attr("readonly").is_none(),
            "select" => true,
            "button" => self.button_type() == ButtonType::Submit,
            _ => false,
        };
        submittable
            && !self
                .ancestors(TreeRelation::Selector)
                .any(|ancestor| html_element(ancestor, "datalist"))
    }

    /// HTML §4.10.5.3.6: a non-empty value, or with `multiple` each of the values, that the
    /// compiled pattern regular expression does not match. A pattern that does not compile
    /// as a `v`-flag JavaScript regular expression imposes no constraint.
    fn pattern_mismatch(self) -> bool {
        let Some(pattern) = self
            .attr("pattern")
            .filter(|_| self.input_applies(InputAttribute::Pattern))
        else {
            return false;
        };
        let value = self.input_value();
        if value.is_empty() || regress::Regex::with_flags(pattern, PATTERN_FLAGS).is_err() {
            return false;
        }
        let anchored = regress::Regex::with_flags(&format!("^(?:{pattern})$"), PATTERN_FLAGS)
            .expect("an anchored valid pattern stays valid");
        if self.attr("multiple").is_some() && self.input_applies(InputAttribute::Multiple) {
            value.split(',').any(|value| anchored.find(value).is_none())
        } else {
            anchored.find(&value).is_none()
        }
    }

    /// Whether a validation candidate satisfies its constraints in its initial state, where
    /// nothing is too long, too short, badly input or custom-invalid.
    fn satisfies_constraints(self) -> bool {
        !self.value_missing()
            && !self.type_mismatch()
            && !self.pattern_mismatch()
            && self.range_flags() == [false; 2]
            && {
                let mismatch = self.input_type().and_then(|kind| {
                    let range = self.numeric_range()?;
                    Some(range.mismatches_step(kind.number(&self.input_value())?))
                });
                mismatch != Some(true)
            }
    }

    fn value_missing(self) -> bool {
        match self.name() {
            "textarea" => self.attr("required").is_some() && self.textarea_value().is_empty(),
            "select" => {
                let selected = self.initial_selected_options();
                self.attr("required").is_some()
                    && (selected.is_empty()
                        || selected.as_slice() == self.placeholder_label_option().as_slice())
            }
            "input" if self.input_applies(InputAttribute::Required) => {
                match self.input_type().expect("HTML input has an initial type") {
                    InputType::Checkbox => {
                        self.attr("required").is_some() && !self.initially_checked()
                    }
                    InputType::Radio => {
                        let group = self.radio_group();
                        group
                            .iter()
                            .any(|element| element.attr("required").is_some())
                            && !group.iter().any(|element| element.initially_checked())
                    }
                    InputType::File => self.attr("required").is_some(),
                    _ => self.attr("required").is_some() && self.input_value().is_empty(),
                }
            }
            _ => false,
        }
    }

    fn type_mismatch(self) -> bool {
        let Some(kind @ (InputType::Email | InputType::Url)) = self.input_type() else {
            return false;
        };
        let value = self.input_value();
        !value.is_empty()
            && !match kind {
                InputType::Url => values::valid_absolute_url(&value),
                _ if self.attr("multiple").is_some() => values::valid_email_list(&value),
                _ => values::valid_email(&value),
            }
    }

    /// Whether the value suffers from an underflow and from an overflow.
    fn range_flags(self) -> [bool; 2] {
        let Some(kind) = self.input_type() else {
            return [false; 2];
        };
        let (Some(range), Some(number)) = (self.numeric_range(), kind.number(&self.input_value()))
        else {
            return [false; 2];
        };
        if range.reversed(kind) {
            let (minimum, maximum) = (range.minimum.unwrap(), range.maximum.unwrap());
            let outside = number > maximum && number < minimum;
            return [outside; 2];
        }
        [
            range.minimum.is_some_and(|minimum| number < minimum),
            range.maximum.is_some_and(|maximum| number > maximum),
        ]
    }

    /// `Some(true)` for `:valid` and `Some(false)` for `:invalid` elements: validation
    /// candidates, and the forms and fieldsets containing them.
    pub(crate) fn validity(self) -> Option<bool> {
        if html_element(self, "form") {
            return Some(
                !self.tree_elements().any(|element| {
                    element.form_owner() == Some(self) && element.invalid_candidate()
                }),
            );
        }
        if html_element(self, "fieldset") {
            return Some(
                !self
                    .as_node()
                    .inclusive_descendants(TreeRelation::Selector)
                    .skip(1)
                    .filter_map(super::tree::Node::element)
                    .any(Element::invalid_candidate),
            );
        }
        self.is_validation_candidate()
            .then(|| self.satisfies_constraints())
    }

    fn invalid_candidate(self) -> bool {
        self.is_validation_candidate() && !self.satisfies_constraints()
    }

    /// `Some(true)` for `:in-range` and `Some(false)` for `:out-of-range` elements.
    pub(crate) fn range_state(self) -> Option<bool> {
        let range = self.numeric_range()?;
        (self.is_validation_candidate() && (range.minimum.is_some() || range.maximum.is_some()))
            .then(|| self.range_flags() == [false; 2])
    }
}

/// A `button` element's `type` state (HTML §4.10.6).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ButtonType {
    Submit,
    Reset,
    Button,
}
