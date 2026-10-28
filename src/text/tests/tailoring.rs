//! CSS Text 4 §6.1 `word-break` and §6.2 `line-break` soft wrap opportunities.
use super::{paragraph, segments};
use crate::style::{LineBreak, Style, WordBreak};
use crate::text::whitespace::opportunities;
use unicode_linebreak::BreakOpportunity;

fn style(word_break: WordBreak, line_break: LineBreak, language: &str) -> Style {
    let mut style = Style::default();
    style.line_breaking.word_break = word_break;
    style.line_breaking.line_break = line_break;
    style.language = language.into();
    style
}

/// `text` split at its soft wrap opportunities.
fn wraps(text: &str, style: &Style) -> String {
    let paragraph = paragraph(text, style);
    let allowed: Vec<usize> = opportunities(&paragraph, &[])
        .into_iter()
        .filter(|&(_, opportunity)| opportunity == BreakOpportunity::Allowed)
        .map(|(offset, _)| offset)
        .collect();
    segments(&paragraph, &allowed)
}

#[test]
fn break_all_treats_letters_as_ideographs_but_keeps_punctuation_rules() {
    let normal = style(WordBreak::Normal, LineBreak::Auto, "en");
    let all = style(WordBreak::BreakAll, LineBreak::Auto, "en");
    for (text, expected_normal, expected_all) in [
        ("ab cd", "ab |cd", "a|b |c|d"),
        // CP stays glued to the letter before it, but a letter may follow it on a new line.
        ("ab)c", "ab)c", "a|b)|c"),
        ("x12", "x12", "x|1|2"),
        // Conjoining jamo of one syllable never split.
        (
            "\u{1100}\u{1161}\u{11a8}",
            "\u{1100}\u{1161}\u{11a8}",
            "\u{1100}\u{1161}\u{11a8}",
        ),
    ] {
        assert_eq!(
            wraps(text, &normal),
            expected_normal,
            "word-break: normal on {text:?}"
        );
        assert_eq!(
            wraps(text, &all),
            expected_all,
            "word-break: break-all on {text:?}"
        );
    }
}

#[test]
fn keep_all_joins_letters_and_ideographs_except_at_explicit_opportunities() {
    let normal = style(WordBreak::Normal, LineBreak::Auto, "ko");
    let keep = style(WordBreak::KeepAll, LineBreak::Auto, "ko");
    for (text, expected_normal, expected_keep) in [
        ("각 줄의 마지막", "각 |줄|의 |마|지|막", "각 |줄의 |마지막"),
        ("这是一些", "这|是|一|些", "这是一些"),
        (
            "这是\u{200b}一些",
            "这|是\u{200b}|一|些",
            "这是\u{200b}|一些",
        ),
        ("漢字。漢字", "漢|字。|漢|字", "漢字。|漢字"),
    ] {
        assert_eq!(
            wraps(text, &normal),
            expected_normal,
            "word-break: normal on {text:?}"
        );
        assert_eq!(
            wraps(text, &keep),
            expected_keep,
            "word-break: keep-all on {text:?}"
        );
    }
    let anywhere = style(WordBreak::KeepAll, LineBreak::Anywhere, "ko");
    assert_eq!(
        wraps("각줄", &anywhere),
        "각|줄",
        "line-break: anywhere overrides keep-all"
    );
}

#[test]
fn southeast_asian_text_breaks_between_letters_unless_manual() {
    let normal = style(WordBreak::Normal, LineBreak::Auto, "th");
    let manual = style(WordBreak::Manual, LineBreak::Auto, "th");
    let keep = style(WordBreak::KeepAll, LineBreak::Auto, "th");
    // §6.1.1.3: without a lexical resource, letters of SA scripts are separate units; the
    // tone mark U+0E48 stays with its consonant.
    assert_eq!(wraps("ก่อนไป", &normal), "ก่|อ|น|ไ|ป");
    assert_eq!(
        wraps("ก่อนไป", &manual),
        "ก่อนไป",
        "manual: SA letters are AL"
    );
    assert_eq!(wraps("ก่อน ไป", &manual), "ก่อน |ไป");
    assert_eq!(wraps("ก่อนไป", &keep), "ก่อนไป", "keep-all joins SA letters");
}

