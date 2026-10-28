mod common;

use common::{assert_pt, local_options};
use ferri_pdf::{LayoutTrace, render_html};

const LEFT: f32 = 36.0;
const WIDTH: f32 = 100.0;

fn render(css: &str, body: &str) -> LayoutTrace {
    render_html(
        &format!("<style>{}p,div{{margin:0;padding:0;width:100pt;font:10pt/12pt Spike}}{css}</style>{body}", common::PROFILE),
        &local_options(),
    ).unwrap_or_else(|error| panic!("indent fixture must render: {error}; CSS: {css}; HTML: {body}"))
        .layout.unwrap()
}

#[test]
fn length_percentage_negative_and_alignment_use_the_logical_start_margin() {
    for (value, indent) in [
        ("20pt", 20.0),
        ("20%", 20.0),
        ("-10pt", -10.0),
        ("calc(10% + 5pt)", 15.0),
    ] {
        for rtl in [false, true] {
            for align in ["start", "end", "center", "left", "right"] {
                let trace = render(
                    &format!(
                        "p{{text-indent:{value};direction:{};text-align:{align}}}",
                        if rtl { "rtl" } else { "ltr" }
                    ),
                    "<p>Alpha<br>Beta</p>",
                );
                for (index, line) in trace.pages[0].iter().enumerate() {
                    let margin = if index == 0 { indent } else { 0.0 };
                    let available = WIDTH - margin;
                    let offset = match align {
                        "right" => available - line.width,
                        "start" if rtl => available - line.width,
                        "end" if !rtl => available - line.width,
                        "center" => (available - line.width) / 2.0,
                        _ => 0.0,
                    };
                    assert_pt(line.x, LEFT + if rtl { 0.0 } else { margin } + offset);
                }
            }
        }
    }
}

#[test]
fn hanging_and_each_line_distinguish_forced_breaks_from_wraps() {
    for (keywords, expected) in [
        ("", [20.0, 0.0, 0.0, 0.0]),
        ("hanging", [0.0, 20.0, 20.0, 20.0]),
        ("each-line", [20.0, 0.0, 20.0, 0.0]),
        ("each-line hanging", [0.0, 20.0, 0.0, 20.0]),
    ] {
        let trace = render(
            &format!("p{{width:65pt;text-indent:20pt {keywords}}}"),
            "<p>MMMM MMMM<br>MMMM MMMM</p>",
        );
        let lines = &trace.pages[0];
        assert_eq!(
            lines.len(),
            expected.len(),
            "{keywords}: {:?}",
            lines.iter().map(|line| &line.text).collect::<Vec<_>>()
        );
        for (line, indent) in lines.iter().zip(expected) {
            assert_pt(line.x, LEFT + indent);
        }
    }
}

#[test]
fn anonymous_blocks_do_not_restart_the_first_formatted_line() {
    for (keywords, after) in [("", 0.0), ("each-line", 20.0), ("hanging", 20.0)] {
        let trace = render(
            &format!("div{{text-indent:20pt {keywords}}}p{{text-indent:0}}"),
            "<div>First<p>Block</p>After</div>",
        );
        let line = trace.pages[0]
            .iter()
            .find(|line| line.text == "After")
            .unwrap();
        assert_pt(line.x, LEFT + after);
    }
}

#[test]
fn float_band_does_not_change_percentage_basis() {
    let trace = render(
        "p{text-indent:20%}.float{float:left;width:30pt;height:30pt}",
        "<p><span class=float></span>Alpha</p>",
    );
    assert_pt(trace.pages[0][0].x, LEFT + 30.0 + 20.0);
}

#[test]
fn indentation_survives_page_breaks_without_restarting() {
    for (keywords, first, following) in [("", 20.0, 0.0), ("hanging", 0.0, 20.0)] {
        let trace = render(
            &format!(
                "@page{{size:180pt 100pt;margin:36pt}}p{{width:65pt;widows:1;orphans:1;text-indent:20pt {keywords}}}"
            ),
            &format!("<p>{}</p>", ["MMMM"; 8].join(" ")),
        );
        assert_eq!(trace.pages.len(), 4);
        for (index, line) in trace.pages.iter().flatten().enumerate() {
            assert_pt(line.x, LEFT + if index == 0 { first } else { following });
        }
    }
}

#[test]
fn indentation_is_inherited_and_supports_the_full_grammar() {
    let trace = render(
        "div{text-indent:each-line 20pt hanging}p{width:65pt}p.reset{text-indent:initial}@supports(text-indent:hanging calc(10% + 10pt) each-line){p.supported{text-indent:20pt}}",
        "<div><p>MMMM MMMM</p><p class=reset>MMMM MMMM</p><p class=supported>MMMM MMMM</p></div>",
    );
    assert_eq!(trace.pages[0].len(), 6);
    for (line, indent) in trace.pages[0].iter().zip([0.0, 20.0, 0.0, 0.0, 20.0, 0.0]) {
        assert_pt(line.x, LEFT + indent);
    }
}

#[test]
fn justified_lines_fill_only_the_indented_available_width() {
    let trace = render(
        "p{width:100pt;text-align:justify;text-indent:20pt}",
        "<p>AA AA AA AA AA AA AA AA AA AA</p>",
    );
    assert!(trace.pages[0].len() >= 2);
    let first = &trace.pages[0][0];
    assert_pt(first.x, LEFT + 20.0);
    assert_pt(first.width, WIDTH - 20.0);
    let second = &trace.pages[0][1];
    assert_pt(second.x, LEFT);
}

#[test]
fn a_float_after_text_counts_the_indent_when_testing_remaining_room() {
    let trace = render(
        "p{text-indent:30pt}.float{float:right;width:50pt;height:24pt;background:red}",
        "<p>MMMM<span class=float></span> N</p>",
    );
    let rect = common::painted_box(&trace, [255, 0, 0]);
    assert_pt(rect.y, LEFT + 12.0);
    assert_pt(trace.pages[0][0].x, LEFT + 30.0);
    assert_eq!(trace.pages[0][0].text, "MMMM N");
}
