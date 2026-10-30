//! CSS Color 4 and 5 values checked against the specifications' worked examples.
use crate::dom::{DocumentTree, Selector};
use crate::paint::{AbsoluteColor, ColorSpace};
use crate::resources::FileResolver;
use crate::style::{
    ColorScheme, PrintPreferences, PrintSettings, Style, StyleError, Stylesheet, StylesheetSource,
    colors, units::Relative,
};

/// Worked examples print four or five significant digits.
const EXAMPLE_TOLERANCE: f32 = 0.006;
const UNIT_TOLERANCE: f32 = 0.0006;
const HUE_TOLERANCE: f32 = 0.06;
/// WCAG 2.2 minimum contrast for matching foreground and background system colours.
const AA_CONTRAST: f64 = 4.5;

fn parse(source: &str) -> AbsoluteColor {
    colors::parse(source, &Relative::initial(), &AbsoluteColor::BLACK)
        .unwrap_or_else(|error| panic!("{source} must parse: {error}"))
        .color
}
fn parse_in(source: &str, scheme: ColorScheme) -> AbsoluteColor {
    let relative = Relative {
        color_scheme: scheme,
        ..Relative::initial()
    };
    colors::parse(source, &relative, &AbsoluteColor::BLACK)
        .unwrap_or_else(|error| panic!("{source} must parse: {error}"))
        .color
}
fn rejects(source: &str) {
    assert!(
        colors::parse(source, &Relative::initial(), &AbsoluteColor::BLACK).is_err(),
        "{source} must be invalid"
    );
}
fn assert_color(
    label: &str,
    actual: AbsoluteColor,
    space: ColorSpace,
    expected: [Option<f32>; 3],
    tolerance: f32,
) {
    assert_eq!(actual.space, space, "{label}: space of {actual:?}");
    for (index, (actual_component, expected_component)) in
        actual.components.iter().zip(expected).enumerate()
    {
        match (actual_component, expected_component) {
            (Some(actual_value), Some(expected_value)) => {
                let tolerance = if space.hue_index() == Some(index) {
                    HUE_TOLERANCE
                } else {
                    tolerance
                };
                assert!(
                    (actual_value - expected_value).abs() <= tolerance,
                    "{label}: component {index} is {actual_value}, expected {expected_value} \
                     within {tolerance}\nactual {actual:?}"
                );
            }
            (None, None) => {}
            _ => panic!(
                "{label}: component {index} missing state differs: {actual_component:?} vs \
                 {expected_component:?}\nactual {actual:?}"
            ),
        }
    }
}
fn some(values: [f32; 3]) -> [Option<f32>; 3] {
    values.map(Some)
}
fn assert_alpha(label: &str, actual: AbsoluteColor, expected: f32) {
    let alpha = actual.alpha.expect("alpha present");
    assert!(
        (alpha - expected).abs() <= UNIT_TOLERANCE,
        "{label}: alpha {alpha}, expected {expected}\nactual {actual:?}"
    );
}

fn computed_with(
    css: &str,
    html: &str,
    selector: &str,
    preferences: PrintPreferences,
) -> Result<Style, StyleError> {
    let settings = PrintSettings {
        preferences,
        ..Default::default()
    };
    let sheet = Stylesheet::parse_with_settings(
        &[StylesheetSource {
            text: css.into(),
            base: Default::default(),
        }],
        &[],
        &Default::default(),
        &settings,
    )?;
    let tree = DocumentTree::parse(html).expect("parse test HTML");
    let element = tree
        .select(&Selector::parse(selector).unwrap())
        .next()
        .unwrap();
    let mut ancestors = element
        .ancestors(crate::dom::TreeRelation::Inheritance)
        .collect::<Vec<_>>();
    ancestors.reverse();
    let mut style = Style::default();
    for element in ancestors.into_iter().chain([element]) {
        style = sheet.computed_with_resolver(element, &style, &FileResolver::default())?;
    }
    Ok(style)
}
fn computed(css: &str, preferences: PrintPreferences) -> Style {
    computed_with(
        css,
        "<div id=parent><p id=target>X</p></div>",
        "#target",
        preferences,
    )
    .unwrap_or_else(|error| panic!("{css} must compute: {error}"))
}
fn dark() -> PrintPreferences {
    PrintPreferences {
        color_scheme: ColorScheme::Dark,
        ..Default::default()
    }
}

