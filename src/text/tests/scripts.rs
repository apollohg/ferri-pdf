//! Splitting text into the runs of one script that shape apart.
use super::{document, first_paragraph};
use crate::resources::FileResolver;
use crate::text::{Scripts, TextContext};
use crate::work::{self, Work};
use std::ops::Range;
use unicode_script::{Script, UnicodeScript};

/// The parts of `range` of `text` that shape apart, found by scanning the range alone: a part
/// starts at each character whose script differs from the script of the character before it,
/// Common and Inherited characters aside.
fn scanned(text: &str, range: Range<usize>) -> Vec<Range<usize>> {
    let mut parts = Vec::new();
    let mut start = range.start;
    let mut current = None;
    for (offset, ch) in text[range.clone()].char_indices() {
        let script = ch.script();
        if matches!(script, Script::Common | Script::Inherited) {
            continue;
        }
        if current.is_some_and(|current| current != script) {
            parts.push(start..range.start + offset);
            start = range.start + offset;
        }
        current = Some(script);
    }
    parts.push(start..range.end);
    parts
}

#[test]
fn any_range_splits_where_its_own_characters_change_script() {
    // Stretches found once for the whole text must split every range of it as scanning the
    // range alone does, including ranges that start or end inside a stretch or between
    // stretches, and ranges of Common characters only.
    let text = "1. Grand total: 漢字かな (Ελληνικά) and ‘עברית’ 42, \u{301}مرحبا end";
    let scripts = Scripts::of(text);
    let offsets: Vec<usize> = text
        .char_indices()
        .map(|(offset, _)| offset)
        .chain([text.len()])
        .collect();
    for (index, &start) in offsets.iter().enumerate() {
        for &end in &offsets[index..] {
            assert_eq!(
                scripts.ranges(start..end),
                scanned(text, start..end),
                "the parts of {:?}",
                &text[start..end]
            );
        }
    }
}

#[test]
fn candidate_lines_look_up_each_character_script_once() {
    // Each candidate end of a line splits the line from its start into scripts; looking the
    // scripts up afresh for each candidate takes work quadratic in a line's words.
    let lookups = |words: usize| {
        let body = format!("<p>{}</p>", vec!["word"; words].join(" "));
        let document = document(&body);
        let paragraph = first_paragraph(document.root()).unwrap();
        let resolver = FileResolver::default();
        let context = TextContext::new(&document.styles().fonts, &resolver);
        let (lines, lookups) = work::measured(Work::ScriptCharacter, || {
            context.layout_paragraph(paragraph, 1.0e6, &[]).unwrap()
        });
        assert_eq!(lines.len(), 1, "{words} words fit one line");
        lookups
    };
    work::assert_linear("script lookups for one line of words", 128, lookups);
}
