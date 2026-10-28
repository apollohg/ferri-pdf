use super::common;
use crate::render_html;
use common::{PROFILE, local_options};

#[test]
fn emergency_wrap_preserves_words_when_they_fit_and_never_splits_combining_sequences() {
    let html = format!(
        "<style>{PROFILE}p{{width:60pt;overflow-wrap:anywhere}}</style><p>one two abcdefghijklmnopqrstuvwxyz tail</p><p>éééééééééé</p>"
    );
    let trace = render_html(&html, &local_options())
        .unwrap()
        .layout
        .unwrap();
    let lines = &trace.pages[0];
    assert_eq!(lines[0].text, "one two");
    assert!(lines.iter().all(|line| line.width <= 60.0));
    assert_eq!(
        lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<String>()
            .replace(' ', ""),
        "onetwoabcdefghijklmnopqrstuvwxyztail".to_owned() + &"e\u{301}".repeat(10)
    );
    assert!(lines.iter().all(|line| !line.text.starts_with('\u{301}')));
    let fit = format!(
        "<style>{PROFILE}p{{width:60pt;overflow-wrap:anywhere}}</style><p>one abcdefgh</p>"
    );
    let trace = render_html(&fit, &local_options()).unwrap().layout.unwrap();
    assert_eq!(
        trace.pages[0]
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>(),
        ["one", "abcdefgh"]
    );
}

#[test]
fn anywhere_changes_table_min_content_but_break_word_does_not() {
    for (value, wraps) in [("anywhere", true), ("break-word", false), ("normal", false)] {
        let html = format!(
            "<style>{PROFILE}table{{width:36pt;border-spacing:0;overflow-wrap:{value}}}td{{padding:0}}</style><table><tr><td>abcdefghijklmno</td></tr></table>"
        );
        let trace = render_html(&html, &local_options())
            .unwrap()
            .layout
            .unwrap();
        assert_eq!(trace.pages[0].len() > 1, wraps, "min-content for {value}");
        if wraps {
            assert!(trace.pages[0].iter().all(|line| line.width <= 36.0));
        }
    }
}

#[test]
fn legacy_alias_cascades_with_overflow_wrap_and_nowrap_disables_emergency_breaks() {
    let html = format!(
        "<style>{PROFILE}p{{width:24pt;word-wrap:break-word;overflow-wrap:normal;word-wrap:anywhere}}p:last-child{{white-space:nowrap}}</style><p>abcdefgh</p><p>abcdefgh</p>"
    );
    let trace = render_html(&html, &local_options())
        .unwrap()
        .layout
        .unwrap();
    assert!(trace.pages[0].len() > 2);
    assert_eq!(trace.pages[0].last().unwrap().text, "abcdefgh");
    assert!(trace.pages[0].last().unwrap().width > 24.0);
}

#[test]
fn oversized_graphemes_make_progress_without_inventing_blank_lines() {
    let html = format!(
        "<style>{PROFILE}p{{width:1pt;overflow-wrap:anywhere}}</style><p>W<br>W</p><p>éé</p>"
    );
    let trace = render_html(&html, &local_options())
        .unwrap()
        .layout
        .unwrap();
    assert_eq!(
        trace.pages[0]
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>(),
        ["W", "W", "e\u{301}", "e\u{301}"]
    );
}

#[test]
fn negative_tracking_does_not_assume_prefix_widths_increase() {
    let html = format!(
        "<style>{PROFILE}p{{width:8pt;overflow-wrap:anywhere}}span{{letter-spacing:-10pt}}</style><p>WW<span>iii</span>WW</p>"
    );
    let trace = render_html(&html, &local_options())
        .unwrap()
        .layout
        .unwrap();
    assert_eq!(trace.pages[0][0].text, "WWiii");
    assert!(trace.pages[0][0].width <= 8.0);
}

#[test]
fn emergency_breaks_preserve_arabic_joining_forms() {
    let html = format!(
        "<style>{PROFILE}p{{width:6pt;overflow-wrap:anywhere}}</style><p dir='rtl'>سسسس</p>"
    );
    let wrapped = render_html(&html, &local_options())
        .unwrap()
        .layout
        .unwrap();
    let full = render_html(&html.replace("width:6pt", "width:200pt"), &local_options())
        .unwrap()
        .layout
        .unwrap();
    assert_eq!(wrapped.pages[0].len(), 4);
    let ids = |lines: &[ferri_pdf_layout::TextLine]| {
        lines
            .iter()
            .flat_map(|line| &line.runs)
            .flat_map(|run| run.glyphs().iter().map(|glyph| glyph.id))
            .collect::<Vec<_>>()
    };
    let mut expected = ids(&full.pages[0]);
    expected.reverse();
    assert_eq!(
        ids(&wrapped.pages[0]),
        expected,
        "each emergency line retains its original initial, medial or final joining form"
    );
}
