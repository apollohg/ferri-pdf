//! Which typographic character units take emphasis marks (CSS Text Decoration 3 §3.1).
use crate::text::emphasis::marked;

#[test]
fn marks_skip_separators_punctuation_and_controls_but_not_listed_symbols() {
    let cases = [
        ("a", true, "a letter"),
        ("漢", true, "an ideograph"),
        ("7", true, "a digit"),
        ("+", true, "a math symbol is not punctuation"),
        ("#", true, "a listed punctuation symbol"),
        ("\u{FF03}", true, "fullwidth number sign normalizes to #"),
        ("\u{FE5F}", true, "small number sign normalizes to #"),
        ("\u{2030}", true, "per mille sign is listed"),
        ("\u{303D}", true, "part alternation mark is listed"),
        (",", false, "a comma is punctuation"),
        ("。", false, "an ideographic full stop is punctuation"),
        ("「", false, "an opening bracket is punctuation"),
        (
            "\u{FE45}",
            false,
            "a sesame dot is punctuation that normalizes to itself",
        ),
        (" ", false, "a space is a separator"),
        ("\u{3000}", false, "an ideographic space is a separator"),
        ("\u{2028}", false, "a line separator"),
        (" \u{0301}", true, "a space carrying a combining mark"),
        ("\u{0007}", false, "a control"),
        ("\u{200B}", false, "zero width space is a format character"),
        ("\u{0378}", false, "an unassigned code point"),
        (
            "e\u{0301}",
            true,
            "a letter with a combining mark is one unit",
        ),
    ];
    for (cluster, expected, reason) in cases {
        assert_eq!(
            marked(cluster),
            expected,
            "{cluster:?} ({:?}): {reason}",
            cluster.chars().map(u32::from).collect::<Vec<_>>()
        );
    }
}