#[test]
fn predefined_spaces_convert_like_the_specification_examples() {
    // CSS Color 4 §10.9: these are exactly equivalent.
    let violet = parse("#7654CD");
    assert_color(
        "#7654CD as rgb()",
        violet.convert(ColorSpace::Srgb),
        ColorSpace::Srgb,
        some([0.4627, 0.3294, 0.8039]),
        UNIT_TOLERANCE,
    );
    assert_color(
        "#7654CD as lab()",
        violet.convert(ColorSpace::Lab),
        ColorSpace::Lab,
        some([44.36, 36.05, -58.99]),
        EXAMPLE_TOLERANCE,
    );
    assert_color(
        "#7654CD in xyz-d50",
        violet.convert(ColorSpace::XyzD50),
        ColorSpace::XyzD50,
        some([0.2005, 0.14089, 0.4472]),
        UNIT_TOLERANCE,
    );
    assert_color(
        "#7654CD in xyz-d65",
        violet.convert(ColorSpace::XyzD65),
        ColorSpace::XyzD65,
        some([0.21661, 0.14602, 0.59452]),
        UNIT_TOLERANCE,
    );
    assert_color(
        "white in xyz-d50",
        parse("#FFFFFF").convert(ColorSpace::XyzD50),
        ColorSpace::XyzD50,
        some([0.9643, 1.0, 0.8251]),
        UNIT_TOLERANCE,
    );
    // §11.1: one colour in five syntaxes.
    let blue = parse("oklch(65% 0.15 270)");
    for (space, expected, tolerance) in [
        (ColorSpace::Lab, [57.9, 11.4, -53.7], 0.06),
        (ColorSpace::DisplayP3, [0.445, 0.529, 0.891], 0.0006),
        (ColorSpace::Rec2020, [0.522, 0.557, 0.892], 0.0006),
        (
            ColorSpace::Srgb,
            [108.0 / 255.0, 136.0 / 255.0, 234.0 / 255.0],
            0.5 / 255.0,
        ),
    ] {
        assert_color(
            &format!("oklch(65% 0.15 270) in {space:?}"),
            blue.convert(space),
            space,
            some(expected),
            tolerance,
        );
    }
    assert_color(
        "chromatic adaptation lifts xyz-d65 white above 100% Lab lightness",
        parse("color(xyz-d65 1 1 1)").convert(ColorSpace::Lab),
        ColorSpace::Lab,
        some([100.1154, 9.064_489, 5.801_761]),
        0.0006,
    );
    // §10.1: an intense rec2020 lime.
    let lime = parse("color(rec2020 0.42053 0.979780 0.00579)");
    assert_color(
        "rec2020 lime in lch",
        lime.convert(ColorSpace::Lch),
        ColorSpace::Lch,
        some([85.9017, 166.116, 138.207]),
        0.006,
    );
    assert_color(
        "rec2020 lime is out of display-p3 gamut, unclamped",
        lime.convert(ColorSpace::DisplayP3),
        ColorSpace::DisplayP3,
        some([-0.350_289, 1.00707, -0.144_209]),
        UNIT_TOLERANCE,
    );
    assert_color(
        "srgb-linear (§10.3)",
        parse("color(srgb 0.691 0.139 0.259)").convert(ColorSpace::SrgbLinear),
        ColorSpace::SrgbLinear,
        some([0.435, 0.017, 0.055]),
        UNIT_TOLERANCE,
    );
    assert_color(
        "display-p3-linear (§10.5)",
        parse("color(display-p3 0.591 0.123 0.264)").convert(ColorSpace::DisplayP3Linear),
        ColorSpace::DisplayP3Linear,
        some([0.3081, 0.014, 0.0567]),
        UNIT_TOLERANCE,
    );
    assert_eq!(
        parse("color(xyz 0.472 0.372 0.131)").space,
        ColorSpace::XyzD65,
        "xyz computes to xyz-d65 (§15.4)"
    );
}

