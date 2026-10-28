//! CSS Containment 2 and 3: the effects of each containment type, `contain-intrinsic-size`
//! and `content-visibility`, measured from the layout.
mod common;

use common::{assert_labelled_pt, painted_box, render_trace};
use ferri_pdf::{DisplayItem, LayoutTrace};

const PAGE: &str = "@page{size:400pt 300pt;margin:0}body{margin:0;font:10pt/12pt serif}";
const RED: [u8; 3] = [255, 0, 0];
const BLUE: [u8; 3] = [0, 0, 255];

fn trace(css: &str, body: &str) -> LayoutTrace {
    render_trace(&format!("<!doctype html><style>{PAGE}{css}</style>{body}"))
}

fn text(trace: &LayoutTrace) -> Vec<String> {
    trace
        .pages
        .iter()
        .flatten()
        .map(|line| line.text.clone())
        .collect()
}

/// §3.1: size containment lays the box out as if empty, unless an explicit intrinsic inner
/// size stands in for the content (CSS Sizing 4 §5.2).
#[test]
fn size_containment_sizes_the_box_as_if_empty() {
    let content = "<div class=c><div style='width:150pt;height:90pt'></div></div>";
    for (css, width, height) in [
        ("float:left;contain:size", 0.0, 0.0),
        ("float:left;contain:inline-size", 0.0, 90.0),
        (
            "float:left;contain:size;contain-intrinsic-size:30pt 40pt",
            30.0,
            40.0,
        ),
        (
            "float:left;contain:size;contain-intrinsic-size:auto 25pt",
            25.0,
            25.0,
        ),
        (
            "float:left;contain:size;contain-intrinsic-width:none;contain-intrinsic-height:20pt",
            0.0,
            20.0,
        ),
        (
            "float:left;contain:inline-size;contain-intrinsic-inline-size:70pt",
            70.0,
            90.0,
        ),
        ("display:inline-flex;contain:size", 0.0, 0.0),
        (
            "display:inline-grid;grid-template-columns:20pt 30pt;column-gap:5pt;contain:size",
            55.0,
            0.0,
        ),
        ("float:left;contain:size;padding:5pt", 10.0, 10.0),
        // Size containment has no effect on tables.
        ("display:table;contain:size", 150.0, 90.0),
    ] {
        let rect = painted_box(&trace(&format!(".c{{background:red;{css}}}"), content), RED);
        assert_labelled_pt(&format!("`{css}` width"), rect.width, width);
        assert_labelled_pt(&format!("`{css}` height"), rect.height, height);
    }
}

/// §3.1: a size containment box is monolithic, so it moves whole to the next page.
#[test]
fn size_containment_boxes_are_monolithic() {
    let trace = trace(
        ".spacer{height:250pt}.c{contain:size;height:100pt;background:red}",
        "<div class=spacer></div><div class=c>Line one<br>Line two<br>Line three</div>",
    );
    let pages: Vec<_> = trace
        .display_lists
        .iter()
        .enumerate()
        .flat_map(|(page, items)| {
            items.iter().filter_map(move |item| match item {
                DisplayItem::Box(paint) if paint.decoration.background.rgb == RED => {
                    Some((page, paint.rect))
                }
                _ => None,
            })
        })
        .collect();
    assert_eq!(
        pages.len(),
        1,
        "the contained box must not fragment: {pages:?}"
    );
    assert_eq!(pages[0].0, 1, "it must move to the second page: {pages:?}");
    assert_labelled_pt("height on the second page", pages[0].1.height, 100.0);
}

