//! First-letter text (CSS Pseudo-Elements 4 §2.2.1): `([P] [Zs P]*)? [L N S] ([Zs
//! [P--[Ps Pd]]]* [P--[Ps Pd]])?` over typographic character units.
use crate::document::inline::collapses_character;
use crate::style::WhiteSpaceCollapse;
use icu_properties::{
    CodePointMapData,
    props::{GeneralCategory, GeneralCategoryGroup},
};
use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;

/// Zs excludes it from the associated punctuation's typographic space.
const IDEOGRAPHIC_SPACE: char = '\u{3000}';
/// CSS Text 3 §4.1 word-separator characters, which never join the following punctuation.
const WORD_SEPARATORS: [char; 7] = [
    ' ',
    '\u{a0}',
    '\u{1361}',
    '\u{10100}',
    '\u{10101}',
    '\u{1039f}',
    '\u{1091f}',
];
/// Dutch begins a first letter "ij" with both letters (CSS Pseudo-Elements 4 §2.2.1).
const DUTCH: &str = "nl";
const DUTCH_DIGRAPH: [&str; 2] = ["i", "j"];

/// The first-letter text at the start of a run of inline content.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum FirstLetter {
    /// Only white space that collapses away at the start of the line: the text may follow.
    Pending,
    /// The run starts with content that holds no first letter.
    None,
    /// Byte ranges of the preceding punctuation, the letter and the following punctuation.
    Found {
        prefix: Range<usize>,
        letter: Range<usize>,
        suffix: Range<usize>,
    },
}
impl FirstLetter {
    /// The first-letter text's extent, from the start of the run.
    pub(crate) fn end(&self) -> Option<usize> {
        match self {
            Self::Found { suffix, .. } => Some(suffix.end),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Class {
    /// White space that collapses to a space, or another typographic space.
    Space {
        separator: bool,
    },
    Opening,
    Punctuation,
    Letter,
    Other,
}

/// Finds the first-letter text at the start of `text`, a run at the start of a line whose
/// white space `collapse` processes, in content language `language`.
pub(crate) fn find(text: &str, collapse: WhiteSpaceCollapse, language: &str) -> FirstLetter {
    let categories = CodePointMapData::<GeneralCategory>::new();
    let class = |unit: &str| {
        let Some(base) = unit.chars().next() else {
            return Class::Other;
        };
        if unit.chars().all(|ch| collapses_character(ch, collapse)) {
            return Class::Space { separator: true };
        }
        let category = categories.get(base);
        if GeneralCategoryGroup::SpaceSeparator.contains(category) && base != IDEOGRAPHIC_SPACE {
            Class::Space {
                separator: WORD_SEPARATORS.contains(&base),
            }
        } else if matches!(
            category,
            GeneralCategory::OpenPunctuation | GeneralCategory::DashPunctuation
        ) {
            Class::Opening
        } else if GeneralCategoryGroup::Punctuation.contains(category) {
            Class::Punctuation
        } else if [
            GeneralCategoryGroup::Letter,
            GeneralCategoryGroup::Number,
            GeneralCategoryGroup::Symbol,
        ]
        .into_iter()
        .any(|group| group.contains(category))
        {
            Class::Letter
        } else {
            Class::Other
        }
    };
    let mut units = text
        .grapheme_indices(true)
        .map(|(start, unit)| (start..start + unit.len(), class(unit)))
        .peekable();
    while units
        .next_if(|(range, _)| {
            text[range.clone()]
                .chars()
                .all(|ch| collapses_character(ch, collapse))
        })
        .is_some()
    {}
    let Some((first, _)) = units.peek() else {
        return FirstLetter::Pending;
    };
    let start = first.start;
    let mut prefix = start..start;
    if let Some((range, _)) =
        units.next_if(|(_, class)| matches!(class, Class::Opening | Class::Punctuation))
    {
        prefix.end = range.end;
        while let Some((range, _)) = units.next_if(|(_, class)| {
            matches!(
                class,
                Class::Opening | Class::Punctuation | Class::Space { .. }
            )
        }) {
            prefix.end = range.end;
        }
    }
    let Some((mut letter, Class::Letter)) = units.next() else {
        return FirstLetter::None;
    };
    if language
        .split('-')
        .next()
        .is_some_and(|primary| primary.eq_ignore_ascii_case(DUTCH))
        && text[letter.clone()].eq_ignore_ascii_case(DUTCH_DIGRAPH[0])
        && let Some((next, _)) =
            units.next_if(|(range, _)| text[range.clone()].eq_ignore_ascii_case(DUTCH_DIGRAPH[1]))
    {
        letter.end = next.end;
    }
    let mut suffix = letter.end..letter.end;
    while let Some((range, class)) = units.next_if(|(_, class)| {
        matches!(
            class,
            Class::Punctuation | Class::Space { separator: false }
        )
    }) {
        if class == Class::Punctuation {
            suffix.end = range.end;
        }
    }
    FirstLetter::Found {
        prefix,
        letter,
        suffix,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text<'a>(source: &'a str, collapse: WhiteSpaceCollapse, language: &str) -> [&'a str; 3] {
        match find(source, collapse, language) {
            FirstLetter::Found {
                prefix,
                letter,
                suffix,
            } => [&source[prefix], &source[letter], &source[suffix]],
            other => panic!("{source:?}: expected a first letter, got {other:?}"),
        }
    }

    #[test]
    fn first_letter_text_follows_the_associated_punctuation_pattern() {
        let collapse = WhiteSpaceCollapse::Collapse;
        for (source, expected) in [
            ("Hello", ["", "H", ""]),
            ("“The quote”", ["“", "T", ""]),
            ("\"  Spaced", ["\"  ", "S", ""]),
            ("67 million dollars", ["", "6", ""]),
            ("(1) item", ["(", "1", ")"]),
            ("« Bonjour »", ["« ", "B", ""]),
            ("A.B", ["", "A", "."]),
            ("A\u{2009}!x", ["", "A", "\u{2009}!"]),
            ("A !x", ["", "A", ""]),
            ("A\u{3000}!x", ["", "A", ""]),
            ("A(b", ["", "A", ""]),
            ("A-b", ["", "A", ""]),
            ("A.\u{2009}", ["", "A", "."]),
            ("e\u{301}lan", ["", "e\u{301}", ""]),
            ("€5", ["", "€", ""]),
            ("\n   Indented", ["", "I", ""]),
        ] {
            assert_eq!(
                text(source, collapse, ""),
                expected,
                "prefix, letter and suffix of {source:?}"
            );
        }
    }

    #[test]
    fn first_letter_needs_a_letter_number_or_symbol_unit_at_the_line_start() {
        let collapse = WhiteSpaceCollapse::Collapse;
        for source in ["...", "\u{3000}A", "“", "\u{200b}A"] {
            assert_eq!(
                find(source, collapse, ""),
                FirstLetter::None,
                "{source:?} has no first letter"
            );
        }
        assert_eq!(find(" \n\t", collapse, ""), FirstLetter::Pending);
        assert_eq!(find("", collapse, ""), FirstLetter::Pending);
        assert_eq!(
            find("  A", WhiteSpaceCollapse::Preserve, ""),
            FirstLetter::None,
            "preserved leading spaces precede the letter on its line"
        );
        assert_eq!(
            text("“ A", WhiteSpaceCollapse::Preserve, ""),
            ["“ ", "A", ""],
            "a preserved space between the punctuation and the letter joins the prefix"
        );
    }

    #[test]
    fn dutch_first_letter_takes_the_ij_digraph() {
        let collapse = WhiteSpaceCollapse::Collapse;
        assert_eq!(text("IJsselmeer", collapse, "nl-NL"), ["", "IJ", ""]);
        assert_eq!(text("ijs", collapse, "nl"), ["", "ij", ""]);
        assert_eq!(text("ijs", collapse, "en"), ["", "i", ""]);
    }
}