#[test]
fn every_space_round_trips_through_xyz() {
    let origin = parse("color(display-p3 0.2 0.6 0.9 / 0.5)");
    for space in [
        ColorSpace::Srgb,
        ColorSpace::SrgbLinear,
        ColorSpace::DisplayP3Linear,
        ColorSpace::A98Rgb,
        ColorSpace::ProphotoRgb,
        ColorSpace::Rec2020,
        ColorSpace::XyzD50,
        ColorSpace::XyzD65,
        ColorSpace::Lab,
        ColorSpace::Lch,
        ColorSpace::Oklab,
        ColorSpace::Oklch,
        ColorSpace::Hsl,
        ColorSpace::Hwb,
    ] {
        let back = origin.convert(space).convert(ColorSpace::DisplayP3);
        assert_color(
            &format!("display-p3 through {space:?}"),
            back,
            ColorSpace::DisplayP3,
            some([0.2, 0.6, 0.9]),
            1.0e-4,
        );
        assert_alpha(&format!("alpha through {space:?}"), back, 0.5);
    }
}

#[test]
fn srgb_functions_parse_and_resolve_as_the_specification_requires() {
    assert_color(
        "hwb(150 20% 10%) is rgb(20% 90% 55%) (§8)",
        parse("hwb(150 20% 10%)").convert(ColorSpace::Srgb),
        ColorSpace::Srgb,
        some([0.2, 0.9, 0.55]),
        UNIT_TOLERANCE,
    );
    assert_eq!(
        parse("hsl(38.824 100% 50%)").used().rgb,
        [255, 165, 0],
        "hsl(38.824 100% 50%) computes to rgb(255, 165, 0) (§15.1)"
    );
    assert_eq!(
        parse("hsl(120 50 50)").used(),
        parse("hsl(120deg 50% 50%)").used(),
        "modern hsl() accepts numbers for saturation and lightness"
    );
    assert_eq!(
        parse("hwb(45 40% 80%)")
            .convert(ColorSpace::Srgb)
            .components,
        [Some(1.0 / 3.0); 3],
        "white + black above 100% is the grey white / (white + black) (§8)"
    );
    assert_eq!(
        parse("rgb(calc(100 * 4) 127 calc(20 - 35))").used().rgb,
        [255, 127, 0],
        "rgb() components clamp to [0, 255] (§15.1)"
    );
    assert_eq!(
        parse("hsl(calc(infinity) 100% 50%)").components[0],
        Some(0.0),
        "infinite hues normalise to 0deg (§4.3)"
    );
    assert_eq!(parse("hsl(-540 0 0)").components[0], Some(180.0), "§4.3");
    assert_eq!(
        parse("lab(-10 0 0)").components[0],
        Some(0.0),
        "lab() lightness clamps at parsed-value time (§9.3)"
    );
    assert_eq!(
        parse("lch(50% -10 0)").components[1],
        Some(0.0),
        "negative chroma clamps to zero (§9.3)"
    );
    assert_eq!(
        parse("oklch(150% 0.1 0)").components[0],
        Some(1.0),
        "oklch() lightness clamps to 100% (§9.4)"
    );
    assert_eq!(
        parse("rgb(10 20 30 / 2)").alpha,
        Some(1.0),
        "alpha clamps to [0, 1] (§4.2)"
    );
    assert_eq!(parse("rgba(255, 0, 0, 50%)").alpha, Some(0.5));
    assert_eq!(parse("rgb(100% 0% 0% / 50%)").used().rgb, [255, 0, 0]);
    assert_eq!(parse("#0f08").alpha, Some(0x88 as f32 / 255.0));
    for source in [
        "rgb(10%, 20, 30)",
        "rgb(none, 0, 0)",
        "rgb(1, 2, none)",
        "hsl(120, 50, 50%)",
        "hwb(120, 10%, 10%)",
        "lab(50, 10, 10)",
        "lch(50% 10 10%)",
        "color(profoto-rgb 0.4 0.9 0.2)",
        "rgba(from darkblue 16, 32, b, 0.5)",
        "notacolor",
    ] {
        rejects(source);
    }
}

