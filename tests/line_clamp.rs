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

#[test]
fn ignored_image_lines_do_not_spend_clamp_budgets_or_create_ellipsis() {
    const IMAGE: &str = "<img src=missing.png alt=''>";
    for css in [
        "line-clamp:1",
        "max-lines:1;continue:discard;block-ellipsis:auto",
    ] {
        for (name, before, after) in [
            ("leading", IMAGE.to_owned(), String::new()),
            (
                "wrapped leading",
                format!("<div>{IMAGE}</div>"),
                String::new(),
            ),
            ("trailing", String::new(), IMAGE.to_owned()),
            (
                "wrapped trailing",
                String::new(),
                format!("<div>{IMAGE}</div>"),
            ),
            (
                "nested trailing",
                String::new(),
                format!("<div><div>{IMAGE}</div></div>"),
            ),
        ] {
            let control = trace(
                &format!(".c{{{css}}}"),
                "<div class=c><p>First</p></div><p>Outside</p>",
            );
            let actual = trace(
                &format!(".c{{{css}}}"),
                &format!("<div class=c>{before}<p>First</p>{after}</div><p>Outside</p>"),
            );
            let visible: Vec<_> = lines(&actual)
                .into_iter()
                .filter(|line| line.text != "\u{fffc}")
                .map(|line| (line.text.as_str(), line.baseline))
                .collect();
            let expected: Vec<_> = lines(&control)
                .into_iter()
                .map(|line| (line.text.as_str(), line.baseline))
                .collect();
            assert_eq!(
                visible, expected,
                "{css}; {name}: ignored lines retain the first real line, do not add an ellipsis, and do not advance Outside"
            );
        }
    }
}

#[test]
fn ignored_lines_leave_the_last_real_line_available_for_a_real_clamp() {
    for image in [
        "<img src=missing.png alt=''>",
        "<div><img src=missing.png alt=''></div>",
    ] {
        let actual = trace(
            ".c{line-clamp:1}",
            &format!("<div class=c>{image}<p>First</p>{image}<p>Second</p></div><p>Outside</p>"),
        );
        let visible: Vec<_> = texts(&actual)
            .into_iter()
            .filter(|text| *text != "\u{fffc}")
            .collect();
        assert_eq!(
            visible,
            ["First…", "Outside"],
            "{image}: the second real line forces the break and ellipsis"
        );
    }
    // A real line remains countable even when its line-height is zero.
    let zero = trace(
        ".c{line-clamp:1}.zero{font-size:0;line-height:0}",
        "<div class=c><p class=zero>Zero</p><p>Second</p></div><p>Outside</p>",
    );
    assert_eq!(
        texts(&zero),
        ["Zero…", "Outside"],
        "text with zero geometric height still consumes one line"
    );
}

#[test]
fn ignored_clamp_lines_preserve_source_events_and_destinations() {
    for image in [
        "<img id=empty src=missing.png alt='' style='string-set:label \"Empty\"'>".to_owned(),
        "<div id=empty style='string-set:label \"Empty\"'><img src=missing.png alt=''></div>"
            .to_owned(),
    ] {
        let actual = trace(
            "@page{@top-left{content:string(label,last)}}.c{line-clamp:1}",
            &format!("<div class=c><p>First</p>{image}</div><p><a href='#empty'>Outside</a></p>"),
        );
        assert!(
            texts(&actual).contains(&"First"),
            "ignored trailing source must not add ellipsis: {:?}",
            texts(&actual)
        );
        assert_eq!(
            texts(&actual)
                .iter()
                .filter(|text| **text == "Empty")
                .count(),
            1,
            "the retained ignored source sets the actual page header once"
        );
        assert_eq!(actual.links[0].len(), 1, "source link remains clickable");
        assert!(
            actual.destinations.iter().any(Option::is_some),
            "ignored image or wrapper destination remains addressable"
        );
    }
}

#[test]
fn clamp_lookahead_keeps_forced_styled_and_independent_content() {
    for following in [
        "<img src=missing.png alt='' style='width:0;height:0'>",
        "<img src=missing.png alt=''><br>",
        "<span style='white-space:pre'> </span>",
        "<div style='border:1pt solid red'><img src=missing.png alt=''></div>",
        "<div style='display:flow-root'><img src=missing.png alt=''></div>",
    ] {
        let actual = trace(
            ".c{line-clamp:1}",
            &format!("<div class=c><p>First</p>{following}</div><p>Outside</p>"),
        );
        assert_eq!(
            texts(&actual),
            ["First…", "Outside"],
            "{following}: actual following content still forces a clamp"
        );
    }
}

#[test]
fn height_discard_counts_real_lines_after_ignored_sources() {
    let actual = trace(
        &format!(".c{{continue:discard;height:{LINE}pt;block-ellipsis:auto}}"),
        "<div class=c><div><img src=missing.png alt=''></div><p>First</p><p>Second</p></div><p>Outside</p>",
    );
    let visible: Vec<_> = texts(&actual)
        .into_iter()
        .filter(|text| *text != "\u{fffc}")
        .collect();
    assert_eq!(
        visible,
        ["First…", "Outside"],
        "height-based discard must use the same real-line counts as max-lines"
    );
}

#[test]
fn a_list_marker_is_real_following_content_after_an_ignored_line() {
    const RULES: &str =
        ".c{line-clamp:1;padding-left:40pt}.item{display:list-item;list-style-type:disc}";
    let forced = trace(
        RULES,
        "<div class=c><p>First</p><div class=item><br></div></div><p>Outside</p>",
    );
    let ignored = trace(
        RULES,
        "<div class=c><p>First</p><div class=item><img src=missing.png alt=''></div></div><p>Outside</p>",
    );
    assert_eq!(
        texts(&forced),
        ["First…", "Outside"],
        "a real following list item must force the region break"
    );
    assert_eq!(
        texts(&ignored),
        texts(&forced),
        "an ignored image line does not make the list marker empty following content"
    );
}
