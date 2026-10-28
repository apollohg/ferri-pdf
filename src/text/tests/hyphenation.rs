//! CSS Text 4 §6.3 automatic hyphenation opportunities.
use super::{paragraph, segments};
use crate::style::{HyphenateLimitChars, Hyphens, Style, WordBreak};
use crate::text::hyphenation::automatic;

fn style(language: &str) -> Style {
    let mut style = Style::default();
    style.hyphens = Hyphens::Auto;
    style.language = language.into();
    style
}

fn hyphenated(text: &str, style: &Style) -> String {
    let paragraph = paragraph(text, style);
    segments(&paragraph, &automatic(&paragraph))
}

#[test]
fn english_and_german_patterns_hyphenate_by_language() {
    assert_eq!(
        hyphenated("representatives", &style("en")),
        "rep|re|sen|ta|tives"
    );
    assert_eq!(
        hyphenated("extraordinarily uncomfortable", &style("en-GB")),
        "ex|tra|or|di|nar|ily un|com|fort|able",
        "region subtags share the language's patterns"
    );
    assert_eq!(
        hyphenated("Donaudampfschifffahrtsgesellschaft", &style("de")),
        "Do|nau|dampf|schiff|fahrts|ge|sell|schaft"
    );
    assert_eq!(
        hyphenated("Rechtsschutzversicherung", &style("de-AT")),
        "Rechts|schutz|ver|si|che|rung"
    );
}

#[test]
fn languages_without_patterns_and_explicit_hyphens_keep_their_manual_behaviour() {
    for language in ["", "xx", "nl", "ja"] {
        assert_eq!(
            hyphenated("representatives", &style(language)),
            "representatives",
            "no resource for {language:?}"
        );
    }
    assert_eq!(
        hyphenated("repre\u{ad}sentatives representatives", &style("en")),
        "repre\u{ad}sentatives rep|re|sen|ta|tives",
        "a conditional hyphen disables automatic opportunities in its word only"
    );
    let mut manual = style("en");
    manual.hyphens = Hyphens::Manual;
    assert_eq!(hyphenated("representatives", &manual), "representatives");
    let mut phrases = style("en");
    phrases.line_breaking.word_break = WordBreak::AutoPhrase;
    assert_eq!(hyphenated("representatives", &phrases), "representatives");
    let mut all = style("en");
    all.line_breaking.word_break = WordBreak::BreakAll;
    assert_eq!(
        hyphenated("representatives", &all),
        "representatives",
        "break-all applies no automatic hyphenation"
    );
}

#[test]
fn limit_chars_bound_the_word_and_both_sides_of_each_opportunity() {
    let mut limited = style("en");
    limited.line_breaking.hyphenate_limit_chars = HyphenateLimitChars {
        word: Some(8),
        before: Some(4),
        after: Some(4),
    };
    assert_eq!(
        hyphenated("representatives", &limited),
        "repre|sen|ta|tives"
    );
    limited.line_breaking.hyphenate_limit_chars.word = Some(16);
    assert_eq!(
        hyphenated("representatives", &limited),
        "representatives",
        "fifteen letters are fewer than the word minimum"
    );
    let mut short = style("en");
    assert_eq!(hyphenated("paper", &short), "pa|per");
    short.line_breaking.hyphenate_limit_chars.word = Some(6);
    assert_eq!(hyphenated("paper", &short), "paper");
    short.line_breaking.hyphenate_limit_chars = HyphenateLimitChars {
        word: None,
        before: Some(3),
        after: None,
    };
    assert_eq!(
        hyphenated("paper", &short),
        "paper",
        "three characters must precede the hyphen, and \"pa\" has two"
    );
}