#[test]
fn missing_components_render_as_zero_and_carry_forward_through_conversion() {
    assert_eq!(
        parse("rgb(none 255 0 / none)").used(),
        crate::paint::Color {
            rgb: [0, 255, 0],
            alpha: 0.0
        },
        "a missing component behaves as zero for rendering (§4.4)"
    );
    // §13.3: lab(50% none none) carries the a/b analogous set to C and H.
    assert_color(
        "lab(50% none none) in lch",
        parse("lab(50% none none)").convert_carrying(ColorSpace::Lch),
        ColorSpace::Lch,
        [Some(50.0), None, None],
        EXAMPLE_TOLERANCE,
    );
    assert_color(
        "rgb(none none none) in oklab",
        parse("rgb(none none none / 50%)").convert_carrying(ColorSpace::Oklab),
        ColorSpace::Oklab,
        [None; 3],
        EXAMPLE_TOLERANCE,
    );
    // The missing hue is analogous; the missing blue component is not.
    assert_color(
        "lch(50% 0.02 none) in oklch",
        parse("lch(50% 0.02 none)").convert_carrying(ColorSpace::Oklch),
        ColorSpace::Oklch,
        [Some(0.568_97), Some(0.0001), None],
        UNIT_TOLERANCE,
    );
    assert_color(
        "color(display-p3 0.7 0.5 none) in oklch",
        parse("color(display-p3 0.7 0.5 none)").convert_carrying(ColorSpace::Oklch),
        ColorSpace::Oklch,
        some([0.636_12, 0.1522, 78.748]),
        UNIT_TOLERANCE,
    );
    assert_color(
        "white has a powerless, missing hue in oklch (§4.4.1)",
        parse("white").convert(ColorSpace::Oklch),
        ColorSpace::Oklch,
        [Some(1.0), Some(0.0), None],
        UNIT_TOLERANCE,
    );
}

#[test]
fn color_mix_matches_the_css_color_5_worked_examples() {
    assert_color(
        "40% peru with palegoldenrod in lch (§3.3)",
        parse("color-mix(in lch, peru 40%, palegoldenrod)"),
        ColorSpace::Lch,
        some([79.7256, 40.448, 84.771]),
        0.01,
    );
    let teal_olive = parse("color-mix(in lch, teal 65%, olive)");
    assert_color(
        "65% teal with olive in lch",
        teal_olive,
        ColorSpace::Lch,
        some([49.4429, 40.4830, 162.5452]),
        0.01,
    );
    assert_color(
        "the teal and olive mix in sRGB",
        teal_olive.convert(ColorSpace::Srgb),
        ColorSpace::Srgb,
        some([0.077_377, 0.525_730, 0.373_213]),
        UNIT_TOLERANCE,
    );
    assert_color(
        "teal, olive and blue in thirds",
        parse("color-mix(in oklab, teal, olive, blue)"),
        ColorSpace::Oklab,
        some([0.5253, -0.0550, -0.0720]),
        UNIT_TOLERANCE,
    );
    let premultiplied =
        parse("color-mix(in srgb, rgb(100% 0% 0% / 0.7) 25%, rgb(0% 100% 0% / 0.2))");
    assert_color(
        "premultiplied mix (§3.5)",
        premultiplied,
        ColorSpace::Srgb,
        some([0.538_46, 0.461_54, 0.0]),
        UNIT_TOLERANCE,
    );
    assert_alpha("premultiplied mix alpha", premultiplied, 0.325);
    let multiplied =
        parse("color-mix(in srgb, rgb(100% 0% 0% / 0.7) 20%, rgb(0% 100% 0% / 0.2) 60%)");
    assert_color(
        "scaled percentages keep the colour",
        multiplied,
        ColorSpace::Srgb,
        some([0.538_46, 0.461_54, 0.0]),
        UNIT_TOLERANCE,
    );
    assert_alpha("an 80% total multiplies alpha by 0.8", multiplied, 0.26);
    assert_eq!(
        parse("color-mix(firebrick, goldenrod)"),
        parse("color-mix(in oklab, firebrick, goldenrod)"),
        "the default mixing space is Oklab (§3.1)"
    );
}