/// §3.4: paint containment clips descendants to the padding box, and §3.2/§3.4 make the
/// box the containing block of absolutely positioned and fixed descendants.
#[test]
fn layout_and_paint_containment_contain_positioned_descendants() {
    for contain in ["layout", "paint", "content", "strict"] {
        let trace = trace(
            &format!(
                ".c{{contain:{contain};margin:50pt 0 0 60pt;width:100pt;height:80pt;background:blue}}\
                 .p{{position:fixed;left:10pt;top:10pt;width:20pt;height:20pt;background:red}}"
            ),
            "<div class=c><div class=p></div></div>",
        );
        let positioned = painted_box(&trace, RED);
        assert_labelled_pt(&format!("{contain} fixed x"), positioned.x, 70.0);
        assert_labelled_pt(&format!("{contain} fixed y"), positioned.y, 60.0);
    }
    let clipped = trace(
        ".c{contain:paint;width:100pt;height:40pt}.o{width:200pt;height:200pt;background:red}",
        "<div class=c><div class=o></div></div>",
    );
    let clip = clipped
        .display_lists
        .iter()
        .flatten()
        .find_map(|item| match item {
            DisplayItem::PushClip(clip) => Some(clip.rect),
            _ => None,
        })
        .expect("paint containment pushes a clip");
    assert_labelled_pt("clip width", clip.width, 100.0);
    assert_labelled_pt("clip height", clip.height, 40.0);
}

/// §3.2: layout containment makes an independent formatting context, laid out exactly as
/// `display: flow-root`: margins do not collapse through it and floats do not intrude.
#[test]
fn layout_containment_establishes_an_independent_formatting_context() {
    let css = ".f{float:left;width:100pt;height:50pt}.c{background:red}\
               .m{margin-top:30pt;height:10pt}";
    let body = "<div class=f></div><div class=c><div class=m></div></div>";
    let contained = painted_box(&trace(&format!("{css}.c{{contain:layout}}"), body), RED);
    let root = painted_box(&trace(&format!("{css}.c{{display:flow-root}}"), body), RED);
    assert_eq!(contained, root, "layout containment against flow-root");
    assert_labelled_pt("own margin inside", contained.height, 40.0);
    let plain = painted_box(&trace(css, body), RED);
    assert_labelled_pt(
        "without containment the margin collapses through",
        plain.height,
        10.0,
    );
}

/// §3.3: increments inside a style-contained subtree create a new counter instead of
/// changing the outer one, which is still readable (the specification's `1 1.2` example).
#[test]
fn style_containment_scopes_counters_and_quotes() {
    let trace = trace(
        "div{contain:style;counter-increment:n}div::before,div::after{content:counters(n,'.') ' '}\
         div::after{counter-increment:n 2}p::before{content:counter(n)}",
        "<div></div><p></p>",
    );
    assert_eq!(text(&trace), ["1 1.2", "1"], "scoped counters");
    for (contain, expected) in [("style", "«x«b»"), ("none", "«x‹b›")] {
        let quotes = self::trace(
            &format!(
                "p,span,q{{quotes:'«' '»' '‹' '›'}}.c{{contain:{contain}}}.c::before{{content:open-quote 'x'}}"
            ),
            "<p><span class=c></span><q>b</q></p>",
        );
        assert_eq!(
            text(&quotes),
            [expected],
            "with `contain: {contain}` the unclosed quote's depth must {} escape the span",
            if contain == "none" { "" } else { "not" }
        );
    }
}

/// §4: `hidden` skips the contents, which applies size containment; `auto` keeps them, since
/// every element is relevant to the user in paged media, with layout, style and paint
/// containment.
#[test]
fn content_visibility_hidden_skips_contents() {
    let hidden = trace(
        ".c{content-visibility:hidden;float:left;padding:4pt;background:red}\
         .c::before{content:'Before'}",
        "<div class=c>Skipped</div><p>After</p>",
    );
    assert_eq!(
        text(&hidden),
        ["After"],
        "skipped contents are not rendered"
    );
    let rect = painted_box(&hidden, RED);
    assert_labelled_pt("hidden width", rect.width, 8.0);
    assert_labelled_pt("hidden height", rect.height, 8.0);
    let auto = trace(
        ".c{content-visibility:auto;background:blue}.p{position:fixed;top:5pt;left:5pt}",
        "<div style='height:20pt'></div><div class=c>Kept<span class=p>Fixed</span></div>",
    );
    assert_eq!(text(&auto), ["Kept", "Fixed"], "auto renders its contents");
    let fixed = auto.pages[0]
        .iter()
        .find(|line| line.text == "Fixed")
        .expect("fixed line");
    let container = painted_box(&auto, BLUE);
    assert!(
        fixed.baseline > container.y,
        "auto's layout containment positions the fixed box inside the container at {} (baseline {})",
        container.y,
        fixed.baseline
    );
}
