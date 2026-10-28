mod common;
use common::{PROFILE, local_options};
use ferri_pdf::render_html;

#[test]
fn case_transforms_preserve_expansions_cascade_and_inline_styles() {
    let html = format!(
        r#"<style>{PROFILE}
p {{text-transform:uppercase}} span {{color:red}}
p::before {{content:"ß ";text-transform:lowercase}}
</style><p>straße <span>ﬁancée</span> <i style="text-transform:none">Mixed</i></p>
<p style="text-transform:lowercase">ΟΣ <span>ΟΣ</span>Α</p>
<p style="text-transform:capitalize">h<span>ELLO</span> wORLD ǳuro</p>"#
    );
    let trace = render_html(&html, &local_options())
        .unwrap()
        .layout
        .unwrap();
    let text: Vec<_> = trace.pages[0].iter().map(|l| l.text.as_str()).collect();
    assert_eq!(
        text,
        ["ß STRASSE FIANCÉE Mixed", "ß ος οσα", "ß HELLO WORLD ǲuro"]
    );
    assert!(
        trace.pages[0][0]
            .runs
            .iter()
            .any(|run| run.text.contains("FIANCÉE") && run.color.rgb == [255, 0, 0])
    );
}

#[test]
fn inherited_language_controls_contextual_case_mapping_across_spans() {
    let html = format!(
        r#"<style>{PROFILE}p {{margin:0}}</style>
<section lang="tr"><p style="text-transform:uppercase">istanbul ı <span>i</span></p>
<p style="text-transform:lowercase">I<span>̇</span> I İ</p>
<p lang="" style="text-transform:uppercase">i</p></section>
<p lang="lt" style="text-transform:lowercase">I<span>́</span></p>
<p lang="nl" style="text-transform:capitalize">i<span>j</span>ssel</p>
<p lang="el" style="text-transform:uppercase">άι άυ ή</p>"#
    );
    let trace = render_html(&html, &local_options())
        .unwrap()
        .layout
        .unwrap();
    let text: Vec<_> = trace.pages[0].iter().map(|l| l.text.as_str()).collect();
    assert_eq!(
        text,
        ["İSTANBUL I İ", "i ı i", "I", "i̇́", "IJssel", "ΑΪ ΑΫ Ή"]
    );
}

#[test]
fn casing_applies_to_markers_and_css_wide_keywords_before_wrapping() {
    let html = format!(
        r#"<style>{PROFILE}section {{text-transform:uppercase}} ol {{margin:0;list-style-type:lower-alpha}} li + li::marker {{text-transform:inherit}}</style>
<section><p>one <span style="text-transform:initial">Two</span> <span style="text-transform:unset">three</span></p>
<ol><li>word</li><li style="list-style-position:inside">word</li></ol></section>"#
    );
    let trace = render_html(&html, &local_options())
        .unwrap()
        .layout
        .unwrap();
    let text: Vec<_> = trace.pages[0]
        .iter()
        .map(|line| line.text.as_str())
        .collect();
    assert_eq!(
        text,
        ["ONE Two THREE", "a. ", "WORD", "B. WORD"],
        "the UA `::marker {{ text-transform: none }}` (CSS Lists 3 §3.1.1) yields to author rules"
    );
    let source = format!(
        "<style>{PROFILE}p{{width:80pt;text-transform:uppercase}}</style><p>straße fiancée straße fiancée</p>"
    );
    let expected = source
        .replace("text-transform:uppercase", "text-transform:none")
        .replace("straße fiancée", "STRASSE FIANCÉE");
    let actual = render_html(&source, &local_options())
        .unwrap()
        .layout
        .unwrap();
    let expected = render_html(&expected, &local_options())
        .unwrap()
        .layout
        .unwrap();
    assert_eq!(actual.pages.len(), expected.pages.len());
    for (actual, expected) in actual.pages.iter().zip(&expected.pages) {
        assert_eq!(actual.len(), expected.len());
        for (actual, expected) in actual.iter().zip(expected) {
            assert_eq!(
                (&actual.text, actual.width, actual.x, actual.baseline),
                (
                    &expected.text,
                    expected.width,
                    expected.x,
                    expected.baseline
                ),
                "casing must precede font selection, measurement and wrapping"
            );
        }
    }
}

#[test]
fn capitalize_handles_combining_letter_units_without_lowering_capital_digraphs() {
    let html = format!(
        "<style>{PROFILE}p{{text-transform:capitalize}}</style><p lang='lt'>i\u{307}<span>\u{301}</span></p><p>ǱURO ǄURO ǳuro</p>"
    );
    let trace = render_html(&html, &local_options())
        .unwrap()
        .layout
        .unwrap();
    let text: Vec<_> = trace.pages[0]
        .iter()
        .map(|line| line.text.as_str())
        .collect();
    assert_eq!(text, ["I\u{301}", "ǱURO ǄURO ǲuro"]);
}

#[test]
fn long_greek_combining_runs_are_removed_without_repeated_prefix_scans() {
    const COMBINING_MARKS: usize = 64 * 1024;
    let html = format!(
        "<style>{PROFILE}p{{text-transform:uppercase}}</style><p lang='el'>α{}</p>",
        "\u{301}".repeat(COMBINING_MARKS)
    );
    let trace = render_html(&html, &local_options())
        .unwrap()
        .layout
        .unwrap();
    assert_eq!(trace.pages[0].len(), 1);
    assert_eq!(trace.pages[0][0].text, "Α");
}

#[test]
fn invalid_transform_combinations_do_not_replace_valid_cascade_values() {
    let html = format!(
        "<style>{PROFILE}p{{text-transform:uppercase;text-transform:uppercase lowercase}}</style><p>mixed</p>"
    );
    let trace = render_html(&html, &local_options())
        .unwrap()
        .layout
        .unwrap();
    assert_eq!(trace.pages[0][0].text, "MIXED");
    let supported = format!(
        "<style>{PROFILE}@supports(text-transform:uppercase full-width){{p{{color:red}}}}</style><p>Supported</p>"
    );
    let output = render_html(&supported, &local_options())
        .unwrap()
        .layout
        .unwrap();
    assert_eq!(output.pages[0][0].color.rgb, [255, 0, 0]);
}