#[test]
fn mix_percentages_normalize_with_an_alpha_multiplier_for_shortfalls() {
    let even = parse("color-mix(in lch, purple 50%, plum 50%)");
    assert_color(
        "a 50-50 lch mix of purple and plum (§3.2)",
        even,
        ColorSpace::Lch,
        some([51.51, 52.21, 325.8]),
        0.01,
    );
    for source in [
        "color-mix(in lch, purple 50%, plum)",
        "color-mix(in lch, purple, plum 50%)",
        "color-mix(in lch, purple, plum)",
        "color-mix(in lch, purple 80%, plum 80%)",
        "color-mix(in lch, 50% purple, plum)",
    ] {
        let mixed = parse(source);
        assert_color(source, mixed, ColorSpace::Lch, even.components, 1.0e-4);
        assert_alpha(source, mixed, 1.0);
    }
    let thin = parse("color-mix(in lch, purple 30%, plum 30%)");
    assert_color("30% each", thin, ColorSpace::Lch, even.components, 1.0e-4);
    assert_alpha("a 60% total leaves alpha 0.6", thin, 0.6);
    let zero = parse("color-mix(in oklch, teal 0%, olive 0%)");
    assert_color(
        "zero percentages mix 50-50 (§3.3)",
        zero,
        ColorSpace::Oklch,
        some([0.5619, 0.109_65, 152.3]),
        0.001,
    );
    assert_alpha("zero percentages are fully transparent", zero, 0.0);
    assert_alpha(
        "one item at 25% is a quarter opaque",
        parse("color-mix(in srgb, red 25%)"),
        0.25,
    );
    for source in [
        "color-mix(in srgb, red -10%, blue)",
        "color-mix(in srgb, red 110%, blue)",
        "color-mix(in srgb longer hue, red, blue)",
        "color-mix(in --profile, red, blue)",
        "color-mix(in srgb red, blue)",
        "color-mix(in srgb, red 10% 20%, blue)",
    ] {
        rejects(source);
    }
}

#[test]
fn hue_interpolation_methods_take_the_specified_arcs() {
    for (method, from, to, hue) in [
        ("shorter", "oklch(0.6 0.24 30)", "oklch(0.8 0.15 90)", 60.0),
        ("longer", "oklch(0.6 0.24 30)", "oklch(0.8 0.15 90)", 240.0),
        (
            "increasing",
            "oklch(0.5 0.1 30)",
            "oklch(0.7 0.1 190)",
            110.0,
        ),
        (
            "increasing",
            "oklch(0.5 0.1 30)",
            "oklch(0.7 0.1 230)",
            130.0,
        ),
        (
            "decreasing",
            "oklch(0.5 0.1 30)",
            "oklch(0.7 0.1 190)",
            290.0,
        ),
        (
            "decreasing",
            "oklch(0.5 0.1 30)",
            "oklch(0.7 0.1 230)",
            310.0,
        ),
    ] {
        let source = format!("color-mix(in oklch {method} hue, {from}, {to})");
        let mixed = parse(&source);
        assert_eq!(
            mixed.components[2],
            Some(hue),
            "{source} midpoint hue (CSS Color 4 §13.5)"
        );
    }
    assert_eq!(
        parse("color-mix(in oklch, oklch(78.3% 0.108 326.5), oklch(39.2% 0.4 none))").components[2],
        Some(326.5),
        "a missing hue takes the other colour's hue (§13.3)"
    );
    let mixed =
        parse("color-mix(in oklch, oklch(0.783 0.108 326.5 / 0.5), oklch(0.392 0.4 0 / none))");
    assert_alpha("a missing alpha takes the other colour's alpha", mixed, 0.5);
    let green = parse("color-mix(in hsl, white 30%, green 70%)");
    assert_eq!(
        green.components[0],
        Some(120.0),
        "white's powerless hue is missing, so green's hue is used (§4.4): {green:?}"
    );
}

#[test]
fn premultiplied_interpolation_matches_the_lab_and_lch_examples() {
    let lab =
        parse("color-mix(in lab, rgb(76% 62% 03% / 0.4), color(display-p3 0.84 0.19 0.72 / 0.6))");
    assert_color(
        "lab midpoint (CSS Color 4 §13.4)",
        lab,
        ColorSpace::Lab,
        some([58.873, 51.552, 7.108]),
        0.02,
    );
    assert_alpha("lab midpoint alpha", lab, 0.5);
    let lch =
        parse("color-mix(in lch, rgb(76% 62% 03% / 0.4), color(display-p3 0.84 0.19 0.72 / 0.6))");
    assert_color(
        "lch midpoint along the shorter arc",
        lch,
        ColorSpace::Lch,
        some([58.873, 81.126, 31.82]),
        0.02,
    );
}

