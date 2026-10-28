//! CSS Overflow 4 §5: `line-clamp`, `max-lines`, `continue` and `block-ellipsis`.
mod common;

use common::{PROFILE, assert_labelled_pt, render_trace};
use ferri_pdf::{LayoutTrace, TextLine};

const ELLIPSIS: char = '\u{2026}';
/// The profile's line height.
const LINE: f32 = 18.0;
const TEXT: &str = "One two three four five six seven eight nine ten eleven twelve \
                    thirteen fourteen fifteen sixteen seventeen eighteen nineteen twenty";

fn trace(css: &str, body: &str) -> LayoutTrace {
    render_trace(&format!(
        "<!doctype html><style>{PROFILE}p,div{{margin:0;width:150pt}}{css}</style>{body}"
    ))
}

fn lines(trace: &LayoutTrace) -> Vec<&TextLine> {
    trace.pages.iter().flatten().collect()
}

fn texts(trace: &LayoutTrace) -> Vec<&str> {
    lines(trace).iter().map(|line| line.text.as_str()).collect()
}

#[test]
fn line_clamp_keeps_lines_ends_with_an_ellipsis_and_sizes_the_box() {
    let full = trace("", &format!("<p>{TEXT}</p><p>after</p>"));
    let clamped = trace(
        ".c{line-clamp:2}",
        &format!("<p class=c>{TEXT}</p><p>after</p>"),
    );
    let kept = texts(&clamped);
    assert_eq!(
        kept.len(),
        3,
        "two clamped lines and the next paragraph: {kept:?}"
    );
    assert_eq!(kept[0], texts(&full)[0]);
    assert!(kept[1].ends_with(ELLIPSIS), "{kept:?}");
    assert!(
        texts(&full)[1].starts_with(kept[1].trim_end_matches(ELLIPSIS).trim_end()),
        "the ellipsis line keeps a prefix of the second line: {kept:?} vs {:?}",
        texts(&full)
    );
    assert!(lines(&clamped)[1].width <= 150.0);
    assert_eq!(kept[2], "after");
    assert_labelled_pt(
        "the next paragraph follows the second line",
        lines(&clamped)[2].baseline - lines(&clamped)[0].baseline,
        2.0 * LINE,
    );
    let string = trace(
        ".c{line-clamp:2 ' [more]'}",
        &format!("<p class=c>{TEXT}</p>"),
    );
    assert!(
        texts(&string)[1].ends_with(" [more]"),
        "{:?}",
        texts(&string)
    );
    let none = trace(".c{line-clamp:2 none}", &format!("<p class=c>{TEXT}</p>"));
    assert_eq!(
        texts(&none),
        texts(&full)[..2],
        "block-ellipsis: none only truncates"
    );
    let short = trace(".c{line-clamp:5}", "<p class=c>one two</p>");
    assert_eq!(texts(&short), ["one two"], "fewer lines than the clamp");
}

#[test]
fn max_lines_counts_nested_lines_of_the_same_formatting_context() {
    // CSS Overflow 4 §5.2 examples.
    let body = "<div id=a>a: line 1<br>a: line 2<div id=b>b: line 1<br>b: line 2<br>\
                b: line 3<br>b: line 4</div>a: line 3<br>a: line 4</div>";
    let nested = trace("#a{line-clamp:5}#b{line-clamp:2}", body);
    assert_eq!(
        texts(&nested),
        [
            "a: line 1",
            "a: line 2",
            "b: line 1",
            "b: line 2…",
            "a: line 3…"
        ]
    );
    let outer = trace("#a{line-clamp:3}#b{line-clamp:2}", body);
    assert_eq!(texts(&outer), ["a: line 1", "a: line 2", "b: line 1…"]);
    // An independent formatting context is skipped over while counting.
    let independent = trace("#a{line-clamp:3}#b{display:flow-root}", body);
    assert_eq!(
        texts(&independent),
        [
            "a: line 1",
            "a: line 2",
            "b: line 1",
            "b: line 2",
            "b: line 3",
            "b: line 4",
            "a: line 3…"
        ]
    );
}

#[test]
fn continue_discard_breaks_where_the_block_size_overflows() {
    let height = 2.5 * LINE;
    let discarded = trace(
        &format!(".h{{continue:discard;height:{height}pt}}"),
        &format!("<div class=h>{TEXT}</div><p>after</p>"),
    );
    let kept = texts(&discarded);
    assert_eq!(kept.len(), 3, "{kept:?}");
    assert!(!kept[1].ends_with(ELLIPSIS), "block-ellipsis is none");
    assert_eq!(kept[2], "after");
    let ellipsis = trace(
        &format!(".h{{continue:discard;max-height:{height}pt;block-ellipsis:auto}}"),
        &format!("<div class=h>{TEXT}</div>"),
    );
    assert_eq!(texts(&ellipsis).len(), 2);
    assert!(
        texts(&ellipsis)[1].ends_with(ELLIPSIS),
        "{:?}",
        texts(&ellipsis)
    );
    let visible = trace(
        &format!(".h{{height:{height}pt}}"),
        &format!("<div class=h>{TEXT}</div>"),
    );
    assert!(
        texts(&visible).len() > 3,
        "continue: auto lets content overflow"
    );
}

#[test]
fn webkit_line_clamp_applies_inside_a_vertical_webkit_box() {
    let vertical = trace(
        ".w{display:-webkit-box;-webkit-box-orient:vertical;-webkit-line-clamp:2;overflow:hidden}",
        &format!("<div class=w>{TEXT}</div>"),
    );
    assert_eq!(texts(&vertical).len(), 2, "{:?}", texts(&vertical));
    assert!(texts(&vertical)[1].ends_with(ELLIPSIS));
    let horizontal = trace(
        ".w{display:-webkit-box;-webkit-line-clamp:2}",
        &format!("<div class=w>{TEXT}</div>"),
    );
    assert!(
        texts(&horizontal).len() > 2,
        "-webkit-discard needs a vertical box"
    );
    let block = trace(
        ".w{-webkit-line-clamp:2}",
        &format!("<div class=w>{TEXT}</div>"),
    );
    assert!(
        texts(&block).len() > 2,
        "-webkit-discard needs a -webkit-box parent"
    );
    let standard = trace(
        ".w{display:block;line-clamp:2}",
        &format!("<div class=w>{TEXT}</div>"),
    );
    assert_eq!(texts(&standard), texts(&vertical));
}

#[test]
fn page_breaks_inside_a_clamped_box_discard_nothing() {
    let paged = trace(
        "@page{size:300pt 120pt;margin:20pt}.c{line-clamp:6}",
        &format!("<p class=c>{TEXT} {TEXT}</p>"),
    );
    assert_eq!(paged.pages.len(), 2, "{:?}", texts(&paged));
    let kept = texts(&paged);
    assert_eq!(kept.len(), 6, "six lines across both pages: {kept:?}");
    assert!(kept[5].ends_with(ELLIPSIS));
    assert!(kept[..5].iter().all(|text| !text.ends_with(ELLIPSIS)));
}
