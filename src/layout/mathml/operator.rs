//! `MathML` Core §3.2.4.2 and Appendix B.1.
use super::operator_data::{ENTRIES, INLINE_AXIS};
use crate::document::mathml::Attributes;
use crate::style::{LengthPercentage, Style};

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub(super) enum Form {
    #[default]
    Infix,
    Prefix,
    Postfix,
}
impl Form {
    fn attribute(attributes: &Attributes) -> Option<Self> {
        let value = attributes.get("form")?;
        if value.eq_ignore_ascii_case("infix") {
            Some(Self::Infix)
        } else if value.eq_ignore_ascii_case("prefix") {
            Some(Self::Prefix)
        } else if value.eq_ignore_ascii_case("postfix") {
            Some(Self::Postfix)
        } else {
            None
        }
    }
}
#[derive(Clone)]
pub(super) struct Operator {
    pub form: Form,
    pub explicit_form: bool,
    pub space: [f32; 2],
    pub stretchy: bool,
    pub symmetric: bool,
    pub large: bool,
    pub movable: bool,
    pub inline_axis: bool,
    pub min: LengthPercentage,
    pub max: Option<LengthPercentage>,
}
#[derive(Clone, Copy, PartialEq)]
enum Category {
    Default,
    ForceDefault,
    A,
    B,
    C,
    D,
    E,
    F,
    G,
    H,
    I,
    J,
    K,
    L,
    M,
}
const MEDIUM_SPACE_EM: f32 = 4.0 / 18.0;
const THICK_SPACE_EM: f32 = 5.0 / 18.0;
const THIN_SPACE_EM: f32 = 3.0 / 18.0;
const KEY_MASK: u16 = 0x3fff;
const CATEGORY_SHIFT: u32 = 12;
const PREFIX_KEY: u16 = 0x1000;
const POSTFIX_KEY: u16 = 0x2000;
const HIGH_UNICODE_OFFSET: u32 = 0x1c00;
const ASCII_PAIR_BASE: u32 = 0x320;
const ASCII_PAIRS: [&str; 18] = [
    "!!", "!=", "&&", "**", "*=", "++", "+=", "--", "-=", "->", "//", "/=", ":=", "<=", "<>", "==",
    ">=", "||",
];

impl Operator {
    pub(super) fn of(attributes: &Attributes, text: &str, style: &Style, form: Form) -> Self {
        let explicit = Form::attribute(attributes);
        let form = explicit.unwrap_or(form);
        let mut selected = category(text, form);
        if selected == Category::Default && explicit.is_none() {
            for form in [Form::Infix, Form::Postfix, Form::Prefix] {
                selected = category(text, form);
                if selected != Category::Default {
                    break;
                }
            }
        }
        let spacing = match selected {
            Category::Default | Category::ForceDefault | Category::A => [THICK_SPACE_EM; 2],
            Category::B => [MEDIUM_SPACE_EM; 2],
            Category::C | Category::H | Category::J => [THIN_SPACE_EM; 2],
            Category::L => [THIN_SPACE_EM, 0.0],
            Category::M => [0.0, THIN_SPACE_EM],
            _ => [0.0; 2],
        };
        let attribute = |name: &str| attributes.length(name).cloned();
        let boolean = |name: &str, default: bool| match attributes.get(name) {
            Some(value) if value.eq_ignore_ascii_case("true") => true,
            Some(value) if value.eq_ignore_ascii_case("false") => false,
            _ => default,
        };
        let mut chars = text.chars();
        let ch = chars.next().filter(|_| chars.next().is_none());
        Self {
            form,
            explicit_form: explicit.is_some(),
            space: [
                attribute("lspace").map_or(spacing[0] * style.size, |value| {
                    value.resolve(spacing[0] * style.size)
                }),
                attribute("rspace").map_or(spacing[1] * style.size, |value| {
                    value.resolve(spacing[1] * style.size)
                }),
            ],
            stretchy: boolean(
                "stretchy",
                matches!(
                    selected,
                    Category::A | Category::F | Category::G | Category::I
                ),
            ),
            symmetric: boolean(
                "symmetric",
                matches!(
                    selected,
                    Category::F | Category::G | Category::H | Category::J
                ),
            ),
            large: boolean("largeop", matches!(selected, Category::H | Category::J)),
            movable: boolean("movablelimits", selected == Category::J),
            inline_axis: ch.is_some_and(|ch| INLINE_AXIS.binary_search(&u32::from(ch)).is_ok()),
            min: attribute("minsize").unwrap_or(LengthPercentage::Percentage(1.0)),
            max: attribute("maxsize"),
        }
    }
}
fn category(text: &str, form: Form) -> Category {
    let mut characters = text.chars();
    let Some(first) = characters.next() else {
        return Category::Default;
    };
    let second = characters.next();
    if characters.next().is_some() {
        return Category::Default;
    }
    let mut point = u32::from(first);
    if let Some(second) = second {
        if matches!(second, '\u{338}' | '\u{20d2}') {
        } else if let Some(index) = ASCII_PAIRS.iter().position(|pair| *pair == text) {
            point = ASCII_PAIR_BASE + index as u32;
        } else {
            return Category::Default;
        }
    } else if (ASCII_PAIR_BASE..=0x3ff).contains(&point) {
        return Category::Default;
    }
    if second.is_none() && matches!(point, 0x1eef0 | 0x1eef1) && form == Form::Postfix {
        return Category::I;
    }
    if form == Form::Infix && matches!(first, '|' | '∼') {
        return Category::ForceDefault;
    }
    if form == Form::Prefix && matches!(point, 0x2145..=0x2146 | 0x2202 | 0x221a..=0x221c) {
        return Category::L;
    }
    if form == Form::Infix && matches!(first, ',' | ':' | ';') {
        return Category::M;
    }
    let point = if point <= 0x3ff {
        point as u16
    } else if (0x2000..=0x2bff).contains(&point) {
        (point - HIGH_UNICODE_OFFSET) as u16
    } else {
        return Category::Default;
    };
    let key = point
        + match form {
            Form::Infix => 0,
            Form::Prefix => PREFIX_KEY,
            Form::Postfix => POSTFIX_KEY,
        };
    let index = ENTRIES.partition_point(|(start, _)| start & KEY_MASK <= key);
    let Some((start, end)) = index.checked_sub(1).and_then(|index| ENTRIES.get(index)) else {
        return Category::Default;
    };
    if key > end & KEY_MASK {
        return Category::Default;
    }
    match start >> CATEGORY_SHIFT {
        0x0 => Category::A,
        0x4 => Category::B,
        0x8 => Category::C,
        0x1 => Category::D,
        0x2 => Category::E,
        0x5 => Category::F,
        0x6 => Category::G,
        0x9 => Category::H,
        0xa => Category::I,
        0xd => Category::J,
        0xc => Category::K,
        _ => Category::Default,
    }
}
