use super::common::local_options;
use crate::{DisplayItem, render_html};

fn math(body: &str) -> std::sync::Arc<crate::layout::ViewportPaint> {
    let font =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fonts/MathTest.ttf");
    let output = render_html(
        &format!("<style>@font-face{{font-family:MathTest;src:url('{}')}}math{{font-family:DejaVu Serif;font-size:20pt}}</style>{body}", font.display()),
        &local_options(),
    )
    .unwrap_or_else(|error| panic!("MathML fixture must render: {error}\n{body}"));
    output
        .layout
        .unwrap()
        .display_lists
        .iter()
        .flatten()
        .find_map(|item| {
            let DisplayItem::Image(paint) = item else {
                return None;
            };
            let crate::images::Image::Viewport(view) = paint.image.as_ref() else {
                return None;
            };
            Some(view.clone())
        })
        .expect("MathML must produce native selectable text in its atomic box")
}

#[test]
fn fraction_centres_children_and_keeps_real_math_font_clearances() {
    let view = math("<math><mfrac><mn>1</mn><mn>234</mn></mfrac></math>");
    let numerator = view.lines.iter().find(|line| line.text == "1").unwrap();
    let denominator = view.lines.iter().find(|line| line.text == "234").unwrap();
    assert!(
        numerator.baseline < denominator.baseline,
        "numerator must be above denominator: {} vs {}",
        numerator.baseline,
        denominator.baseline
    );
    assert!(
        (numerator.x + numerator.width / 2.0 - denominator.x - denominator.width / 2.0).abs()
            < 0.01,
        "fraction children must share a horizontal centre"
    );
    assert_eq!(
        numerator.font_size, 16.0,
        "compact fraction scripts use DejaVu MATH's 80 percent sizing, not a fixed scaling factor"
    );
    assert!(
        view.items
            .iter()
            .any(|item| matches!(item, DisplayItem::Box(_))),
        "nonzero fraction bar must be vector paint"
    );
}

#[test]
fn zero_rule_fraction_stacks_without_painting_a_bar() {
    let view = math("<math><mfrac linethickness='0'><mn>12</mn><mn>34</mn></mfrac></math>");
    assert!(
        !view
            .items
            .iter()
            .any(|item| matches!(item, DisplayItem::Box(_))),
        "zero line thickness must not paint a bar"
    );
    assert!(
        view.lines[0].baseline < view.lines[1].baseline,
        "zero-rule fractions must remain stacked"
    );
}

#[test]
fn paired_scripts_are_sized_from_math_table_and_separated_from_the_base() {
    let view = math("<math><msubsup><mn>8</mn><mn>1</mn><mn>2</mn></msubsup></math>");
    let find = |text| view.lines.iter().find(|line| line.text == text).unwrap();
    let (base, sub, sup) = (find("8"), find("1"), find("2"));
    assert!(
        sup.baseline < base.baseline && base.baseline < sub.baseline,
        "script baseline order must be superscript, base, subscript: {}, {}, {}",
        sup.baseline,
        base.baseline,
        sub.baseline
    );
    assert_eq!(
        (sub.font_size, sup.font_size),
        (16.0, 16.0),
        "MATH script sizing must affect both scripts"
    );
    assert!(
        sub.x >= base.x + base.width && sup.x >= base.x + base.width,
        "scripts must follow the base, without overlapping its advance"
    );
}

#[test]
fn unknown_math_content_keeps_an_explicit_error_until_its_layout_exists() {
    let error = render_html(
        "<math><mtable><mtr><mtd><mn>1</mn></mtd></mtr></mtable></math>",
        &local_options(),
    )
    .err()
    .expect("unimplemented table layout must fail");
    assert!(
        error.to_string().contains("mtable"),
        "error must name the unsupported construct: {error}"
    );
}

#[test]
fn spaces_padding_and_phantoms_preserve_dimensions_without_painting_hidden_text() {
    let view = math(
        "<math><mn>1</mn><mspace width='10pt' height='30pt' depth='8pt'/><mphantom><mn>234</mn></mphantom><mn>5</mn></math>",
    );
    let first = view.lines.iter().find(|line| line.text == "1").unwrap();
    let last = view.lines.iter().find(|line| line.text == "5").unwrap();
    assert!(
        last.x > first.x + first.width + 10.0,
        "phantom must reserve its full advance"
    );
    assert!(
        !view.lines.iter().any(|line| line.text == "234"),
        "phantom text must not paint or leak into PDF extraction"
    );
    assert_eq!(
        view.area.height, 38.0,
        "mspace must contribute requested height and depth to the line box"
    );
    let padded = math(
        "<math><mpadded width='60pt' height='40pt' depth='10pt' lspace='7pt' voffset='3pt'><mn>8</mn></mpadded><mn>9</mn></math>",
    );
    let eight = padded.lines.iter().find(|line| line.text == "8").unwrap();
    let nine = padded.lines.iter().find(|line| line.text == "9").unwrap();
    assert_eq!(
        (eight.x, nine.x),
        (7.0, 60.0),
        "padding offsets its inner expression while reserving the requested width"
    );
    assert_eq!(
        eight.baseline + 3.0,
        nine.baseline,
        "positive voffset raises the inner expression"
    );
}

#[test]
fn css_overrides_math_script_size_and_semantics_hides_annotations() {
    let view = math(
        "<math><semantics><msup><mn>8</mn><mn style='font-size:11pt'>2</mn></msup><annotation>do not paint</annotation></semantics></math>",
    );
    assert_eq!(
        view.lines
            .iter()
            .find(|line| line.text == "2")
            .unwrap()
            .font_size,
        11.0,
        "explicit author sizes override the font-size:math UA rule"
    );
    assert!(
        !view
            .lines
            .iter()
            .any(|line| line.text.contains("do not paint")),
        "semantics annotations must not be flattened into visible text"
    );
}

