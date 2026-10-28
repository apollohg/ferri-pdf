use crate::state::is_html_whitespace;
use crate::{
    Element, HTML_NAMESPACE, InitialState, SVG_NAMESPACE, TreeRelation, non_negative_integer_digits,
};

const SINGLE_SELECT_SIZE: u8 = 1;
const GREGORIAN_CYCLE: u32 = 400;
const MONTHS_PER_YEAR: u8 = 12;
const HOURS_PER_DAY: u8 = 24;
const MINUTES_PER_HOUR: u8 = 60;
const SECONDS_PER_MINUTE: u8 = 60;
const MIN_YEAR_DIGITS: usize = 4;
const DECIMAL_RADIX: u8 = 10;
const DIGIT_PAIR_LENGTH: usize = 2;
const LEAP_PERIOD: u32 = 4;
const CENTURY: u32 = 100;
const DAYS_PER_WEEK: u32 = 7;
const WEDNESDAY: u32 = 3;
const THURSDAY: u32 = 4;
const REGULAR_WEEKS: u8 = 52;
const LONG_WEEKS: u8 = 53;

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
pub enum InputType {
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
    pub(crate) fn auto_direction_value(self, element: Element<'_>) -> Option<String> {
        let value = element.attr("value").unwrap_or_default();
        match self {
            Self::Text | Self::Search | Self::Tel | Self::Password | Self::Url | Self::Email => {
                let value: String = value
                    .chars()
                    .filter(|ch| !matches!(ch, '\r' | '\n'))
                    .collect();
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
}

fn html_element(element: Element<'_>, name: &str) -> bool {
    element.namespace() == HTML_NAMESPACE && element.name() == name
}

impl Element<'_> {
    pub fn input_type(self) -> Option<InputType> {
        self.initial_state().input_type
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

    pub fn initially_checked(self) -> bool {
        if !html_element(self, "input") || self.attr("checked").is_none() {
            return false;
        }
        match self.input_type() {
            Some(InputType::Checkbox) => true,
            Some(InputType::Radio) => {
                let Some(name) = self.attr("name").filter(|name| !name.is_empty()) else {
                    return true;
                };
                let owner = self.form_owner();
                self.tree_elements()
                    .filter(|element| {
                        html_element(*element, "input")
                            && element.input_type() == Some(InputType::Radio)
                            && element.attr("name") == Some(name)
                            && element.form_owner() == owner
                            && element.attr("checked").is_some()
                    })
                    .last()
                    .is_some_and(|element| element == self)
            }
            _ => false,
        }
    }

    pub fn initial_selected_options(self) -> Vec<Self> {
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
        let size = self
            .attr("size")
            .and_then(non_negative_integer_digits)
            .map(|digits| {
                digits.bytes().fold(0u8, |value, digit| {
                    value
                        .saturating_mul(DECIMAL_RADIX)
                        .saturating_add(digit - b'0')
                })
            })
            .filter(|size| *size != 0)
            .unwrap_or(SINGLE_SELECT_SIZE);
        if size != SINGLE_SELECT_SIZE {
            return Vec::new();
        }
        options
            .into_iter()
            .find(|option| !option.initial_state().control_ancestry.option_disabled)
            .into_iter()
            .collect()
    }

    fn select_options(self) -> Vec<Self> {
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

    pub fn option_value(self) -> Option<String> {
        if !html_element(self, "option") {
            return None;
        }
        if let Some(value) = self.attr("value") {
            return Some(value.to_owned());
        }
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
        Some(
            text.split(is_html_whitespace)
                .filter(|part| !part.is_empty())
                .collect::<Vec<_>>()
                .join(" "),
        )
    }

    pub fn is_blank_control(self) -> bool {
        if self.namespace() != HTML_NAMESPACE {
            return false;
        }
        match self.name() {
            "textarea" => self.text().all(str::is_empty),
            "select" => self
                .initial_selected_options()
                .into_iter()
                .all(|option| option.option_value().is_none_or(|value| value.is_empty())),
            "input" => {
                let kind = self.input_type().expect("HTML input has an initial type");
                let value = self.attr("value").unwrap_or_default();
                match kind {
                    InputType::Hidden
                    | InputType::Range
                    | InputType::Color
                    | InputType::Submit
                    | InputType::Image
                    | InputType::Reset
                    | InputType::Button => false,
                    InputType::Checkbox | InputType::Radio => !self.initially_checked(),
                    InputType::File => true,
                    InputType::Number => !valid_number(value),
                    InputType::Date => !valid_date(value),
                    InputType::Month => !valid_month(value),
                    InputType::Week => !valid_week(value),
                    InputType::Time => !valid_time(value),
                    InputType::DateTimeLocal => !value
                        .split_once(['T', ' '])
                        .is_some_and(|(date, time)| valid_date(date) && valid_time(time)),
                    _ => kind
                        .auto_direction_value(self)
                        .is_some_and(|value| value.is_empty()),
                }
            }
            _ => false,
        }
    }
}

fn two_digits(value: &str) -> Option<u8> {
    (value.len() == DIGIT_PAIR_LENGTH && value.bytes().all(|ch| ch.is_ascii_digit()))
        .then(|| (value.as_bytes()[0] - b'0') * DECIMAL_RADIX + value.as_bytes()[1] - b'0')
}
fn year(value: &str) -> Option<u32> {
    (value.len() >= MIN_YEAR_DIGITS
        && value.bytes().all(|ch| ch.is_ascii_digit())
        && value.bytes().any(|ch| ch != b'0'))
    .then(|| {
        value.bytes().fold(0, |year, digit| {
            (year * u32::from(DECIMAL_RADIX) + u32::from(digit - b'0')) % GREGORIAN_CYCLE
        })
    })
}
fn month(value: &str) -> Option<(u32, u8)> {
    let (y, m) = value.split_once('-')?;
    let y = year(y)?;
    let m = two_digits(m)?;
    (m != 0 && m <= MONTHS_PER_YEAR).then_some((y, m))
}
fn leap(year: u32) -> bool {
    year.is_multiple_of(LEAP_PERIOD)
        && (!year.is_multiple_of(CENTURY) || year.is_multiple_of(GREGORIAN_CYCLE))
}
fn valid_month(value: &str) -> bool {
    month(value).is_some()
}
fn valid_date(value: &str) -> bool {
    let Some((ym, day)) = value.rsplit_once('-') else {
        return false;
    };
    let (Some((year, month)), Some(day)) = (month(ym), two_digits(day)) else {
        return false;
    };
    let days = match month {
        2 if leap(year) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    day != 0 && day <= days
}
fn valid_week(value: &str) -> bool {
    let Some((y, week)) = value.split_once("-W") else {
        return false;
    };
    let (Some(y), Some(week)) = (year(y), two_digits(week)) else {
        return false;
    };
    let prior = (y + GREGORIAN_CYCLE - 1) % GREGORIAN_CYCLE;
    let jan_first = (prior + prior / LEAP_PERIOD - prior / CENTURY + 1) % DAYS_PER_WEEK;
    let last = if jan_first == THURSDAY || (jan_first == WEDNESDAY && leap(y)) {
        LONG_WEEKS
    } else {
        REGULAR_WEEKS
    };
    week != 0 && week <= last
}
fn valid_time(value: &str) -> bool {
    let mut parts = value.split(':');
    let (Some(hour), Some(minute)) = (
        parts.next().and_then(two_digits),
        parts.next().and_then(two_digits),
    ) else {
        return false;
    };
    if hour >= HOURS_PER_DAY || minute >= MINUTES_PER_HOUR {
        return false;
    }
    if let Some(seconds) = parts.next() {
        let (seconds, fraction) = seconds
            .split_once('.')
            .map_or((seconds, None), |(s, f)| (s, Some(f)));
        if two_digits(seconds).is_none_or(|s| s >= SECONDS_PER_MINUTE)
            || fraction.is_some_and(|f| f.is_empty() || !f.bytes().all(|ch| ch.is_ascii_digit()))
        {
            return false;
        }
    }
    parts.next().is_none()
}
fn valid_number(value: &str) -> bool {
    let value = value.strip_prefix('-').unwrap_or(value);
    let (significand, exponent) = value
        .split_once(['e', 'E'])
        .map_or((value, None), |(s, e)| (s, Some(e)));
    if exponent.is_some_and(|e| {
        let e = e.strip_prefix(['+', '-']).unwrap_or(e);
        e.is_empty() || !e.bytes().all(|ch| ch.is_ascii_digit())
    }) {
        return false;
    }
    let valid = if let Some((integer, fraction)) = significand.split_once('.') {
        integer.bytes().all(|ch| ch.is_ascii_digit())
            && !fraction.is_empty()
            && fraction.bytes().all(|ch| ch.is_ascii_digit())
    } else {
        !significand.is_empty() && significand.bytes().all(|ch| ch.is_ascii_digit())
    };
    valid && value.parse::<f64>().is_ok_and(f64::is_finite)
}