#[test]
fn relative_colors_match_the_css_color_5_examples() {
    let examples: [(&str, ColorSpace, [f32; 3], f32); 9] = [
        (
            "rgb(from indianred 255 g b)",
            ColorSpace::Srgb,
            [1.0, 92.0 / 255.0, 92.0 / 255.0],
            UNIT_TOLERANCE,
        ),
        (
            "rgb(from darkblue 16 32 b / 0.5)",
            ColorSpace::Srgb,
            [16.0 / 255.0, 32.0 / 255.0, 139.0 / 255.0],
            UNIT_TOLERANCE,
        ),
        (
            "hsl(from lightseagreen calc(h + 180) s l)",
            ColorSpace::Hsl,
            [356.712, 69.524, 41.176],
            0.01,
        ),
        (
            "lch(from peru calc(l * 0.8) c h)",
            ColorSpace::Lch,
            [49.80256, 54.0114, 63.6769],
            0.006,
        ),
        (
            "lab(from orchid l 0 0)",
            ColorSpace::Lab,
            [62.753, 0.0, 0.0],
            0.006,
        ),
        (
            "oklab(from oklab(54.3% -22.5% -5%) calc(1.0 - l) calc(a * 0.8) b)",
            ColorSpace::Oklab,
            [0.457, -0.072, -0.02],
            UNIT_TOLERANCE,
        ),
        (
            "oklch(from oklch(52.6% 0.115 44.6deg) l c calc(h + 90))",
            ColorSpace::Oklch,
            [0.526, 0.115, 134.6],
            UNIT_TOLERANCE,
        ),
        (
            "color(from color(display-p3 1 1 0) srgb r g calc(b + 0.5))",
            ColorSpace::Srgb,
            [1.0, 1.0, 0.1537],
            UNIT_TOLERANCE,
        ),
        // The §5.1 example prints z = 0.044 for the origin, which the §19 sample code does
        // not produce; these halves follow the sample code.
        (
            "color(from color(display-p3 0.7 0.5 0.1) xyz-d65 calc(x/2) calc(y/2) calc(z/2))",
            ColorSpace::XyzD65,
            [0.138_414, 0.125_717, 0.010_06],
            UNIT_TOLERANCE,
        ),
    ];
    for (source, space, expected, tolerance) in examples {
        assert_color(source, parse(source), space, some(expected), tolerance);
    }
    assert_alpha(
        "the origin alpha is the default",
        parse("rgb(from rgb(0 0 0 / 30%) r g b)"),
        0.3,
    );
    assert_alpha(
        "relative alpha clamps",
        parse("oklch(from oklch(78% 0.06 75 / 0.7) l c h / calc(alpha * 2))"),
        1.0,
    );
    let alpha_channels = parse("color(from color(srgb 0 0 0 / 60%) srgb alpha 0.6 0.6 / 0.9)");
    assert_color(
        "alpha is a number channel",
        alpha_channels,
        ColorSpace::Srgb,
        some([0.6; 3]),
        UNIT_TOLERANCE,
    );
    assert_color(
        "alpha keeps its 0-1 scale in rgb()",
        parse("rgb(from rgb(0 0 0 / 60%) alpha 153 153 / 0.9)"),
        ColorSpace::Srgb,
        some([0.6 / 255.0, 0.6, 0.6]),
        UNIT_TOLERANCE,
    );
    let half = parse("alpha(from oklch(60% 0.25 315 / 0.8) / calc(alpha * 0.5))");
    assert_color(
        "alpha() keeps the origin's space and components",
        half,
        ColorSpace::Oklch,
        some([0.6, 0.25, 315.0]),
        UNIT_TOLERANCE,
    );
    assert_alpha("alpha() halves alpha", half, 0.4);
    assert_eq!(
        parse("oklch(from hsl(none 3% 50%) calc(l * 0.8) c h)").components[2],
        None,
        "the origin's missing hue is carried forward (CSS Color 5 §4.2)"
    );
    assert!(
        !parse("rgb(from red r g b)").legacy,
        "relative colours interpolate as non-legacy colours"
    );
}

#[test]
fn custom_color_spaces_without_a_profile_are_invalid_opaque_black() {
    let invalid = parse("color(--swopc 0.0134 0.8078 0.7451 0.3019)");
    assert_eq!(
        invalid.used(),
        crate::paint::Color::BLACK,
        "the pinned grammar has no @color-profile, so the colour space does not exist \
         (CSS Color 4 §10.1)"
    );
    rejects("color(from red --swopc r g b)");
    rejects("color(--swopc)");
}