#[test]
fn line_break_strictness_follows_the_css_text_requirements() {
    // Each pair is split by `line-break: strict`, `normal` and `loose` in Japanese.
    let table = [
        // Small kana and the prolonged sound mark (class CJ): loose only.
        ("あぁ", ["あぁ", "あぁ", "あ|ぁ"]),
        ("アー", ["アー", "アー", "ア|ー"]),
        // Iteration marks: loose only.
        ("あ々", ["あ々", "あ々", "あ|々"]),
        ("あゝ", ["あゝ", "あゝ", "あ|ゝ"]),
        // Inseparable characters: between two of them, loose only.
        ("あ‥‥", ["あ‥‥", "あ‥‥", "あ‥|‥"]),
        // CJK hyphen-like characters: normal and loose.
        ("あ〜", ["あ〜", "あ|〜", "あ|〜"]),
        ("あ゠", ["あ゠", "あ|゠", "あ|゠"]),
        // Hyphens after an ideographic character: loose only.
        ("あ\u{2010}", ["あ\u{2010}", "あ\u{2010}", "あ|\u{2010}"]),
        ("あ\u{2013}", ["あ\u{2013}", "あ\u{2013}", "あ|\u{2013}"]),
        // Centered punctuation: loose only.
        ("あ・", ["あ・", "あ・", "あ|・"]),
        ("あ：", ["あ：", "あ：", "あ|："]),
        ("あ！", ["あ！", "あ！", "あ|！"]),
        // Wide postfixes break before them, wide prefixes after them: loose only.
        ("あ％", ["あ％", "あ％", "あ|％"]),
        ("＄あ", ["＄あ", "＄あ", "＄|あ"]),
        // Opening punctuation still prohibits a break after it.
        ("（ぁ", ["（ぁ", "（ぁ", "（ぁ"]),
    ];
    for (text, expected) in table {
        for (line_break, expected) in [LineBreak::Strict, LineBreak::Normal, LineBreak::Loose]
            .into_iter()
            .zip(expected)
        {
            assert_eq!(
                wraps(text, &style(WordBreak::Normal, line_break, "ja")),
                expected,
                "line-break: {line_break:?} on {text:?} in Japanese"
            );
        }
        assert_eq!(
            wraps(text, &style(WordBreak::Normal, LineBreak::Auto, "ja")),
            wraps(text, &style(WordBreak::Normal, LineBreak::Normal, "ja")),
            "line-break: auto is normal for {text:?}"
        );
    }
    // Rules for Chinese or Japanese writing systems do not apply to other languages.
    for text in ["あ〜", "あ・", "あ％", "＄あ"] {
        assert_eq!(
            wraps(text, &style(WordBreak::Normal, LineBreak::Loose, "en")),
            text,
            "loose {text:?} outside Chinese and Japanese"
        );
    }
    assert_eq!(
        wraps(
            "あ〜",
            &style(WordBreak::Normal, LineBreak::Normal, "zh-Hant")
        ),
        "あ|〜",
        "Chinese shares the CJK hyphen rule"
    );
    // A letter treated as ideographic by break-all allows a loose break before a hyphen.
    assert_eq!(
        wraps(
            "a\u{2010}",
            &style(WordBreak::BreakAll, LineBreak::Loose, "en")
        ),
        "a|\u{2010}"
    );
    assert_eq!(
        wraps(
            "a\u{2010}",
            &style(WordBreak::Normal, LineBreak::Loose, "en")
        ),
        "a\u{2010}"
    );
}

#[test]
fn anywhere_breaks_around_every_character_unit_despite_glue() {
    let anywhere = style(WordBreak::Normal, LineBreak::Anywhere, "en");
    assert_eq!(wraps("ab", &anywhere), "a|b");
    assert_eq!(wraps("a\u{a0}b", &anywhere), "a|\u{a0}|b");
    assert_eq!(
        wraps("a\u{2060}b", &anywhere),
        "a|\u{2060}|b",
        "word joiner"
    );
    assert_eq!(
        wraps("e\u{301}x", &anywhere),
        "e\u{301}|x",
        "a combining mark stays"
    );
}
