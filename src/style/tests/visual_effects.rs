use crate::dom::{DocumentTree, Selector};
use crate::paint::{BlendMode, BorderStyle, LengthPercentage, OutlineStyle, PositionComponent};
use crate::resources::FileResolver;
use crate::style::{
    Rotation, Style, StyleError, Stylesheet, StylesheetSource, TransformBox, TransformStyle,
};

const DOCUMENT: &str = "<div id=parent><p id=child>Text</p></div>";
const PX: f32 = 0.75;
const FONT_SIZE_PT: f32 = 20.0;
const TOLERANCE: f32 = 1.0e-5;

/// The parent and child styles; the parent has a 20pt font for `em` resolution.
fn styles(css: &str) -> Result<(Style, Style), StyleError> {
    let sheet = Stylesheet::parse(&[StylesheetSource {
        text: format!("#parent{{font-size:{FONT_SIZE_PT}pt}}{css}"),
        base: Default::default(),
    }])?;
    let html = DocumentTree::parse(DOCUMENT).expect("parse test HTML");
    let select = |id: &str| html.select(&Selector::parse(id).unwrap()).next().unwrap();
    let parent = sheet.computed_with_resolver(
        select("#parent"),
        &Style::default(),
        &FileResolver::default(),
    )?;
    let child =
        sheet.computed_with_resolver(select("#child"), &parent, &FileResolver::default())?;
    Ok((parent, child))
}

fn parent(css: &str) -> Style {
    styles(&format!("#parent{{{css}}}"))
        .unwrap_or_else(|error| panic!("`{css}` must compute: {error}"))
        .0
}

fn rotation(css: &str) -> Rotation {
    parent(css)
        .rotate
        .unwrap_or_else(|| panic!("`{css}` must compute a rotation"))
}

#[test]
fn individual_transforms_compute_absolute_lengths_axes_and_default_factors() {
    let style = parent("translate:1em 10%");
    assert_eq!(
        style.translate,
        Some([
            LengthPercentage::Length(FONT_SIZE_PT),
            LengthPercentage::Percentage(0.1),
            LengthPercentage::ZERO,
        ]),
        "translate computes em to points, keeps percentages and defaults z to 0"
    );
    assert_eq!(
        parent("translate:1px 2px 3px").translate,
        Some([1.0, 2.0, 3.0].map(|px| LengthPercentage::Length(px * PX)))
    );
    assert_eq!(parent("translate:none").translate, None);
    for (css, axis) in [
        ("rotate:90deg", [0.0, 0.0, 1.0]),
        ("rotate:x 90deg", [1.0, 0.0, 0.0]),
        ("rotate:90deg y", [0.0, 1.0, 0.0]),
        ("rotate:z 90deg", [0.0, 0.0, 1.0]),
        ("rotate:1 2 3 90deg", [1.0, 2.0, 3.0]),
    ] {
        let Rotation {
            axis: actual,
            angle,
        } = rotation(css);
        assert_eq!(
            actual, axis,
            "{css}: the axis keyword or vector, in either order"
        );
        assert!(
            (angle - std::f32::consts::FRAC_PI_2).abs() < TOLERANCE,
            "{css}: the angle is radians, got {angle}"
        );
    }
    assert!(
        (rotation("rotate:0.5turn").angle - std::f32::consts::PI).abs() < TOLERANCE,
        "turns convert to radians"
    );
    for (css, factors) in [
        ("scale:2", [2.0, 2.0, 1.0]),
        ("scale:50% 3", [0.5, 3.0, 1.0]),
        ("scale:1 2 3", [1.0, 2.0, 3.0]),
    ] {
        assert_eq!(
            parent(css).scale,
            Some(factors),
            "{css}: y defaults to x, z to one, percentages are fractions"
        );
    }
    for (css, rejected) in [
        ("rotate:1 2 90deg", "two axis numbers"),
        ("scale:2 3 4 5", "four factors"),
    ] {
        assert_eq!(
            parent(&format!("rotate:none;scale:none;{css}")),
            parent("rotate:none;scale:none"),
            "{css} has {rejected}, so the declaration is invalid and dropped"
        );
    }
}