#[test]
fn interpolation_in_a_custom_color_space_invalidates_the_declaration() {
    use crate::style::{declaration, grammar::Property};
    for (property, custom, predefined) in [
        (
            Property::FontPalette,
            "palette-mix(in --sample, normal, light 30%)",
            "palette-mix(in lch longer hue, normal, light 30%)",
        ),
        (
            Property::BackgroundImage,
            "none, linear-gradient(in --sample, red, blue)",
            "none, linear-gradient(in oklab, red, blue)",
        ),
        (
            Property::Color,
            "color-mix(in --sample, red, blue)",
            "color-mix(in srgb, red, blue)",
        ),
    ] {
        let parsed = |source| {
            declaration::value(property, source, &mut 0)
                .unwrap_or_else(|error| panic!("{}: {source}: {error}", property.name()))
        };
        assert!(
            parsed(custom).is_none(),
            "{}: {custom} interpolates in a space without an @color-profile, so it must be \
             invalid (CSS Color 5 §9.1)",
            property.name()
        );
        assert!(
            parsed(predefined).is_some(),
            "{}: {predefined} interpolates in a predefined space and must stay valid",
            property.name()
        );
    }
}

#[test]
fn gamut_mapping_reduces_oklch_chroma_to_the_srgb_boundary() {
    for source in [
        "oklch(70% 0.4 30)",
        "color(display-p3 1 0 0)",
        "color(rec2020 0.42053 0.979780 0.00579)",
        "lch(60% 90 200)",
        "oklch(50% 0.3 264)",
    ] {
        let color = parse(source);
        let mapped = color.mapped_srgb();
        assert!(
            mapped.iter().all(|channel| (0.0..=1.0).contains(channel)),
            "{source} maps into sRGB: {mapped:?}"
        );
        let original = color
            .convert(ColorSpace::Oklch)
            .components
            .map(Option::unwrap);
        let result = AbsoluteColor::srgb(mapped.map(|channel| channel as f32), 1.0)
            .convert(ColorSpace::Oklch)
            .components
            .map(Option::unwrap);
        assert!(
            result[1] < original[1],
            "{source}: chroma {} must drop below {}",
            result[1],
            original[1]
        );
        assert!(
            (result[0] - original[0]).abs() < 0.02,
            "{source}: lightness {} stays near {} (one JND, §14.2.1)",
            result[0],
            original[0]
        );
        let hue_shift = (result[2] - original[2] + 180.0).rem_euclid(360.0) - 180.0;
        assert!(
            hue_shift.abs() < 3.0,
            "{source}: hue {} stays near {}",
            result[2],
            original[2]
        );
        assert!(
            mapped
                .iter()
                .any(|channel| *channel <= 1.0e-6 || *channel >= 1.0 - 1.0e-6),
            "{source}: the result lies on the gamut boundary: {mapped:?}"
        );
    }
    assert_eq!(
        parse("oklch(100% 0.3 120)").used().rgb,
        [255; 3],
        "lightness at 100% maps to white"
    );
    assert_eq!(
        parse("lab(0% 50 50)").used().rgb,
        [0; 3],
        "lightness at 0% maps to black"
    );
    assert_eq!(
        parse("color(display-p3 0.5 0.5 0.5)").used(),
        parse("color(srgb 0.5 0.5 0.5)").used(),
        "in-gamut colours are unchanged (relative colorimetric)"
    );
}

fn relative_luminance(color: AbsoluteColor) -> f64 {
    let [red, green, blue] = color
        .convert(ColorSpace::SrgbLinear)
        .components
        .map(|value| f64::from(value.unwrap()));
    0.2126 * red + 0.7152 * green + 0.0722 * blue
}
fn contrast(first: AbsoluteColor, second: AbsoluteColor) -> f64 {
    let [lighter, darker] = {
        let (a, b) = (relative_luminance(first), relative_luminance(second));
        [a.max(b), a.min(b)]
    };
    (lighter + 0.05) / (darker + 0.05)
}