#[test]
fn operator_dictionary_spacing_depends_on_form_and_stretches_fences_from_math_assemblies() {
    let infix = math("<math><mn>1</mn><mo>+</mo><mn>2</mn></math>");
    let prefix = math("<math><mn>1</mn><mo form='prefix'>+</mo><mn>2</mn></math>");
    let position = |view: &crate::layout::ViewportPaint, text: &str| {
        view.lines.iter().find(|line| line.text == text).unwrap().x
    };
    const OPERATOR_MEDIUM_SPACE_EM: f32 = 4.0 / 18.0;
    assert!(
        (position(&infix, "2") - position(&prefix, "2") - 2.0 * OPERATOR_MEDIUM_SPACE_EM * 20.0)
            .abs()
            < 0.01,
        "infix and explicit-prefix plus must use their distinct dictionary spacing"
    );
    let view = math(
        "<math style=\"font-family:MathTest\"><mo>(</mo><mfrac><mfrac><mn>1</mn><mn>2</mn></mfrac><mn>3</mn></mfrac><mo>)</mo></math>",
    );
    let fence = view.lines.iter().find(|line| line.text == "(").unwrap();
    assert!(
        fence.runs[0].glyphs().len() > 1,
        "a tall fence must use the font's MATH glyph assembly, without geometric distortion"
    );
    assert!(
        fence.height
            >= view
                .lines
                .iter()
                .filter(|line| line.text != "(" && line.text != ")")
                .map(|line| line.height)
                .fold(0.0, f32::max),
        "stretched fence must cover the enclosed ink"
    );
}

#[test]
fn radicals_add_native_surd_and_overbar_and_size_root_indices_from_math_table() {
    let view = math(
        "<math style=\"font-family:MathTest\"><mroot><mfrac><mn>1</mn><mn>2</mn></mfrac><mn>3</mn></mroot></math>",
    );
    let surd = view
        .lines
        .iter()
        .find(|line| line.text == "√")
        .expect("radical symbol stays selectable");
    let index = view.lines.iter().find(|line| line.text == "3").unwrap();
    assert_eq!(
        index.font_size, 12.0,
        "root indices use MATH scriptScriptPercentScaleDown, rather than scaling once or twice by the script percentage"
    );
    assert!(
        surd.runs[0].glyphs().len() > 1,
        "fraction radicand must construct a tall surd from the font"
    );
    assert_eq!(
        view.items
            .iter()
            .filter(|item| matches!(item, DisplayItem::Box(_)))
            .count(),
        2,
        "fraction bar and radical overbar must both paint as vector rules"
    );
}

#[test]
fn invalid_bundled_assemblies_keep_the_specified_base_glyph_fallback() {
    let view = math(
        "<math><mo>(</mo><mfrac><mfrac><mn>1</mn><mn>2</mn></mfrac><mn>3</mn></mfrac><mo>)</mo></math>",
    );
    let fence = view.lines.iter().find(|line| line.text == "(").unwrap();
    assert_eq!(
        fence.runs[0].glyphs().len(),
        1,
        "the unmodified bundled font has outer connectors below MinConnectorOverlap, so its assembly must be rejected under Core §5.3.1"
    );
    assert_eq!(
        fence.font_size, 20.0,
        "invalid assemblies must not be replaced by geometric font scaling"
    );
}

#[test]
fn movable_limits_switch_to_scripts_in_compact_math_and_accents_stretch_inline() {
    let compact = math(
        "<math style='font-family:MathTest'><munderover><mo>∑</mo><mn>1</mn><mn>2</mn></munderover></math>",
    );
    let display = math(
        "<math display='block' style='font-family:MathTest'><munderover><mo>∑</mo><mn>1</mn><mn>2</mn></munderover></math>",
    );
    let base = |view: &crate::layout::ViewportPaint| {
        view.lines
            .iter()
            .find(|line| line.text == "∑")
            .unwrap()
            .clone()
    };
    let lower = |view: &crate::layout::ViewportPaint| {
        view.lines
            .iter()
            .find(|line| line.text == "1")
            .unwrap()
            .clone()
    };
    assert!(
        lower(&compact).x >= base(&compact).x + base(&compact).width,
        "movable limits must become scripts in compact math"
    );
    assert!(
        (lower(&display).x + lower(&display).width / 2.0
            - base(&display).x
            - base(&display).width / 2.0)
            .abs()
            < 0.01,
        "display-style limits must centre under the large operator"
    );
    assert_ne!(
        base(&display).runs[0].glyphs()[0].id,
        base(&compact).runs[0].glyphs()[0].id,
        "normal math style must select the MATH display-operator variant"
    );
    let accent = math(
        "<math style='font-family:MathTest'><mover accent='true'><mn>123456</mn><mo>_</mo></mover></math>",
    );
    let bar = accent.lines.iter().find(|line| line.text == "_").unwrap();
    let digits = accent
        .lines
        .iter()
        .find(|line| line.text == "123456")
        .unwrap();
    assert!(
        bar.width >= digits.width,
        "horizontal MATH assembly must cover the accent base"
    );
    assert_eq!(
        bar.font_size, 20.0,
        "accent scripts inherit the base font size rather than reducing it"
    );
}

#[test]
fn automatic_math_italic_uses_original_text_node_boundaries_before_collapsing_spaces() {
    let view = math("<math><mi>a<!-- split -->b</mi><mi>ab</mi><mi> a </mi></math>");
    assert_eq!(
        view.lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>(),
        ["𝑎𝑏", "ab", "a"],
        "Core §4.2 applies math-auto separately to original single-character text nodes; comments and collapsed whitespace must not change eligibility"
    );
}
