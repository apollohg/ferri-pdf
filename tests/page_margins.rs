mod common;
use common::{PROFILE, local_options};
use ferri_pdf::render_html;

#[test]
fn page_margin_counters_repeat_without_changing_body_pagination() {
    let html = format!(
        r#"<style>{PROFILE}
        html {{font-family:Spike;font-size:12pt;line-height:18pt}}
        @page {{
            @top-left {{content:"Left"}}
            @top-center {{content:"Centre"}}
            @top-right {{content:"Right"}}
            @bottom-left {{content:"Page " counter(page) " of " counter(pages)}}
            @bottom-right {{content:"Footer"}}
        }}
        </style><p>Body one</p><p class="new-page">Body two</p>"#
    );
    let trace = render_html(&html, &local_options())
        .unwrap()
        .layout
        .unwrap();
    assert_eq!(
        trace.pages.len(),
        2,
        "margin content must not consume body flow space"
    );
    const TOP_BASELINE_PT: f32 = 22.1543;
    const BOTTOM_BASELINE_PT: f32 = 466.1543;
    const BODY_BASELINE_PT: f32 = 49.1543;
    for (index, page) in trace.pages.iter().enumerate() {
        for (text, baseline) in [
            ("Left".to_owned(), TOP_BASELINE_PT),
            ("Centre".to_owned(), TOP_BASELINE_PT),
            ("Right".to_owned(), TOP_BASELINE_PT),
            (format!("Page {} of 2", index + 1), BOTTOM_BASELINE_PT),
            ("Footer".to_owned(), BOTTOM_BASELINE_PT),
            (
                if index == 0 { "Body one" } else { "Body two" }.to_owned(),
                BODY_BASELINE_PT,
            ),
        ] {
            let line = page
                .iter()
                .find(|line| line.text == text)
                .unwrap_or_else(|| panic!("page {} missing {text}", index + 1));
            assert!(
                (line.baseline - baseline).abs() < 0.001,
                "{text}: baseline {} expected {baseline}",
                line.baseline
            );
        }
        let left = page.iter().find(|line| line.text == "Left").unwrap();
        let centre = page.iter().find(|line| line.text == "Centre").unwrap();
        let right = page.iter().find(|line| line.text == "Right").unwrap();
        assert_eq!(left.x, 36.0);
        assert!((centre.x - 159.9375).abs() < 0.001);
        assert!((right.x - 292.4004).abs() < 0.001);
    }
}

#[test]
fn later_none_removes_generated_boxes_without_evaluating_their_fonts() {
    let html = format!(
        r#"<style>{PROFILE}
      @page {{@bottom-left {{content:"Removed";font-family:Absent}}}}
      @page {{@bottom-left {{content:none}} @bottom-right {{content:"Kept"!important;font-family:Spike}}}}
      @page {{@bottom-right {{content:none}}}}
      </style><p>Body</p>"#
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
        ["Body", "Kept"]
    );
}

#[test]
fn page_margin_overflow_keeps_only_fitting_lines_and_centres_retained_content() {
    let html = format!(
        r#"<style>{PROFILE}html{{font-family:Spike;font-size:12pt;line-height:18pt}}
      @page {{@top-left{{content:"First\a Second\a Third";white-space:pre;padding:2pt}}
             @bottom-left{{content:"Left footer words";width:80pt;border:1pt solid black}}
             @bottom-center{{content:"Middle footer words";padding:2pt}}
             @bottom-right{{content:"Right footer words"}}}}
      </style><p>Body</p>"#
    );
    let trace = render_html(&html, &local_options())
        .unwrap()
        .layout
        .unwrap();
    let page = &trace.pages[0];
    assert!(
        !page
            .iter()
            .any(|line| line.text.contains("Second") || line.text.contains("Third")),
        "overflow lines must not leak into the body"
    );
    let first = page.iter().find(|line| line.text == "First").unwrap();
    assert!(
        (first.baseline - 22.1543).abs() < 0.001,
        "the retained line, not all source lines, is vertically centred"
    );
    let middle = page
        .iter()
        .find(|line| line.text == "Middle footer words")
        .expect("fixed side width must leave a wide enough middle box for one line");
    assert!((middle.x - 120.5273).abs() < 0.001);
    assert!((middle.baseline - 466.1543).abs() < 0.001);
}

#[test]
fn fixed_margin_dimensions_fill_the_margin_and_middle_boxes_force_symmetric_sides() {
    let html = format!(
        r#"<style>{PROFILE}html{{font-family:Spike;font-size:12pt;line-height:18pt}}
      @page {{
        @top-left{{content:"L";width:20pt;height:1pt;background:red}}
        @top-center{{content:"C";width:60pt;background:blue}}
        @top-right{{content:"R";width:100pt;background:red}}
        @bottom-left-corner{{content:"X";width:10pt;height:10pt;margin:2pt 3pt 4pt 5pt;background:blue}}
      }}</style><p>Body</p>"#
    );
    let trace = render_html(&html, &local_options())
        .unwrap()
        .layout
        .unwrap();
    let rectangles: Vec<_> = trace.display_lists[0]
        .iter()
        .filter_map(|item| match item {
            ferri_pdf::DisplayItem::Box(paint) => Some(paint.rect),
            _ => None,
        })
        .collect();
    for expected in [
        [36.0, 0.0, 114.0, 36.0],
        [150.0, 0.0, 60.0, 36.0],
        [210.0, 0.0, 114.0, 36.0],
        [5.0, 446.0, 28.0, 30.0],
    ] {
        assert!(
            rectangles
                .iter()
                .any(|rect| [rect.x, rect.y, rect.width, rect.height] == expected),
            "missing native margin rectangle {expected:?}; got {rectangles:?}"
        );
    }
}

