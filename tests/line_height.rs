mod common;
use common::{PROFILE, assert_pt, local_options};
use ferri_pdf::render_html;

const FALLBACK_FACES: &str = "@font-face{font-family:Stix;src:url('../assets/STIXGeneral.ttf')}@font-face{font-family:First;src:url('../assets/DejaVuSans.ttf')}";

#[test]
fn explicit_line_height_uses_the_first_font_for_fallback_layout_bounds() {
    const LINE_HEIGHT: f32 = 14.0;
    const STIX_BASELINE: f32 = 10.0;
    for line_height in ["14pt", "1.4", "140%"] {
        for content in [
            "A العربية B",
            "العربية",
            "A עברית B",
            "<span>العربية</span>",
        ] {
            let html = format!(
                "<!doctype html><style>{PROFILE}{FALLBACK_FACES}@page{{size:240pt 100pt;margin:10pt}}body{{font:10pt/{line_height} Stix,Spike}}p{{margin:0}}</style><p>{content}<br>{content}<br>{content}</p>"
            );
            let trace = render_html(&html, &local_options())
                .unwrap()
                .layout
                .unwrap();
            assert_eq!(trace.pages.len(), 1, "{line_height}, {content}");
            assert_eq!(trace.pages[0].len(), 3, "{line_height}, {content}");
            for (index, line) in trace.pages[0].iter().enumerate() {
                assert_pt(line.height, LINE_HEIGHT);
                assert_pt(
                    line.baseline,
                    10.0 + STIX_BASELINE + index as f32 * LINE_HEIGHT,
                );
                assert!(
                    line.runs
                        .iter()
                        .any(|run| run.text.contains('ع') || run.text.contains('ע')),
                    "fallback text must survive: {line_height}, {content}"
                );
            }
        }
    }
}

#[test]
fn normal_line_height_still_includes_fallback_font_extents() {
    const STIX_ASCENT: f32 = 10.55;
    const STIX_HEIGHT: f32 = 15.1;
    let html = format!(
        "<!doctype html><style>{PROFILE}{FALLBACK_FACES}@page{{size:240pt 100pt;margin:10pt}}body{{font:10pt/normal First,Stix}}p{{margin:0}}</style><p>A ⅊<br>A ⅊</p>"
    );
    let trace = render_html(&html, &local_options())
        .unwrap()
        .layout
        .unwrap();
    assert_eq!(trace.pages[0].len(), 2);
    for (index, line) in trace.pages[0].iter().enumerate() {
        assert_pt(line.height, STIX_HEIGHT);
        assert_pt(
            line.baseline,
            10.0 + STIX_ASCENT + index as f32 * STIX_HEIGHT,
        );
        assert_eq!(
            line.runs
                .iter()
                .map(|run| run.font_id)
                .collect::<std::collections::HashSet<_>>()
                .len(),
            2,
            "normal line metrics must include the actual fallback face"
        );
    }
}

#[test]
fn initial_line_height_uses_font_metrics_instead_of_a_numeric_multiplier() {
    const NATIVE_NORMAL_HEIGHT: f32 = 27.9375;
    const TOLERANCE_PT: f32 = 0.0001;
    let html = format!(
        "<style>{PROFILE}p{{font-size:24pt;line-height:initial}}</style><p>One<br>Two<br>Three</p>"
    );
    let trace = render_html(&html, &local_options())
        .unwrap()
        .layout
        .unwrap();
    for pair in trace.pages[0].windows(2) {
        let advance = pair[1].baseline - pair[0].baseline;
        assert!(
            (advance - NATIVE_NORMAL_HEIGHT).abs() < TOLERANCE_PT,
            "native normal leading {NATIVE_NORMAL_HEIGHT}pt, got {advance}pt"
        );
    }
}

#[test]
fn percentage_font_size_uses_parent_size_before_resolving_em_spacing() {
    let html = format!(
        "<style>{PROFILE}section{{font-size:10pt}}p{{font-size:150%;margin:0}}span{{font-size:200%;letter-spacing:0.1em}}h1{{font-size:200%;margin:0}}</style><section><p>Parent <span>Child <b>Inherited</b></span></p><h1>Heading</h1></section>"
    );
    let trace = render_html(&html, &local_options())
        .expect("percentage font sizes must render")
        .layout
        .unwrap();
    let sizes: Vec<_> = trace.pages[0]
        .iter()
        .flat_map(|line| line.runs.iter().map(|run| run.font_size))
        .collect();
    assert!(
        sizes.contains(&15.0),
        "paragraph is 150% of 10pt: {sizes:?}"
    );
    assert!(sizes.contains(&30.0), "span is 200% of 15pt: {sizes:?}");
    let child = trace.pages[0]
        .iter()
        .flat_map(|line| &line.runs)
        .find(|run| run.text.contains("Child"))
        .unwrap();
    let tracking = child.glyphs()[0].x_spacing;
    assert!(
        (tracking - 3.0).abs() < 0.0001,
        "0.1em must resolve against the child's computed 30pt size: {tracking}"
    );
    let inherited = trace.pages[0]
        .iter()
        .flat_map(|line| &line.runs)
        .find(|run| run.text.contains("Inherited"))
        .unwrap();
    assert_eq!(
        inherited.font_size, 30.0,
        "inheritance must not reapply the 200% multiplier"
    );
    assert_eq!(
        trace.pages[0].last().unwrap().font_size,
        20.0,
        "heading percentage uses its parent's size, not its UA heading multiplier"
    );
}

#[test]
fn percentage_line_height_inherits_the_computed_length() {
    for (specified, expected) in [("150%", 30.0), ("1.5", 15.0)] {
        let html = format!(
            "<style>{PROFILE}section{{font-size:20pt;line-height:{specified}}}p{{font-size:10pt;margin:0}}</style><section><p>First<br>Second<br>Third</p></section>"
        );
        let trace = render_html(&html, &local_options())
            .expect("percentage line height must render")
            .layout
            .unwrap();
        for pair in trace.pages[0].windows(2) {
            let actual = pair[1].baseline - pair[0].baseline;
            assert!(
                (actual - expected).abs() < 0.001,
                "{specified} inherited by a 10pt child: expected {expected}pt, got {actual}pt"
            );
        }
    }
}

#[test]
fn overflowing_percentage_typography_is_invalid_at_computed_value_time() {
    for (declaration, reported, reason) in [
        (
            "font-size:10000%",
            "font-size: 10000%",
            "non-finite font size",
        ),
        (
            "line-height:10000%",
            "line-height: 10000%",
            "non-finite line height",
        ),
    ] {
        // The paragraph has no text, so the inherited 1e38pt size lays out no line.
        let html = format!(
            "<style>{PROFILE}section{{font-size:1e38pt}}p{{{declaration}}}</style><section><p></p></section><div>After</div>"
        );
        let output = render_html(&html, &local_options())
            .unwrap_or_else(|error| panic!("overflowing {declaration} must not fail: {error}"));
        assert!(
            output.diagnostics.iter().any(|diagnostic| matches!(
                diagnostic,
                ferri_pdf::RenderDiagnostic::Degraded(
                    ferri_pdf::Degradation::InvalidAtComputedValueTime { declaration, reason: found }
                ) if declaration == reported && found == reason
            )),
            "overflowing {declaration} is discarded during CSS computation: {:?}",
            output.diagnostics
        );
        assert_eq!(
            output.layout.unwrap().pages[0][0].text,
            "After",
            "the rest of the document renders"
        );
    }
}