#[test]
fn system_color_pairs_are_legible_in_both_schemes() {
    for scheme in [ColorScheme::Light, ColorScheme::Dark] {
        for (background, foreground) in [
            ("Canvas", "CanvasText"),
            ("Canvas", "LinkText"),
            ("Canvas", "VisitedText"),
            ("Canvas", "ActiveText"),
            ("Canvas", "GrayText"),
            ("ButtonFace", "ButtonText"),
            ("Field", "FieldText"),
            ("Mark", "MarkText"),
            ("Highlight", "HighlightText"),
            ("SelectedItem", "SelectedItemText"),
            ("AccentColor", "AccentColorText"),
        ] {
            let ratio = contrast(parse_in(background, scheme), parse_in(foreground, scheme));
            assert!(
                ratio >= AA_CONTRAST,
                "{scheme:?} {foreground} on {background}: contrast {ratio:.2} below WCAG AA \
                 (CSS Color 4 §6.2)"
            );
        }
    }
    assert_eq!(parse_in("Canvas", ColorScheme::Light).used().rgb, [255; 3]);
    assert_eq!(
        parse_in("CanvasText", ColorScheme::Light).used().rgb,
        [0; 3]
    );
    assert!(
        relative_luminance(parse_in("Canvas", ColorScheme::Dark)) < 0.05,
        "the dark Canvas is dark"
    );
    for (deprecated, current) in [
        ("InfoText", "CanvasText"),
        ("ThreeDFace", "ButtonFace"),
        ("WindowFrame", "ButtonBorder"),
        ("InactiveCaptionText", "GrayText"),
    ] {
        for scheme in [ColorScheme::Light, ColorScheme::Dark] {
            assert_eq!(
                parse_in(deprecated, scheme),
                parse_in(current, scheme),
                "{deprecated} computes to {current} (Appendix A)"
            );
        }
    }
    assert_eq!(
        parse_in("light-dark(red, blue)", ColorScheme::Light)
            .used()
            .rgb,
        [255, 0, 0]
    );
    assert_eq!(
        parse_in("light-dark(red, blue)", ColorScheme::Dark)
            .used()
            .rgb,
        [0, 0, 255]
    );
}

#[test]
fn color_scheme_negotiates_the_used_scheme_for_light_dark_and_system_colours() {
    let white = [u8::MAX; 3];
    let dark_text = computed(
        "#target{color-scheme:light dark;color:canvastext;background-color:light-dark(red,blue)}",
        dark(),
    );
    assert_eq!(dark_text.color.rgb, white, "a listed preferred dark scheme");
    assert_eq!(dark_text.decoration.background.rgb, [0, 0, 255]);
    let first_supported = computed(
        "#target{color-scheme:dark;background-color:light-dark(red,blue)}",
        PrintPreferences::default(),
    );
    assert_eq!(
        first_supported.decoration.background.rgb,
        [0, 0, 255],
        "the first supported listed scheme applies when the preference is unlisted"
    );
    let normal = computed("#target{background-color:light-dark(red,blue)}", dark());
    assert_eq!(
        normal.decoration.background.rgb,
        [255, 0, 0],
        "normal without page schemes uses the light default even when dark is preferred"
    );
    let custom = computed(
        "#target{color-scheme:sepia only;background-color:light-dark(red,blue)}",
        dark(),
    );
    assert_eq!(custom.decoration.background.rgb, [255, 0, 0]);
    let inherited = computed(
        "#parent{color-scheme:dark;color:canvastext}#target{background-color:Canvas}",
        PrintPreferences::default(),
    );
    assert_eq!(
        inherited.color.rgb, white,
        "color inherits its computed value"
    );
    assert_ne!(
        inherited.decoration.background.rgb, white,
        "color-scheme inherits, so Canvas is dark"
    );
    let root = computed_with(
        ":root{color-scheme:dark}",
        "<p id=target>X</p>",
        "#target",
        PrintPreferences::default(),
    )
    .unwrap();
    assert_eq!(
        root.color.rgb, white,
        "the root's initial canvastext follows its own colour scheme"
    );
}

#[test]
fn currentcolor_in_mixes_and_relative_colours_uses_the_computed_colour() {
    let style = computed(
        "#parent{color:oklch(60% 0.3 30)}#target{color:color-mix(in oklch, currentcolor, oklch(60% 0.3 250));\
         background-color:oklch(from currentcolor l 0.1 h)}",
        PrintPreferences::default(),
    );
    assert_color(
        "color-mix with the inherited currentcolor keeps its oklch value",
        style.computed_color,
        ColorSpace::Oklch,
        [Some(0.6), Some(0.3), Some(320.0)],
        UNIT_TOLERANCE,
    );
    let expected = parse("oklch(60% 0.1 320)").used();
    assert_eq!(
        style.decoration.background, expected,
        "currentcolor in other properties is the element's own computed colour"
    );
}
