mod common;

use common::{assert_pt, local_options};
use ferri_pdf::{LayoutTrace, render_html};

// Liberation Mono hmtx advance / unitsPerEm at 10pt.
const CELL: f32 = 1229.0 / 2048.0 * 10.0;

fn render(css: &str, body: &str) -> LayoutTrace {
    render_html(
        &format!("<style>{}p,div{{margin:0;font:10pt/12pt 'Liberation Mono';white-space:break-spaces;width:19pt}}{css}</style>{body}", common::PROFILE),
        &local_options(),
    ).unwrap_or_else(|error| panic!("break-spaces must render: {error}; CSS={css}; HTML={body}"))
        .layout.unwrap()
}

fn texts(trace: &LayoutTrace) -> Vec<&str> {
    trace
        .pages
        .iter()
        .flatten()
        .map(|line| line.text.as_str())
        .collect()
}

#[test]
fn preserved_spaces_wrap_individually_and_never_hang() {
    let trace = render("", "<p>A     B  </p>");
    assert_eq!(texts(&trace), ["A  ", "   ", "B  "]);
    for line in trace.pages.iter().flatten() {
        assert_pt(line.width, 3.0 * CELL);
    }
    let narrow = render("p{width:1pt}", "<p>  </p>");
    assert_eq!(texts(&narrow), [" ", " "]);
    for line in narrow.pages.iter().flatten() {
        assert_pt(line.width, CELL);
    }
}

#[test]
fn breaks_follow_unicode_separators_but_not_nbsp() {
    for space in ['\u{2002}', '\u{2003}', '\u{2007}', '\u{202f}', '\u{3000}'] {
        let trace = render("p{width:1pt}", &format!("<p>A{space}{space}B</p>"));
        assert_eq!(
            texts(&trace),
            [format!("A{space}"), space.to_string(), "B".into()],
            "separator {space:?}"
        );
    }
    let trace = render("p{width:1pt}", "<p>A\u{a0}\u{a0}B</p>");
    assert_eq!(texts(&trace), ["A\u{a0}\u{a0}B"]);
}

#[test]
fn tabs_remain_one_break_unit_after_case_expansion() {
    let trace = render("p{tab-size:4;text-transform:uppercase}", "<p>ß\t B</p>");
    assert_eq!(texts(&trace), ["SS    ", " B"]);
    assert_pt(trace.pages[0][0].width, 6.0 * CELL);
}

#[test]
fn inheritance_shorthand_pairs_and_forced_breaks_keep_spacing() {
    for value in ["break-spaces", "break-spaces wrap", "wrap break-spaces"] {
        let trace = render(
            &format!("p{{white-space:{value}}}"),
            "<p>A<span>   </span>B\n C<br> D</p>",
        );
        assert_eq!(texts(&trace), ["A  ", " B", " C", " D"], "{value}");
    }
    let trace = render("p{white-space:break-spaces nowrap}", "<p>A     B\n C</p>");
    assert_eq!(texts(&trace), ["A     B", " C"]);
}

#[test]
fn intrinsic_widths_include_trailing_spaces_and_each_break_unit() {
    for (width, cells) in [("1pt", 2.0), ("100pt", 9.0)] {
        let trace = render(
            &format!("div{{width:{width}}}span{{display:inline-block;background:rgb(1,2,3)}}"),
            "<div><span>A     B  </span></div>",
        );
        assert_pt(common::painted_box(&trace, [1, 2, 3]).width, cells * CELL);
    }
}

#[test]
fn mixed_modes_capability_queries_and_page_continuations_preserve_units() {
    let trace = render(
        "p{white-space:normal}@supports(white-space:break-spaces){p{white-space:break-spaces}}",
        "<p>A<span style='white-space:normal'>   B</span>   C</p>",
    );
    assert_eq!(texts(&trace), ["A", "B  ", " C"]);
    let trace = render(
        "@page{size:100pt 60pt;margin:18pt}p{orphans:1;widows:1}",
        "<p>A     B     C     D  </p>",
    );
    assert!(trace.pages.len() > 1);
    assert_eq!(
        texts(&trace),
        ["A  ", "   ", "B  ", "   ", "C  ", "   ", "D  "]
    );
}

#[test]
fn emergency_wrapping_does_not_split_a_preserved_tab() {
    let trace = render("p{tab-size:4;overflow-wrap:anywhere}", "<p>A\tB</p>");
    assert_eq!(texts(&trace), ["A", "    ", "B"]);
}
