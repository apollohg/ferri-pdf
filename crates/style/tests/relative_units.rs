//! CSS Values 4 §6.1 relative lengths at computed-value time, against the bundled faces' own
//! tables.
use ferri_pdf_dom::{DocumentTree, Selector};
use ferri_pdf_resources::FileResolver;
use ferri_pdf_style::{LengthPercentage, Size, Style, Stylesheet, StylesheetSource};

const TOLERANCE_PT: f32 = 1e-3;
const UNITS_PER_EM: f32 = 2048.0;
/// `DejaVu` Sans has no OS/2 x-height or cap height, so `ex` and `cap` take the tops of `x` and
/// `H`; its `0` advances 1303 units.
const DEJAVU_X_TOP: f32 = 1120.0;
const DEJAVU_H_TOP: f32 = 1493.0;
const DEJAVU_ZERO_ADVANCE: f32 = 1303.0;
/// Liberation Serif's OS/2 `sxHeight` and `sCapHeight`, its `0` advance, and its normal line
/// height: ascender 1825, descender 443 and line gap 87.
const SERIF_X_HEIGHT: f32 = 940.0;
const SERIF_CAP_HEIGHT: f32 = 1341.0;
const SERIF_ZERO_ADVANCE: f32 = 1024.0;
const SERIF_NORMAL_LINE: f32 = 1825.0 + 443.0 + 87.0;
const INITIAL_FONT_SIZE: f32 = 12.0;

/// The `#target` element's style in `<html><body><div id=target>`, after the page geometry
/// is resolved from the root as documents do.
fn target(css: &str) -> (Style, Stylesheet) {
    let resolver = FileResolver::default();
    let mut sheet = Stylesheet::parse(&[StylesheetSource {
        text: css.into(),
        base: Default::default(),
    }])
    .unwrap_or_else(|error| panic!("{css}: {error}"));
    let html = DocumentTree::parse("<div id=target>X</div>").expect("parse test HTML");
    let select = |selector: &str| {
        html.select(&Selector::parse(selector).unwrap())
            .next()
            .unwrap()
    };
    let root = sheet
        .resolve_document_root(select("html"), &resolver)
        .unwrap();
    let body = sheet
        .computed_with_resolver(select("body"), &root, &resolver)
        .unwrap();
    let target = sheet
        .computed_with_resolver(select("#target"), &body, &resolver)
        .unwrap_or_else(|error| panic!("{css}: {error}"));
    (target, sheet)
}

fn points(size: &Size) -> f32 {
    match size {
        Size::Value(LengthPercentage::Length(points)) => *points,
        other => panic!("{other:?} is not a computed length"),
    }
}

/// A rule, the computed field it governs, and the expected points.
type Case<'a> = (&'a str, fn(&Style) -> f32, f32);