#[test]
fn transform_context_properties_compute_and_do_not_inherit() {
    let (parent, child) = styles(
        "#parent{transform-box:content-box;transform-style:preserve-3d;perspective:2em;\
         perspective-origin:right 10px bottom 5px;backface-visibility:hidden}",
    )
    .expect("transform context properties must compute");
    assert_eq!(parent.transform_box, TransformBox::ContentBox);
    assert_eq!(parent.transform_style, TransformStyle::Preserve3d);
    assert_eq!(
        parent.perspective,
        Some(2.0 * FONT_SIZE_PT),
        "perspective is an absolute length"
    );
    assert_eq!(
        parent.perspective_origin,
        [
            PositionComponent {
                from_end: true,
                offset: LengthPercentage::Length(10.0 * PX),
            },
            PositionComponent {
                from_end: true,
                offset: LengthPercentage::Length(5.0 * PX),
            },
        ],
        "perspective-origin keeps its edge offsets like background-position"
    );
    assert!(parent.backface_hidden);
    assert!(parent.has_transform_effects() && parent.preserves_3d());
    assert_eq!(child.transform_box, TransformBox::default());
    assert_eq!(child.transform_style, TransformStyle::Flat);
    assert_eq!(child.perspective, None);
    assert!(!child.backface_hidden, "none of these properties inherit");
    let grouped = styles("#parent{transform-style:preserve-3d;overflow:hidden}")
        .expect("grouped preserve-3d must compute")
        .0;
    assert!(
        grouped.has_transform_effects() && !grouped.preserves_3d(),
        "overflow:hidden is a grouping property: the computed preserve-3d still contains \
         descendants, but its used value is flat"
    );
    assert_eq!(
        styles("#parent{perspective:0}")
            .expect("zero perspective")
            .0
            .perspective,
        Some(0.0),
        "clamping to 1px happens only when rendering"
    );
}

#[test]
fn outline_longhands_compute_per_css_ui_4() {
    let outline = |css: &str| parent(&format!("color:#00f;{css}")).decoration.outline;
    let solid = outline("outline:thick solid;outline-offset:-1em");
    assert_eq!(
        (solid.style, solid.width, solid.offset, solid.color.rgb),
        (
            OutlineStyle::Line(BorderStyle::Solid),
            2.8125,
            -FONT_SIZE_PT,
            [0, 0, 0xff]
        ),
        "thick is 3.75px, the offset an absolute length, and auto colour currentcolor"
    );
    assert_eq!(
        outline("outline-width:5pt").computed_width(),
        0.0,
        "the computed width is zero while the style is none"
    );
    assert_eq!(
        outline("outline-style:solid").width,
        crate::paint::DEFAULT_BORDER_WIDTH,
        "an undeclared width is the initial medium once the style is not none"
    );
    let auto = outline("outline:auto");
    assert_eq!(
        (auto.style, auto.color.rgb),
        (OutlineStyle::Auto, [0; 3]),
        "a lone auto sets both style and colour to auto; auto colour with auto style is the \
         accent colour"
    );
    assert_eq!(
        outline("outline:auto 1pt").used_width(),
        crate::paint::DEFAULT_BORDER_WIDTH,
        "outline-width is ignored when outline-style is auto"
    );
    assert_eq!(
        outline("outline:1pt dotted red;outline-color:auto")
            .color
            .rgb,
        [0, 0, 0xff],
        "auto with a line style is currentcolor"
    );
    assert_eq!(
        outline("outline:1pt dotted red;outline-style:hidden").style,
        OutlineStyle::Line(BorderStyle::Dotted),
        "hidden is not an outline style, so the declaration is dropped"
    );
    let (_, child) =
        styles("#parent{outline:2pt solid red;outline-offset:2pt}").expect("outlines must compute");
    assert_eq!(
        child.decoration.outline.style,
        OutlineStyle::default(),
        "outlines do not inherit"
    );
}

#[test]
fn blending_and_isolation_compute_as_specified_without_inheriting() {
    let (parent, child) = styles(
        "#parent{mix-blend-mode:color-dodge;isolation:isolate;\
         background-blend-mode:multiply,luminosity}",
    )
    .expect("compositing properties must compute");
    assert_eq!(parent.mix_blend_mode, BlendMode::ColorDodge);
    assert!(parent.isolate && parent.is_grouped());
    assert_eq!(
        parent.decoration.backgrounds.blend_modes,
        [BlendMode::Multiply, BlendMode::Luminosity],
        "one blend mode per listed background layer"
    );
    assert_eq!(child.mix_blend_mode, BlendMode::Normal);
    assert!(!child.isolate && !child.is_grouped());
    assert_eq!(
        child.decoration.backgrounds.blend_modes,
        [BlendMode::Normal]
    );
    for mode in ["plus-lighter", "plus-darker"] {
        let error = styles(&format!("#parent{{mix-blend-mode:{mode}}}"))
            .expect_err("additive compositing has no PDF equivalent");
        assert!(
            matches!(&error, StyleError::Unsupported(message)
                if message.starts_with(&format!("mix-blend-mode: {mode} (additive compositing"))),
            "{mode} must stay an error naming why: {error}"
        );
    }
}
