use crate::dom::{DocumentTree, Selector};
use crate::paint::MAX_CALC_MAGNITUDE;
use crate::resources::FileResolver;
use crate::style::{LengthPercentage, Size, Style, StyleError, Stylesheet, StylesheetSource};

const TOLERANCE: f32 = 1e-4;

fn computed(css: &str) -> Result<Style, StyleError> {
    let sheet = Stylesheet::parse(&[StylesheetSource {
        text: css.into(),
        base: Default::default(),
    }])?;
    let html = DocumentTree::parse("<section id=parent><div id=child>X</div></section>")
        .expect("parse test HTML");
    let select = |id: &str| html.select(&Selector::parse(id).unwrap()).next().unwrap();
    let parent = sheet.computed_with_resolver(
        select("#parent"),
        &Style::default(),
        &FileResolver::default(),
    )?;
    sheet.computed_with_resolver(select("#child"), &parent, &FileResolver::default())
}
fn child(declarations: &str) -> Style {
    computed(&format!(
        "#parent{{font-size:10pt}}#child{{{declarations}}}"
    ))
    .unwrap_or_else(|error| panic!("{declarations} must compute: {error}"))
}
fn used(size: &Size, basis: f32) -> f32 {
    size.resolve(basis)
        .unwrap_or_else(|| panic!("{size:?} must not be auto"))
}
fn assert_close(actual: f32, expected: f32, context: &str) {
    assert!(
        (actual - expected).abs() < TOLERANCE,
        "{context}: expected {expected}, got {actual}"
    );
}

#[test]
fn invalid_math_functions_drop_the_declaration_and_keep_the_earlier_value() {
    for invalid in [
        "calc(10pt + 0)",
        "calc(5pt+3pt)",
        "calc(5pt -3pt)",
        "calc(10pt * 10pt)",
        "calc(10pt / 1pt)",
        "calc(0)",
        "calc()",
        "calc(10pt + 1deg)",
        "min(10pt, 2)",
        "round(25pt)",
        "round(sideways, 25pt, 1pt)",
        "clamp(1pt, 2pt)",
        "clamp(1pt, none, 2pt)",
        "sin(10pt)",
        "pow(2pt, 2)",
        "calc(1fr)",
        "calc(foo)",
        "calc(10pt",
        "calc(1s)",
    ] {
        let style = child(&format!("margin-left:9pt;margin-left:{invalid}"));
        assert_eq!(
            style.margin[3],
            Size::Value(LengthPercentage::Length(9.0)),
            "invalid {invalid} must be dropped at parse time so the earlier 9pt wins"
        );
    }
}

#[test]
fn invalid_math_inside_shorthands_drops_the_whole_shorthand() {
    let style = child(
        "border-left:4pt solid navy;border-left:calc(10%) solid red;padding:1pt;padding:1pt calc(2pt + 0)",
    );
    assert_eq!(
        style.decoration.borders[3].width, 4.0,
        "percentages are not border widths, so the second border-left is dropped"
    );
    assert_eq!(
        style.decoration.borders[3].color.rgb,
        [0, 0, 128],
        "the dropped shorthand must not leak its colour"
    );
    assert_eq!(
        style.padding,
        std::array::from_fn::<_, 4, _>(|_| LengthPercentage::Length(1.0)),
        "a type error in one padding component invalidates every side"
    );
}

#[test]
fn percentages_stay_unresolved_until_the_containing_size_is_known() {
    let style = child(
        "width:calc(100% - 20pt);min-width:min(50%, 30pt);max-width:clamp(40pt, 75%, 150pt);margin-left:calc((100% - 20pt) / 2 + max(10%, 5pt));padding-left:calc(10% - 100pt)",
    );
    assert!(
        matches!(style.width, Size::Value(LengthPercentage::Calc(_))),
        "a mixed calculation keeps its percentage term: {:?}",
        style.width
    );
    for (basis, width, minimum, maximum, margin) in [
        (200.0, 180.0, 30.0, 150.0, 110.0),
        (100.0, 80.0, 30.0, 75.0, 50.0),
        (40.0, 20.0, 20.0, 40.0, 15.0),
    ] {
        let context = format!("basis {basis}pt");
        assert_close(
            used(&style.width, basis),
            width,
            &format!("{context} width"),
        );
        assert_close(
            used(&style.min_width, basis),
            minimum,
            &format!("{context} min()"),
        );
        assert_close(
            used(&style.max_width, basis),
            maximum,
            &format!("{context} clamp()"),
        );
        assert_close(
            used(&style.margin[3], basis),
            margin,
            &format!("{context} nested"),
        );
        assert_close(
            style.padding[3].resolve(basis),
            0.0,
            &format!("{context} negative padding clamps to zero at used time"),
        );
    }
    assert_eq!(
        style.width.resolve_definite(None),
        None,
        "a percentage calculation against an indefinite basis behaves as auto"
    );
}

