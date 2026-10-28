use crate::style::ListStyleType;

const FIRST_ORDINAL: i128 = 1;
const MAX_ROMAN_ORDINAL: i128 = 3999;
const LOWER_LATIN: &str = "abcdefghijklmnopqrstuvwxyz";
const UPPER_LATIN: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZ";
const LOWER_GREEK: &str = "αβγδεζηθικλμνξοπρστυφχψω";
const ROMAN_DIGITS: [(i128, &str); 13] = [
    (1000, "m"),
    (900, "cm"),
    (500, "d"),
    (400, "cd"),
    (100, "c"),
    (90, "xc"),
    (50, "l"),
    (40, "xl"),
    (10, "x"),
    (9, "ix"),
    (5, "v"),
    (4, "iv"),
    (1, "i"),
];

fn alphabetic(mut value: i128, alphabet: &str) -> String {
    if value < FIRST_ORDINAL {
        return value.to_string();
    }
    let radix = alphabet.chars().count() as i128;
    let mut digits = Vec::new();
    while value > 0 {
        value -= FIRST_ORDINAL;
        digits.push(alphabet.chars().nth((value % radix) as usize).unwrap());
        value /= radix;
    }
    digits.into_iter().rev().collect()
}
fn roman(mut value: i128) -> String {
    if !(FIRST_ORDINAL..=MAX_ROMAN_ORDINAL).contains(&value) {
        return value.to_string();
    }
    let mut result = String::new();
    for (amount, digit) in ROMAN_DIGITS {
        while value >= amount {
            result.push_str(digit);
            value -= amount;
        }
    }
    result
}

impl ListStyleType {
    /// CSS counter representation, without a list marker's prefix or suffix.
    pub(crate) fn representation(self, value: i128) -> String {
        match self {
            Self::None => String::new(),
            Self::Disc => "•".into(),
            Self::Circle => "◦".into(),
            Self::Square => "▪".into(),
            Self::Decimal => value.to_string(),
            Self::DecimalLeadingZero => format!("{value:02}"),
            Self::LowerRoman => roman(value),
            Self::UpperRoman => roman(value).to_uppercase(),
            Self::LowerAlpha => alphabetic(value, LOWER_LATIN),
            Self::UpperAlpha => alphabetic(value, UPPER_LATIN),
            Self::LowerGreek => alphabetic(value, LOWER_GREEK),
        }
    }

    /// Default suffix for the predefined counter style (CSS Counter Styles 3 §6).
    pub(crate) fn suffix(self) -> &'static str {
        match self {
            Self::None => "",
            Self::Disc | Self::Circle | Self::Square => " ",
            _ => ". ",
        }
    }
}