#[test]
fn nonfinite_page_margin_geometry_is_rejected_even_without_text() {
    let html = format!(
        r#"<style>{PROFILE}@page{{@top-left{{content:"";background:red;padding:1e38%}}}}</style><p>Body</p>"#
    );
    let error = render_html(&html, &local_options())
        .err()
        .expect("overflowing margin geometry must not reach the PDF writer");
    assert!(
        error
            .to_string()
            .contains("non-finite page-margin geometry"),
        "unexpected error: {error}"
    );
}

#[test]
fn page_box_and_footnote_area_reject_effects_they_cannot_paint() {
    for (rule, expected) in [
        ("@page{opacity:.5}", "opacity in @page"),
        ("@page{filter:blur(1pt)}", "filter in @page"),
        ("@page{mix-blend-mode:multiply}", "mix-blend-mode in @page"),
        ("@page{clip-path:inset(10%)}", "clip-path in @page"),
        (
            "@page{mask-image:linear-gradient(red,red)}",
            "mask-image in @page",
        ),
        ("@page{rotate:10deg}", "rotate in @page"),
        (
            "@page{@footnote{transform:scale(2)}}",
            "transform in @footnote",
        ),
        (
            "@page{@footnote{perspective:100pt}}",
            "perspective in @footnote",
        ),
    ] {
        let html = format!("<style>{PROFILE}{rule}</style><p>Body</p>");
        let error = render_html(&html, &local_options())
            .err()
            .unwrap_or_else(|| panic!("{rule} must not be accepted and ignored"));
        assert!(
            error.to_string().contains(expected),
            "{rule}: expected an unsupported {expected:?} error, got {error}"
        );
    }
}

#[test]
fn page_counters_share_predefined_styles_without_marker_punctuation() {
    const PAGE_COUNT: usize = 28;
    let body = (0..PAGE_COUNT).map(|_| "<p>Body</p>").collect::<String>();
    let html = format!(
        r"<style>{PROFILE}
      p + p {{break-before:page}}
      @page {{
        @top-left {{content:counter(page,lower-roman) '|' counter(page,lower-alpha) '|' counter(page,lower-greek) '|' counter(page,decimal-leading-zero)}}
        @bottom-center {{content:counter(pages,upper-roman)}}
      }}
      </style>{body}"
    );
    let trace = render_html(&html, &local_options())
        .expect("predefined page counter styles")
        .layout
        .unwrap();
    assert_eq!(trace.pages.len(), PAGE_COUNT);
    for (number, expected) in [
        (1, "i|a|α|01"),
        (24, "xxiv|x|ω|24"),
        (25, "xxv|y|αα|25"),
        (26, "xxvi|z|αβ|26"),
        (27, "xxvii|aa|αγ|27"),
        (28, "xxviii|ab|αδ|28"),
    ] {
        let text = trace.pages[number - 1]
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>();
        assert!(
            text.contains(&expected),
            "page {number}: expected {expected:?}, got {text:?}"
        );
        assert!(
            text.contains(&"XXVIII"),
            "total pages must use its own counter style: {text:?}"
        );
    }
    let symbols = render_html(&format!(r"<style>{PROFILE}
      @page {{@top-left {{content:'[' counter(page,disc) counter(page,circle) counter(page,square) counter(page,none) ']'}}
               @bottom-center {{content:counter(page,upper-latin) '/' counter(pages,lower-latin)}}}}
      </style><p>Body</p>"), &local_options()).unwrap().layout.unwrap();
    let text = symbols.pages[0]
        .iter()
        .map(|line| line.text.as_str())
        .collect::<Vec<_>>();
    assert!(
        text.contains(&"[•◦▪]"),
        "symbol styles and none must add no list suffix: {text:?}"
    );
    assert!(
        text.contains(&"A/a"),
        "Latin aliases must share alphabetic formatting: {text:?}"
    );
}

#[test]
fn page_margin_text_box_trim_matches_a_trimmed_block_in_the_page_area() {
    const MARGIN_PT: f32 = 60.0;
    const FONT: &str = "font:20pt/40pt Spike";
    let baselines = |trim: &str| {
        let html = format!(
            "<style>{PROFILE}body{{margin:0}}@page{{margin:{MARGIN_PT}pt;\
             @top-left{{content:'Margin';vertical-align:top;{FONT};{trim}}}}}\
             div{{{FONT};{trim}}}</style><div>Body</div>"
        );
        let trace = render_html(&html, &local_options())
            .unwrap()
            .layout
            .unwrap();
        let baseline = |text: &str| {
            trace.pages[0]
                .iter()
                .find(|line| line.text == text)
                .unwrap_or_else(|| panic!("{text} line: {:?}", trace.pages[0]))
                .baseline
        };
        (baseline("Margin"), baseline("Body") - MARGIN_PT)
    };
    let (untrimmed_margin, untrimmed_body) = baselines("");
    let (trimmed_margin, trimmed_body) = baselines("text-box:trim-both cap alphabetic");
    assert!(
        trimmed_body < untrimmed_body,
        "the block's trim raises its first line: {untrimmed_body} -> {trimmed_body}"
    );
    common::assert_labelled_pt(
        "untrimmed margin baseline",
        untrimmed_margin,
        untrimmed_body,
    );
    common::assert_labelled_pt("trimmed margin baseline", trimmed_margin, trimmed_body);
}