#[test]
fn basis_independent_calculations_fold_to_plain_lengths_and_percentages() {
    let style = child(
        "width:calc(10% * 2);margin-left:calc((1em + 2pt) * 2);margin-right:calc(1in - 1cm + 4px + 1pc + 8Q);height:calc(50% - 50%);min-height:calc(100% / 3)",
    );
    assert_eq!(style.width, Size::Value(LengthPercentage::Percentage(0.2)));
    assert_eq!(
        style.margin[3],
        Size::Value(LengthPercentage::Length(24.0)),
        "em resolves against the element's 10pt font size"
    );
    assert_close(
        used(&style.margin[1], 0.0),
        72.0 - 72.0 / 2.54 + 3.0 + 12.0 + 72.0 / 25.4 * 2.0,
        "absolute units convert to points",
    );
    assert_eq!(
        style.height,
        Size::Value(LengthPercentage::Percentage(0.0)),
        "cancelling percentages retain their percentage basis"
    );
    assert!(style.height.is_zero(), "folded zero must report zero");
    assert_eq!(style.height.resolve_definite(None), None);
    assert_eq!(style.height.resolve_definite(Some(100.0)), Some(0.0));
    assert!(
        matches!(style.min_height, Size::Value(LengthPercentage::Percentage(p)) if (p - 1.0 / 3.0).abs() < TOLERANCE),
        "{:?}",
        style.min_height
    );
}

#[test]
fn values4_functions_evaluate_with_spec_rounding_and_signs() {
    for (value, expected) in [
        ("round(23pt, 10pt)", 20.0),
        ("round(25pt, 10pt)", 30.0),
        ("round(up, 21pt, 10pt)", 30.0),
        ("round(down, 29pt, -10pt)", 20.0),
        ("round(to-zero, -25pt, 10pt)", -20.0),
        ("calc(round(-2.5) * 1pt)", -2.0),
        ("mod(-27pt, 10pt)", 3.0),
        ("rem(-27pt, 10pt)", -7.0),
        ("abs(-20pt)", 20.0),
        ("calc(sign(-5pt) * 15pt)", -15.0),
        ("calc(sin(30deg) * 40pt)", 20.0),
        ("calc(cos(pi) * 10pt)", -10.0),
        ("calc(atan2(1pt, 1pt) / 1deg * 1pt)", 45.0),
        ("calc(acos(0) / 1turn * 4pt)", 1.0),
        ("hypot(30pt, 40pt)", 50.0),
        ("calc(pow(2, 3) * 1pt + sqrt(16) * 1pt)", 12.0),
        ("calc(log(8, 2) * 1pt + exp(0) * 1pt)", 4.0),
        ("calc(10pt * 10pt / 1pt)", 100.0),
        ("calc(10pt * (1s / 500ms))", 20.0),
        ("calc(1pt * 96dpi / 1dppx)", 1.0),
        ("CALC(12pt)", 12.0),
    ] {
        let style = child(&format!("margin-left:{value}"));
        assert_close(used(&style.margin[3], 0.0), expected, value);
    }
}

#[test]
fn non_finite_results_censor_nan_and_clamp_infinity() {
    for (value, basis, expected) in [
        ("calc(1pt / 0)", 0.0, MAX_CALC_MAGNITUDE),
        ("calc(-1pt / 0)", 0.0, -MAX_CALC_MAGNITUDE),
        ("calc(-infinity * 1pt)", 0.0, -MAX_CALC_MAGNITUDE),
        ("calc(NaN * 1pt)", 0.0, 0.0),
        ("calc(1pt / 0 * 0 + 3pt)", 0.0, 0.0),
        ("min(calc(1pt / 0), 40pt)", 0.0, 40.0),
        ("calc(100% * infinity)", 50.0, MAX_CALC_MAGNITUDE),
        ("calc(100% / 0 - 100% / 0)", 50.0, 0.0),
        (
            "round(up, 1pt, calc(infinity * 1pt))",
            0.0,
            MAX_CALC_MAGNITUDE,
        ),
        ("round(down, 1pt, calc(infinity * 1pt))", 0.0, 0.0),
        ("round(1pt, 0pt)", 0.0, 0.0),
        ("mod(-1pt, calc(infinity * 1pt))", 0.0, 0.0),
        ("rem(-1pt, calc(infinity * 1pt))", 0.0, -1.0),
        ("mod(1pt, calc(infinity * 1pt))", 0.0, 1.0),
    ] {
        let style = child(&format!("margin-left:{value}"));
        assert_close(used(&style.margin[3], basis), expected, value);
    }
    let style = child("width:calc(-1pt / 0);padding-left:calc(-100% * infinity)");
    assert_eq!(
        used(&style.width, 0.0),
        0.0,
        "negative infinity clamps into width's range"
    );
    assert_eq!(
        style.padding[3].resolve(10.0),
        0.0,
        "and into padding's range"
    );
}