fn check(css: &str, cases: &[Case<'_>]) {
    let (style, _) = target(css);
    let failures: Vec<String> = cases
        .iter()
        .filter_map(|(what, field, expected)| {
            let actual = field(&style);
            ((actual - expected).abs() > TOLERANCE_PT)
                .then(|| format!("{what}: computed {actual}pt, expected {expected}pt"))
        })
        .collect();
    assert!(failures.is_empty(), "{css}\n{}", failures.join("\n"));
}

const MARGIN_LEFT: fn(&Style) -> f32 = |style| points(&style.margin[3]);
const MARGIN_RIGHT: fn(&Style) -> f32 = |style| points(&style.margin[1]);
const MARGIN_TOP: fn(&Style) -> f32 = |style| points(&style.margin[0]);
const MARGIN_BOTTOM: fn(&Style) -> f32 = |style| points(&style.margin[2]);
const FONT_SIZE: fn(&Style) -> f32 = |style| style.size;

#[test]
fn font_relative_units_measure_the_first_available_font_or_the_glyph_that_renders() {
    let size = 100.0;
    check(
        "#target{font-family:'DejaVu Sans';font-size:100pt;margin:1ex 1cap 1ch 1ic}",
        &[
            (
                "ex is the top of x without an OS/2 x-height",
                MARGIN_TOP,
                DEJAVU_X_TOP / UNITS_PER_EM * size,
            ),
            (
                "cap is the top of H without an OS/2 cap height",
                MARGIN_RIGHT,
                DEJAVU_H_TOP / UNITS_PER_EM * size,
            ),
            (
                "ch is the advance of 0",
                MARGIN_BOTTOM,
                DEJAVU_ZERO_ADVANCE / UNITS_PER_EM * size,
            ),
            ("ic is 1em when no face has 水", MARGIN_LEFT, size),
        ],
    );
    check(
        "#target{font-size:100pt;margin:1ex 1cap 1ch 2em}",
        &[
            (
                "ex is the initial serif face's sxHeight",
                MARGIN_TOP,
                SERIF_X_HEIGHT / UNITS_PER_EM * size,
            ),
            (
                "cap is its sCapHeight",
                MARGIN_RIGHT,
                SERIF_CAP_HEIGHT / UNITS_PER_EM * size,
            ),
            (
                "ch is its 0 advance",
                MARGIN_BOTTOM,
                SERIF_ZERO_ADVANCE / UNITS_PER_EM * size,
            ),
            ("em", MARGIN_LEFT, 2.0 * size),
        ],
    );
    check(
        "#target{font-size:20pt;margin-left:calc(1ex + 1ch);margin-right:max(1lh, 1cap);line-height:10pt}",
        &[
            (
                "math functions take font-relative units",
                MARGIN_LEFT,
                (SERIF_X_HEIGHT + SERIF_ZERO_ADVANCE) / UNITS_PER_EM * 20.0,
            ),
            (
                "max() compares them",
                MARGIN_RIGHT,
                SERIF_CAP_HEIGHT / UNITS_PER_EM * 20.0,
            ),
        ],
    );
}

#[test]
fn font_size_and_line_height_measure_the_parents_font_and_line() {
    let body = "body{font-size:10pt;line-height:30pt;font-family:'DejaVu Sans'}";
    for (declarations, expected, rule) in [
        (
            "font-size:2ex",
            2.0 * DEJAVU_X_TOP / UNITS_PER_EM * 10.0,
            "ex in font-size is the parent's",
        ),
        (
            "font-size:2ch",
            2.0 * DEJAVU_ZERO_ADVANCE / UNITS_PER_EM * 10.0,
            "ch in font-size is the parent's",
        ),
        (
            "font-size:1lh",
            30.0,
            "lh in font-size is the parent's line height",
        ),
        (
            "font-size:50%",
            5.0,
            "percentages are the parent's font size",
        ),
    ] {
        check(
            &format!("{body}#target{{{declarations};font-family:serif}}"),
            &[(rule, FONT_SIZE, expected)],
        );
    }
    check(
        &format!("{body}#target{{line-height:1lh;margin-left:1lh;font-size:20pt}}"),
        &[(
            "lh in line-height is the parent's, then the element's",
            MARGIN_LEFT,
            30.0,
        )],
    );
    check(
        "#target{font-size:10pt;line-height:normal;margin-left:10lh;margin-right:1lh;line-height:2}",
        &[(
            "a number line height scales the element's font size",
            MARGIN_RIGHT,
            20.0,
        )],
    );
    check(
        "#target{font-size:10pt;margin-left:10lh}",
        &[(
            "normal uses the first available font's ascent, descent and gap",
            MARGIN_LEFT,
            SERIF_NORMAL_LINE / UNITS_PER_EM * 100.0,
        )],
    );
}

#[test]
fn root_relative_units_measure_the_root_and_its_own_font_size_uses_initial_values() {
    check(
        "html{font-size:20pt;line-height:40pt;font-family:'DejaVu Sans'}body{font-size:10pt;line-height:12pt}#target{font-family:serif;margin:1rem 1rlh 1rex 1rch}",
        &[
            ("rem", MARGIN_TOP, 20.0),
            ("rlh", MARGIN_RIGHT, 40.0),
            (
                "rex measures the root's face",
                MARGIN_BOTTOM,
                DEJAVU_X_TOP / UNITS_PER_EM * 20.0,
            ),
            (
                "rch measures the root's face",
                MARGIN_LEFT,
                DEJAVU_ZERO_ADVANCE / UNITS_PER_EM * 20.0,
            ),
        ],
    );
    let initial_line = SERIF_NORMAL_LINE / UNITS_PER_EM * INITIAL_FONT_SIZE;
    check(
        "html{font-size:2rem;line-height:2rlh}#target{margin-left:1rem;margin-right:1rlh}",
        &[
            (
                "rem in the root's font-size is the initial font size",
                MARGIN_LEFT,
                2.0 * INITIAL_FONT_SIZE,
            ),
            (
                "rlh in the root's line-height is the initial normal line height",
                MARGIN_RIGHT,
                2.0 * initial_line,
            ),
        ],
    );
}

#[test]
fn viewport_units_are_percentages_of_the_page_area() {
    // A 400pt × 600pt page with 50pt and 20pt margins has a 360pt × 500pt page area.
    let page = "@page{size:400pt 600pt;margin:50pt 20pt}";
    for (unit, expected) in [
        ("vw", 36.0),
        ("vh", 50.0),
        ("vmin", 36.0),
        ("vmax", 50.0),
        ("svw", 36.0),
        ("lvh", 50.0),
        ("dvmin", 36.0),
        ("vi", 36.0),
        ("vb", 50.0),
        ("cqw", 36.0),
        ("cqb", 50.0),
        ("cqmax", 50.0),
    ] {
        check(
            &format!("{page}#target{{margin-left:10{unit}}}"),
            &[(unit, MARGIN_LEFT, expected)],
        );
    }
    check(
        &format!("{page}html{{font-size:20pt}}#target{{margin-left:calc(10vw - 1rem)}}"),
        &[("calc mixes viewport and root units", MARGIN_LEFT, 16.0)],
    );
}

#[test]
fn page_context_lengths_use_its_font_and_the_first_page_area() {
    for (css, side, expected, rule) in [
        (
            "@page{size:400pt 600pt;margin:0 0 0 2em}html{font-size:20pt}",
            3,
            40.0,
            "em is the page context's font, inherited from the root",
        ),
        (
            "@page{size:400pt 600pt;margin:0 0 0 2em;font-size:30pt}html{font-size:20pt}",
            3,
            60.0,
            "@page font-size sets the page context's font",
        ),
        (
            "@page{size:400pt 600pt;margin:0 0 0 1rem}html{font-size:20pt}",
            3,
            20.0,
            "rem is the root's",
        ),
        (
            "@page{size:400pt 600pt;margin:0 0 0 1ex;font-family:'DejaVu Sans';font-size:100pt}",
            3,
            DEJAVU_X_TOP / UNITS_PER_EM * 100.0,
            "ex measures the page context's face",
        ),
        (
            "@page{size:400pt 600pt;margin:0 0 0 1lh;line-height:50pt}",
            3,
            50.0,
            "lh is the page context's line height",
        ),
        (
            "@page{size:400pt 600pt;margin:50pt 0 0 10vh}",
            3,
            55.0,
            "vh measures the 550pt page area height without a cycle",
        ),
    ] {
        let (_, sheet) = target(css);
        assert!(
            (sheet.margin[side] - expected).abs() < TOLERANCE_PT,
            "{css}: {rule}; margin {:?}",
            sheet.margin
        );
    }
    let (_, sheet) = target("@page{size:30em 40em}html{font-size:20pt}");
    assert_eq!(
        sheet.page,
        [600.0, 800.0],
        "size lengths in em use the page context's font, the root's 20pt"
    );
}