#[test]
fn numeric_contexts_round_integers_and_clamp_ranges() {
    let style = child(
        "order:calc(5 / 2);z-index:calc(-5 / 2);widows:calc(1 - 4);orphans:calc(7 / 2);flex-grow:calc(1 - 3);flex-shrink:calc(3 / 2);font-weight:calc(350 + 350);tab-size:calc(2 * 2);line-height:calc(1 + 0.5)",
    );
    assert_eq!(style.flex.order, 3, "2.5 rounds half towards +infinity");
    assert_eq!(
        style.z_index,
        Some(-2),
        "-2.5 rounds half towards +infinity"
    );
    assert_eq!(style.widows, 1, "widows clamps to its minimum of one");
    assert_eq!(style.orphans, 4);
    assert_eq!(style.flex.grow, 0.0, "flex factors clamp to zero");
    assert_eq!(style.flex.shrink, 1.5);
    assert_eq!(style.font_weight.value(), 700.0);
    assert_eq!(style.tab_size, crate::style::TabSize::Number(4.0));
    assert_eq!(style.line_height.resolve(style.size), Some(15.0));
    let heavy = child("font-weight:calc(2000)");
    assert_eq!(heavy.font_weight.value(), 1000.0, "weights clamp to 1000");
}

#[test]
fn negative_spacing_literals_drop_whole_declarations_but_math_clamps_each_value() {
    let fallback = child(
        "gap:3pt 5pt;gap:-1pt 7pt;border-spacing:4pt 6pt;border-spacing:8pt -2pt;line-height-step:9pt;line-height-step:-3pt;outline:7pt solid;outline-width:-4pt",
    );
    assert_eq!(
        fallback.flex.gap.map(|gap| gap.resolve(100.0)),
        [3.0, 5.0],
        "an invalid gap shorthand must preserve both earlier axes"
    );
    assert_eq!(
        fallback.table.spacing,
        [4.0, 6.0],
        "an invalid second border-spacing value must preserve both earlier axes"
    );
    assert_eq!(
        fallback.line_height_step, 9.0,
        "a negative literal must not abort style computation or replace its fallback"
    );
    assert_eq!(
        fallback.decoration.outline.width, 7.0,
        "negative outline widths must be discarded before computing the outline"
    );

    let calculated = child(
        "gap:calc(1pt - 3pt) calc(2% - 5%);border-spacing:calc(2pt - 8pt) 6pt;line-height-step:calc(1pt - 4pt);outline:solid;outline-width:calc(2pt - 6pt)",
    );
    assert_eq!(
        calculated.flex.gap.map(|gap| gap.resolve(100.0)),
        [0.0; 2],
        "negative length and percentage calculations clamp to the grammar minimum"
    );
    assert_eq!(
        calculated.table.spacing,
        [0.0, 6.0],
        "range clamping applies to each spacing component separately"
    );
    assert_eq!(calculated.line_height_step, 0.0);
    assert_eq!(calculated.decoration.outline.width, 0.0);
}

#[test]
fn typography_percentages_resolve_against_font_sizes() {
    let style = child(
        "font-size:calc(10pt + 50%);line-height:calc(100% + 4pt);letter-spacing:calc(1pt + 0.5pt)",
    );
    assert_eq!(
        style.size, 15.0,
        "font-size percentages use the parent's 10pt"
    );
    assert_eq!(style.line_height.resolve(style.size), Some(19.0));
    assert_eq!(style.letter_spacing, 1.5);
    let clamped = child("font-size:max(5pt, 140%);line-height:calc(2em - 4pt)");
    assert_eq!(clamped.size, 14.0);
    assert_eq!(clamped.line_height.resolve(clamped.size), Some(24.0));
}

#[test]
fn colour_components_accept_numbers_percentages_and_angles() {
    let style = child(
        "color:rgb(calc(255 / 2) 0 0);background-color:hsl(calc(60deg + 0.5 * 120deg) 100% 50%);border-left-color:rgb(calc(20% + 30%) 0% 0% / calc(0.25 * 2))",
    );
    assert_eq!(style.color.rgb, [128, 0, 0]);
    assert_eq!(
        style.decoration.background.rgb,
        [0, 255, 0],
        "120deg hue is green"
    );
    assert_eq!(style.decoration.borders[3].color.rgb, [128, 0, 0]);
    assert_eq!(style.decoration.borders[3].color.alpha, 0.5);
    let dropped = child("color:navy;color:rgb(calc(1pt) 0 0)");
    assert_eq!(
        dropped.color.rgb,
        [0, 0, 128],
        "a length channel is invalid"
    );
}

#[test]
fn substituted_invalid_math_is_invalid_at_computed_value_time() {
    let style = child(
        "--bad:calc(1pt + 1);--good:4pt;margin-left:7pt;margin-left:var(--bad);padding-left:calc(1pt + var(--good) * 2)",
    );
    assert!(
        style.margin[3].is_zero(),
        "an invalid substitution resets margin-left to its initial value: {:?}",
        style.margin[3]
    );
    assert_eq!(style.padding[3], LengthPercentage::Length(9.0));
}
