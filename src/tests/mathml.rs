use super::common::local_options;
use crate::{DisplayItem, render_html};

fn math_output(body: &str) -> crate::RenderOutput {
    let font =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fonts/MathTest.ttf");
    let metrics_font = font.with_file_name("MathMetrics.otf");
    render_html(
        &format!("<style>@font-face{{font-family:MathTest;src:url('{}')}}@font-face{{font-family:MathMetrics;src:url('{}')}}math{{font-family:DejaVu Serif;font-size:20pt}}</style>{body}", font.display(), metrics_font.display()),
        &local_options(),
    )
    .unwrap_or_else(|error| panic!("MathML fixture must render: {error}\n{body}"))
}

struct MathView {
    lines: Vec<crate::layout::TextLine>,
    items: Vec<DisplayItem>,
    area: crate::graphics::Rect,
    baseline: Option<f32>,
}
fn math(body: &str) -> MathView {
    const MEASURE_BACKGROUND: [u8; 3] = [253, 252, 254];
    const BASELINE_TEXT: &str = "FERRI_BASELINE";
    let trace = math_output(&format!("<style>math{{background:rgb(253,252,254)!important}}body{{white-space:nowrap}}</style>{body}<span>{BASELINE_TEXT}</span>")).layout.unwrap();
    let mut lines = Vec::new();
    let mut items = Vec::new();
    let mut bounds = None;
    let mut baseline = None;
    let mut stack = vec![kurbo::Affine::IDENTITY];
    for item in &trace.display_lists[0] {
        let matrix = *stack.last().unwrap();
        match item {
            DisplayItem::PushTransform(transform) => {
                stack.push(matrix * kurbo::Affine::new(transform.map(f64::from)));
            }
            DisplayItem::PushClip(_)
            | DisplayItem::PushOpacity(_)
            | DisplayItem::PushBlend(_)
            | DisplayItem::PushBackdropRoot
            | DisplayItem::PushFilter(_)
            | DisplayItem::PushMask(_) => stack.push(matrix),
            DisplayItem::Pop => {
                stack.pop().expect("paint pushes and pops balance");
            }
            DisplayItem::Text { line } => {
                let mut line = trace.pages[0][*line].clone();
                let point = matrix * kurbo::Point::new(f64::from(line.x), f64::from(line.baseline));
                line.x = point.x as f32;
                line.baseline = point.y as f32;
                if line.text.ends_with(BASELINE_TEXT) {
                    baseline = Some(line.baseline);
                } else {
                    lines.push(line);
                }
            }
            DisplayItem::Box(paint) => {
                let mut paint = paint.clone();
                let rect = matrix.transform_rect_bbox(kurbo::Rect::new(
                    f64::from(paint.rect.x),
                    f64::from(paint.rect.y),
                    f64::from(paint.rect.x + paint.rect.width),
                    f64::from(paint.rect.y + paint.rect.height),
                ));
                paint.rect = crate::graphics::Rect {
                    x: rect.x0 as f32,
                    y: rect.y0 as f32,
                    width: rect.width() as f32,
                    height: rect.height() as f32,
                };
                if paint.decoration.background.rgb == MEASURE_BACKGROUND {
                    bounds = Some(paint.rect);
                } else if paint.decoration.is_visible() {
                    items.push(DisplayItem::Box(paint));
                }
            }
            _ => {}
        }
    }
    let area =
        bounds.expect("the real MathML principal box must paint its measurable CSS background");
    for line in &mut lines {
        line.x -= area.x;
        line.baseline -= area.y;
    }
    for item in &mut items {
        if let DisplayItem::Box(paint) = item {
            paint.rect.x -= area.x;
            paint.rect.y -= area.y;
        }
    }
    MathView {
        lines,
        items,
        baseline: baseline.map(|baseline| baseline - area.y),
        area: crate::graphics::Rect {
            x: 0.0,
            y: 0.0,
            ..area
        },
    }
}

#[test]
fn token_ink_metrics_ignore_nonbreaking_spaces_and_nowrap_soft_candidates() {
    for tag in ["mi", "mn", "mtext", "ms"] {
        let reference = math(&format!("<math><{tag}>AA</{tag}></math>"));
        for text in ["A A", "A&#160;A"] {
            let actual = math(&format!("<math><{tag}>{text}</{tag}></math>"));
            assert!(
                (actual.area.height - reference.area.height).abs() < 0.01
                    && (actual.baseline.unwrap() - reference.baseline.unwrap()).abs() < 0.01,
                "{tag} {text}: nowrap token text must keep the same A-glyph ink ascent/descent as AA; actual height/baseline={}/{:?}, expected={}/{:?}",
                actual.area.height,
                actual.baseline,
                reference.area.height,
                reference.baseline
            );
        }
    }
}

#[test]
fn forced_unicode_token_lines_use_the_canonical_css_line_baselines() {
    for tag in ["mi", "mn", "mtext", "ms"] {
        for text in ["A&#x2028;A", "A&#x2028;"] {
            let body = |display| {
                format!("<math><{tag} style='line-height:30pt;{display}'>{text}</{tag}></math>")
            };
            let actual = math(&body(""));
            let reference = math(&body("display:block"));
            assert_eq!(
                actual.lines.len(),
                reference.lines.len(),
                "{tag}/{text}: a mandatory Unicode line separator must retain the canonical CSS lines"
            );
            assert!(
                !reference.lines.is_empty(),
                "{tag}/{text}: the CSS control must paint its visible A glyphs"
            );
            assert!(
                (actual.area.height - reference.area.height).abs() < 0.01,
                "{tag}/{text}: forced-break token height {} must match ordinary CSS height {}",
                actual.area.height,
                reference.area.height
            );
            for (actual, expected) in actual.lines.iter().zip(&reference.lines) {
                assert!(
                    (actual.baseline - expected.baseline).abs() < 0.01,
                    "{tag}/{text}: mandatory-break token baseline {} must match ordinary CSS baseline {}",
                    actual.baseline,
                    expected.baseline
                );
            }
        }
    }
}

#[test]
fn css_layout_keeps_core_operator_spacing_through_grouping_boxes() {
    for display in ["block", "inline"] {
        for wrapped in [false, true] {
            let operator = |left, right| {
                let operator = format!(
                    "<mo lspace='{left}em' rspace='{right}em' style='display:{display};width:20pt;height:20pt;line-height:20pt'>+</mo>"
                );
                if wrapped {
                    format!("<mrow style='display:block;width:20pt;height:20pt'>{operator}</mrow>")
                } else {
                    operator
                }
            };
            let actual = math(&format!("<math>{}<mn>Z</mn></math>", operator(2, 3)));
            let expected = math(&format!(
                "<math><mspace width='40pt'/>{}<mspace width='60pt'/><mn>Z</mn></math>",
                operator(0, 0)
            ));
            for text in ["+", "Z"] {
                let found = actual.lines.iter().find(|line| line.text == text).unwrap();
                let control = expected
                    .lines
                    .iter()
                    .find(|line| line.text == text)
                    .unwrap();
                assert!(
                    (found.x - control.x).abs() < 0.01
                        && (found.baseline - control.baseline).abs() < 0.01,
                    "CSS {display}, wrapped={wrapped}: retained core spacing must match ordinary spacer controls without replacing CSS layout for {text}; actual=({},{}), expected=({},{})",
                    found.x,
                    found.baseline,
                    control.x,
                    control.baseline
                );
            }
        }
    }
}

#[test]
fn math_padding_preserves_ink_baselines_until_a_used_border_replaces_them() {
    let glyphs = |edges: &str| {
        let view = math(&format!(
            "<math style='font-family:MathTest'><mo>(</mo><mspace height='40pt' depth='10pt' style='{edges}'/></math>"
        ));
        view.lines
            .iter()
            .find(|line| line.text == "(")
            .unwrap()
            .runs[0]
            .glyphs()
            .iter()
            .map(|glyph| glyph.id)
            .collect::<Vec<_>>()
    };
    let plain = glyphs("");
    assert_eq!(
        glyphs("padding-top:50pt"),
        plain,
        "padding grows the line box but must retain content ink baselines used by neighboring stretchy operators"
    );
    assert!(
        glyphs("padding-top:50pt;border-top:1pt solid black").len() > plain.len(),
        "a positive over-side border replaces the ink baseline with its edge and must grow the fence assembly"
    );
}

#[test]
fn positioned_children_of_native_rows_escape_without_entering_math_arity() {
    let normal = math("<style>body{margin:0}</style><math><mn>A</mn><mn>Z</mn></math>");
    for position in ["fixed", "absolute"] {
        let view = math(&format!(
            "<style>body{{margin:0}}</style><math><mn>A</mn><mrow style='position:{position};left:100pt;top:30pt'><mn>F</mn></mrow><mn>Z</mn></math>"
        ));
        let fixed: Vec<_> = view.lines.iter().filter(|line| line.text == "F").collect();
        assert_eq!(
            fixed.len(),
            1,
            "native rows must retain an escaped {position} child exactly once"
        );
        assert!(
            (fixed[0].x - 100.0).abs() < 0.01,
            "escaped {position} uses its external containing block: x={}",
            fixed[0].x
        );
        let z = view.lines.iter().find(|line| line.text == "Z").unwrap();
        let expected = normal.lines.iter().find(|line| line.text == "Z").unwrap();
        assert!(
            (z.x - expected.x).abs() < 0.01,
            "positioned native child must not consume an in-flow slot"
        );
    }
}

#[test]
fn math_alignment_properties_do_not_move_css_fallback_tracks_or_math_grid_items() {
    let control = math(
        "<math><mrow style='display:grid;width:100pt;height:100pt;grid-template-columns:20pt;grid-template-rows:20pt'><mn>1</mn></mrow><mn>2</mn></math>",
    );
    for (parent, child) in [
        ("align-content:end;justify-content:end", ""),
        ("", "align-self:end;justify-self:end"),
    ] {
        let view = math(&format!(
            "<math><mrow style='display:grid;width:100pt;height:100pt;grid-template-columns:20pt;grid-template-rows:20pt;{parent}'><mn style='{child}'>1</mn></mrow><mn>2</mn></math>"
        ));
        let expected = control.lines.iter().find(|line| line.text == "1").unwrap();
        let found = view.lines.iter().find(|line| line.text == "1").unwrap();
        assert!(
            (found.x - expected.x).abs() < 0.01
                && (found.baseline - expected.baseline).abs() < 0.01,
            "Core ignores alignment properties on MathML, including ordinary CSS fallback: parent={parent}, child={child}, expected=({},{}), found=({},{})",
            expected.x,
            expected.baseline,
            found.x,
            found.baseline
        );
    }
}

#[test]
fn floating_math_children_remain_in_flow_and_keep_fraction_arity() {
    let normal = math("<math><mfrac><mn>1</mn><mn>2</mn></mfrac></math>");
    let floated = math("<math><mfrac><mn style='float:left'>1</mn><mn>2</mn></mfrac></math>");
    for text in ["1", "2"] {
        let expected = normal.lines.iter().find(|line| line.text == text).unwrap();
        let found = floated
            .lines
            .iter()
            .find(|line| line.text == text)
            .unwrap_or_else(|| {
                panic!(
                    "float must not remove direct math child {text} from the fraction: {:?}",
                    floated
                        .lines
                        .iter()
                        .map(|line| &line.text)
                        .collect::<Vec<_>>()
                )
            });
        assert!(
            (found.x - expected.x).abs() < 0.01
                && (found.baseline - expected.baseline).abs() < 0.01,
            "ignored float must preserve native fraction placement for {text}"
        );
    }
}

#[test]
fn percentage_css_widths_on_native_space_and_padding_use_the_parent_content_box() {
    const PARENT_WIDTH: f32 = 100.0;
    for content in [
        "<mn>1</mn><mspace style='width:50%;height:20pt'/><mn>2</mn>",
        "<mpadded style='width:50%'><mn>1</mn></mpadded><mn>2</mn>",
    ] {
        let view = math(&format!(
            "<math style='width:{PARENT_WIDTH}pt'>{content}</math>"
        ));
        let one = view.lines.iter().find(|line| line.text == "1").unwrap();
        let two = view.lines.iter().find(|line| line.text == "2").unwrap();
        let advance = two.x
            - one.x
            - if content.starts_with("<mn>") {
                one.width
            } else {
                0.0
            };
        assert!(
            (advance - PARENT_WIDTH / 2.0).abs() < 0.01,
            "CSS percentage width must reserve half the parent's content width without scaling text: {content}, advance={advance}"
        );
    }
}

#[test]
fn generated_fraction_children_participate_in_native_arity_and_source_order() {
    let view = math(
        "<style>mfrac::before{content:'3'}</style><math><mfrac><mn>1</mn><mn>2</mn></mfrac></math>",
    );
    let texts: Vec<_> = view.lines.iter().map(|line| line.text.as_str()).collect();
    assert_eq!(
        texts,
        ["3", "1", "2"],
        "generated in-flow content is a real fraction child; three children recover to a row"
    );
    let baseline = view.lines[0].baseline;
    assert!(
        view.lines
            .iter()
            .all(|line| (line.baseline - baseline).abs() < 0.01),
        "invalid fraction arity must use row geometry including the generated child: {:?}",
        view.lines
            .iter()
            .map(|line| (&line.text, line.baseline))
            .collect::<Vec<_>>()
    );
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
fn math_table_uses_real_table_layout_and_retains_all_selectable_cell_text() {
    let view = math(
        "<math><mtable><mtr><mtd><mn>1</mn></mtd><mtd><mn>22</mn></mtd></mtr><mtr><mtd><mn>333</mn></mtd><mtd><mn>4</mn></mtd></mtr></mtable></math>",
    );
    let find = |text| {
        view.lines
            .iter()
            .find(|line| line.text == text)
            .unwrap_or_else(|| {
                panic!(
                    "missing real table cell text {text}: {:?}",
                    view.lines.iter().map(|line| &line.text).collect::<Vec<_>>()
                )
            })
    };
    assert!(
        find("1").x < find("22").x && find("333").x < find("4").x,
        "both table rows must retain real column placement"
    );
    assert!(
        find("1").baseline < find("333").baseline && find("22").baseline < find("4").baseline,
        "rows must stack through the ordinary table solver"
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
    let position =
        |view: &MathView, text: &str| view.lines.iter().find(|line| line.text == text).unwrap().x;
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
    let base = |view: &MathView| {
        view.lines
            .iter()
            .find(|line| line.text == "∑")
            .unwrap()
            .clone()
    };
    let lower = |view: &MathView| {
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

#[test]
fn tensor_prescripts_and_postscripts_share_baselines_and_single_pair_matches_subsup() {
    let ordinary = math("<math><msubsup><mn>8</mn><mn>1</mn><mn>2</mn></msubsup></math>");
    let tensor = math("<math><mmultiscripts><mn>8</mn><mn>1</mn><mn>2</mn></mmultiscripts></math>");
    assert_eq!(
        (tensor.area.width, tensor.area.height),
        (ordinary.area.width, ordinary.area.height),
        "Core §3.4.3.1 requires one postscript pair to match msubsup exactly"
    );
    for (actual, expected) in tensor.lines.iter().zip(&ordinary.lines) {
        assert_eq!(
            (actual.text.as_str(), actual.x, actual.baseline),
            (expected.text.as_str(), expected.x, expected.baseline),
            "single-pair tensor geometry differs from msubsup"
        );
    }
    let view = math(
        "<math><mmultiscripts><mn>8</mn><mn>1</mn><mn>2</mn><mprescripts/><mn>3</mn><mn>4</mn></mmultiscripts></math>",
    );
    let find = |text| view.lines.iter().find(|line| line.text == text).unwrap();
    let (base, post_sub, post_sup, pre_sub, pre_sup) =
        (find("8"), find("1"), find("2"), find("3"), find("4"));
    assert!(
        pre_sub.x + pre_sub.width <= base.x && pre_sup.x + pre_sup.width <= base.x,
        "both prescripts must precede the base without overlap"
    );
    assert!(
        post_sub.x >= base.x + base.width && post_sup.x >= base.x + base.width,
        "both postscripts must follow the base"
    );
    assert_eq!(
        (pre_sub.baseline, pre_sup.baseline),
        (post_sub.baseline, post_sup.baseline),
        "tensor script shifts must be shared across pre/post pairs"
    );
    let invalid = math("<math><mmultiscripts><mn>8</mn><mn>1</mn></mmultiscripts></math>");
    assert_eq!(
        invalid.lines[0].baseline, invalid.lines[1].baseline,
        "an odd script count recovers through mrow, without silently dropping a script"
    );
}

#[test]
fn unknown_legacy_math_elements_recover_as_rows_without_inventing_old_notation() {
    let view = math(
        "<math><mfenced open='[' close=']'><mn>1</mn><mn>2</mn></mfenced><mstack><mn>3</mn><mn>4</mn></mstack></math>",
    );
    assert_eq!(
        view.lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>(),
        ["1", "2", "3", "4"],
        "Core §4.1 maps unknown MathML elements to mrow; legacy fence and stack attributes do not invent extra notation"
    );
    assert!(
        view.lines
            .windows(2)
            .all(|pair| pair[0].baseline == pair[1].baseline),
        "unknown grouping elements use row baselines"
    );
    let script = math("<math><mmultiscripts><mn>8</mn><none/><mn>2</mn></mmultiscripts></math>");
    assert_eq!(
        script
            .lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>(),
        ["8", "2"],
        "the old none element is an empty unknown row, usable as a tensor placeholder"
    );
}

#[test]
fn legacy_math_style_hints_accept_only_their_attribute_grammar_and_follow_author_css() {
    let view = math(
        "<math mathcolor='red'><mn mathsize='small'>1</mn><mn mathsize='inherit' mathcolor='initial'>2</mn><mn mathsize='50%'>3</mn><mn mathsize='50%' style='font-size:11pt'>4</mn></math>",
    );
    assert_eq!(
        view.lines
            .iter()
            .map(|line| line.font_size)
            .collect::<Vec<_>>(),
        [20.0, 20.0, 10.0, 11.0],
        "mathsize accepts length-percentage attributes, not CSS global/absolute-size keywords, and author CSS wins over valid hints"
    );
    assert!(
        view.lines.iter().all(|line| line.color.rgb == [255, 0, 0]),
        "invalid mathcolor=initial must inherit the parent's valid red hint"
    );
}

#[test]
fn math_direction_hints_mirror_inline_positions_scripts_and_stretched_fence_glyphs() {
    let view = math(
        "<math dir='rtl'><mn>1</mn><msubsup><mn>8</mn><mn>2</mn><mn>3</mn></msubsup><mn>4</mn></math>",
    );
    let find = |text| view.lines.iter().find(|line| line.text == text).unwrap();
    assert!(
        find("1").x > find("8").x && find("8").x > find("4").x,
        "dir=rtl is a MathML presentation hint affecting row progression"
    );
    assert!(
        find("2").x + find("2").width <= find("8").x
            && find("3").x + find("3").width <= find("8").x,
        "scripts must follow the base in the RTL inline direction"
    );
    let ltr = math(
        "<math style='font-family:MathTest'><mo>(</mo><mfrac><mn>1</mn><mn>2</mn></mfrac><mo>)</mo></math>",
    );
    let rtl = math(
        "<math style='font-family:MathTest;direction:rtl'><mo>(</mo><mfrac><mn>1</mn><mn>2</mn></mfrac><mo>)</mo></math>",
    );
    let glyphs = |view: &MathView, text: &str| {
        view.lines
            .iter()
            .find(|line| line.text == text)
            .unwrap()
            .runs[0]
            .glyphs()
            .iter()
            .map(|glyph| glyph.id)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        glyphs(&rtl, "("),
        glyphs(&ltr, ")"),
        "stretching must use the bidi-mirrored base glyph selected by normal shaping"
    );
    assert_eq!(
        glyphs(&rtl, ")"),
        glyphs(&ltr, "("),
        "both fence assemblies must preserve bidi mirroring"
    );
}

#[test]
fn space_dimension_attributes_participate_in_the_cascade_and_preserve_negative_depth() {
    let view =
        math("<math><mn>1</mn><mspace width='30pt' style='width:inherit'/><mn>2</mn></math>");
    assert_eq!(
        view.lines[1].x,
        view.lines[0].x + view.lines[0].width,
        "an author width:inherit overriding the mspace width hint must inherit auto, rather than reapply the attribute after the cascade"
    );
    let space = math(
        "<math><mspace width='10pt' height='30pt' depth='-10pt' mathbackground='red'/></math>",
    );
    assert_eq!(
        space.area.height, 20.0,
        "mspace height presentation hint is calc(height + depth), before nonnegative CSS height clamping; negative depth is not independently clamped"
    );
    assert_eq!(
        space.baseline,
        Some(30.0),
        "the requested ascent comes from the separately clamped height attribute"
    );
}

#[test]
fn unchanged_math_depth_does_not_load_unused_font_metrics_while_formatting_a_space() {
    #[derive(Default)]
    struct Resolver(std::cell::RefCell<Vec<String>>);
    impl crate::resources::ResourceResolver for Resolver {
        fn resolve(
            &self,
            reference: &str,
            _: &crate::resources::ResourceBase,
        ) -> Result<crate::resources::Resource, crate::resources::ResourceError> {
            self.0.borrow_mut().push(reference.to_owned());
            Err(crate::resources::ResourceError::UnsupportedScheme(
                reference.to_owned(),
            ))
        }
    }
    let resolver = Resolver::default();
    let options = local_options();
    crate::document::StyledDocument::parse("<style>@font-face{font-family:UnusedMath;src:url(unused-math.ttf)}math{font-family:UnusedMath}</style><math><mspace width='10pt' height='20pt'/></math>", &options.base, &[], &resolver).expect("formatting a fixed-size math space needs no font-size metrics when depth is unchanged");
    assert!(
        resolver.0.borrow().is_empty(),
        "Core §4.5 returns scale 1 before consulting MATH when inherited and computed depths match; unexpected fetches: {:?}",
        resolver.0.borrow()
    );
}

#[test]
fn hidden_semantic_annotations_do_not_activate_unsupported_style_or_font_work() {
    let view = math(
        "<math><semantics><mn>1</mn><annotation style='background-image:element(#missing)'>hidden</annotation></semantics><maction><mn>2</mn><mrow style='background-image:element(#missing)'><mn>3</mn></mrow></maction></math>",
    );
    assert_eq!(
        view.lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>(),
        ["1", "2"],
        "Core UA display:none annotations and alternative actions must take the renderer's normal hidden-box path"
    );
}

#[test]
fn specified_math_width_does_not_scale_its_height_like_a_replaced_image() {
    let rectangle = |declaration: &str| {
        let output = render_html(&format!("<style>math{{font-family:DejaVu Serif;font-size:20pt;background:#010203;{declaration}}}</style><math><mn>8</mn></math>"), &local_options()).expect("sized math box must render");
        output
            .layout
            .unwrap()
            .display_lists
            .into_iter()
            .flatten()
            .find_map(|item| match item {
                DisplayItem::Box(paint) if paint.decoration.background.rgb == [1, 2, 3] => {
                    Some(paint.rect)
                }
                _ => None,
            })
            .expect("math background exposes its used content dimensions")
    };
    let natural = rectangle("");
    let sized = rectangle("width:100pt");
    assert_eq!(
        sized.width, 100.0,
        "author width sizes the math content box"
    );
    assert_eq!(
        sized.height, natural.height,
        "Core §3.1.2 changes the requested axis independently; native math text and baseline must not inherit a replaced-image aspect ratio"
    );
}

#[test]
fn anonymous_math_text_preserves_nonbreaking_space_without_treating_it_as_indentation() {
    let ordinary = math("<math>  <mn>1</mn></math>");
    let nonbreaking = math("<math>&nbsp;<mn>1</mn></math>");
    let position = |view: &MathView| view.lines.iter().find(|line| line.text == "1").unwrap().x;
    assert!(
        position(&nonbreaking) > position(&ordinary),
        "U+00A0 is a retained text advance; CSS indentation whitespace and Unicode whitespace are not interchangeable"
    );
}

#[test]
fn math_edges_extend_present_italic_corrections_in_each_inline_direction() {
    const EDGE_WIDTH: f32 = 15.0;
    for direction in ["ltr", "rtl"] {
        let side = if direction == "ltr" { "right" } else { "left" };
        for (token, expected) in [("<mi>f</mi>", 2.0 * EDGE_WIDTH), ("<mn>1</mn>", EDGE_WIDTH)] {
            let expression = |edge: bool| {
                let token = if edge {
                    token.replacen(
                        '>',
                        &format!(
                            " style='padding-{side}:7pt;border-{side}:3pt solid;margin-{side}:5pt'>"
                        ),
                        1,
                    )
                } else {
                    token.to_owned()
                };
                math(&format!(
                    "<math dir='{direction}' style='font-family:MathMetrics'><msup>{token}<mn>2</mn></msup></math>"
                ))
            };
            let plain = expression(false);
            let edged = expression(true);
            let offset = |view: &MathView| {
                let sup = view.lines.iter().find(|line| line.text == "2").unwrap();
                if direction == "ltr" {
                    sup.x
                } else {
                    view.area.width - sup.x - sup.width
                }
            };
            let delta = offset(&edged) - offset(&plain);
            assert!(
                (delta - expected).abs() < 0.01,
                "{direction} {token}: superscript offset must include the margin-box width and extend only an existing italic correction; expected {expected}, got {delta}"
            );
        }
    }
}

#[test]
fn overscript_top_accent_attachment_uses_logical_inline_start_edges() {
    let expression = |direction: &str, side: &str| {
        math(&format!(
            "<math dir='{direction}' style='font-family:MathMetrics'><mover><mspace width='10pt'/><mn style='padding-{side}:20pt'>1</mn></mover></math>"
        ))
    };
    let ltr = expression("ltr", "left");
    let rtl = expression("rtl", "right");
    assert!(
        (ltr.area.width - rtl.area.width).abs() < 0.01,
        "mirrored overscripts must preserve width when their MATH top accent attachment is increased by inline-start padding: LTR {}, RTL {}",
        ltr.area.width,
        rtl.area.width
    );
}

#[test]
fn math_nesting_limit_returns_an_error_without_overflowing_a_library_thread() {
    const CHILD_ENV: &str = "FERRI_MATH_NESTING_CHILD";
    const TEST_NAME: &str =
        "tests::mathml::math_nesting_limit_returns_an_error_without_overflowing_a_library_thread";
    const MATH_NESTING_LIMIT: usize = 256;
    const LIBRARY_STACK_BYTES: usize = 2 * 1024 * 1024;
    if std::env::var_os(CHILD_ENV).is_none() {
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", TEST_NAME, "--nocapture"])
            .env(CHILD_ENV, "1")
            .status()
            .expect("MathML depth regression subprocess must start");
        assert!(
            status.success(),
            "MathML nesting must remain bounded on a {LIBRARY_STACK_BYTES}-byte library thread; child exit: {status}"
        );
        return;
    }
    std::thread::Builder::new()
        .stack_size(LIBRARY_STACK_BYTES)
        .spawn(|| {
            for (levels, allowed) in [(MATH_NESTING_LIMIT - 2, true), (MATH_NESTING_LIMIT - 1, false)] {
                eprintln!("checking native MathML boundary: {levels} mrows on a {LIBRARY_STACK_BYTES}-byte stack");
                let html = format!("<math>{}<mn>1</mn>{}</math>", "<mrow>".repeat(levels), "</mrow>".repeat(levels));
                let result = render_html(&html, &local_options());
                if allowed {
                    let output = result.expect("the inclusive MathML nesting boundary must render");
                    assert!(output.layout.unwrap().pages.iter().flatten().any(|line| line.text == "1"), "valid deep math must retain canonical selectable native text");
                } else {
                    let error = result.err().expect("one further native math level must return its nesting error");
                    assert!(error.to_string().contains(&format!("MathML nesting exceeds {MATH_NESTING_LIMIT}")), "native math must report its explicit resource boundary: {error}");
                }
            }
            const ELEMENTS_PER_MIXED_SEGMENT: usize = 4;
            const MIXED_SEGMENTS: usize = (MATH_NESTING_LIMIT - 1) / ELEMENTS_PER_MIXED_SEGMENT;
            const REMAINING_ROWS: usize = MATH_NESTING_LIMIT - 2 - MIXED_SEGMENTS * ELEMENTS_PER_MIXED_SEGMENT;
            for (rows, allowed) in [(REMAINING_ROWS, true), (REMAINING_ROWS + 1, false)] {
                const MIXED_CSS_SIZE_PT: usize = 30;
                let open = format!("<mrow><mtext><span style='display:block;width:{MIXED_CSS_SIZE_PT}pt;height:{MIXED_CSS_SIZE_PT}pt;writing-mode:vertical-rl;opacity:.9'><math>");
                let close = "</math></span></mtext></mrow>";
                let html = format!(
                    "<math>{}{}<mn>1</mn>{}{}</math>",
                    open.repeat(MIXED_SEGMENTS), "<mrow>".repeat(rows),
                    "</mrow>".repeat(rows), close.repeat(MIXED_SEGMENTS)
                );
                eprintln!("checking mixed Math/CSS/writing boundary: {MIXED_SEGMENTS} four-element segments plus {rows} rows on a {LIBRARY_STACK_BYTES}-byte stack");
                let result = render_html(&html, &local_options());
                if allowed {
                    let output = result.expect("mixed source/native boundaries must preserve the same inclusive depth and default stack");
                    assert_eq!(output.layout.unwrap().pages.iter().flatten().filter(|line| line.text == "1").count(), 1, "mixed native/CSS transport must retain the single canonical token exactly once");
                } else {
                    let error = result.err().expect("a Math/CSS/writing boundary must not reset the scoped depth counter");
                    assert!(error.to_string().contains(&format!("MathML nesting exceeds {MATH_NESTING_LIMIT}")), "mixed sources must report the same typed formatting boundary: {error}");
                }
            }
        })
        .expect("bounded-stack library thread must start")
        .join()
        .expect("bounded-stack math layout must finish");
}

#[test]
fn outer_math_uses_the_shared_positioning_and_effects_pipeline() {
    let output = render_html(
        "<style>@page{margin:0}body{margin:0}math{position:absolute;left:30pt;top:40pt;transform:translate(7pt,9pt);opacity:.4;filter:opacity(.8);background:red}</style><math><mfrac><mn>1</mn><mn>2</mn></mfrac></math>",
        &local_options(),
    ).expect("outer native math effects belong to the existing CSS box painter");
    let trace = output.layout.unwrap();
    let items: Vec<_> = trace.display_lists.iter().flatten().collect();
    let image = items
        .iter()
        .find_map(|item| match item {
            DisplayItem::Box(paint) if paint.decoration.background.rgb == [255, 0, 0] => {
                Some(paint)
            }
            _ => None,
        })
        .expect("positioned math must retain its real CSS principal box");
    assert!(
        (image.rect.x - 30.0).abs() < 0.01 && (image.rect.y - 40.0).abs() < 0.01,
        "absolute math content must use its containing block offsets: {:?}",
        image.rect
    );
    assert!(items.iter().any(|item| matches!(item, DisplayItem::PushTransform(matrix) if (matrix[4] - 7.0).abs() < 0.01 && (matrix[5] - 9.0).abs() < 0.01)), "outer transform must reach the shared display list");
    assert!(
        items
            .iter()
            .any(|item| matches!(item, DisplayItem::PushOpacity(alpha) if *alpha == 0.4)),
        "outer opacity must reach the shared compositor"
    );
    assert!(
        items
            .iter()
            .any(|item| matches!(item, DisplayItem::PushFilter(_))),
        "outer filter must reach the shared filter painter"
    );
}

#[test]
fn scroll_and_layout_containment_suppress_native_math_baseline_participation() {
    for css in [
        "overflow:auto",
        "overflow:hidden",
        "overflow:scroll",
        "contain:layout",
    ] {
        let html = format!(
            "<style>@page{{margin:0}}body{{margin:0;font:20pt DejaVu Serif}}</style>L<math style='{css};background:red'><mn>g</mn></math>R"
        );
        let trace = render_html(&html, &local_options())
            .expect("contained math must render")
            .layout
            .unwrap();
        let baseline = trace
            .pages
            .iter()
            .flatten()
            .find(|line| line.text.contains('L'))
            .unwrap()
            .baseline;
        let image = trace
            .display_lists
            .iter()
            .flatten()
            .find_map(|item| match item {
                DisplayItem::Box(paint) if paint.decoration.background.rgb == [255, 0, 0] => {
                    Some(paint)
                }
                _ => None,
            })
            .unwrap();
        assert!(
            (image.rect.y + image.rect.height - baseline).abs() < 0.01,
            "{css}: a math box without baseline participation aligns its bottom margin edge to the surrounding alphabetic baseline; bottom {}, baseline {baseline}",
            image.rect.y + image.rect.height
        );
    }
}

#[test]
fn limit_line_metrics_follow_core_equations_independently_of_script_ink_positions() {
    const FONT_SIZE: f32 = 20.0;
    const BASE_ASCENT: f32 = 10.0;
    const BASE_DESCENT: f32 = 2.0;
    const SCRIPT_ASCENT: f32 = 7.0;
    const SCRIPT_DESCENT: f32 = 3.0;
    let face =
        ttf_parser::Face::parse(include_bytes!("../../assets/fonts/DejaVuSerif.ttf"), 0).unwrap();
    let constants = face.tables().math.unwrap().constants.unwrap();
    let scale = FONT_SIZE / f32::from(face.units_per_em());
    let under_shift = f32::from(constants.underbar_vertical_gap().value) * scale;
    let under_extra = f32::from(constants.underbar_extra_descender().value) * scale;
    let over_shift = f32::from(constants.overbar_vertical_gap().value) * scale;
    let over_extra = f32::from(constants.overbar_extra_ascender().value) * scale;
    for (tag, under, over) in [
        ("munder", true, false),
        ("mover", false, true),
        ("munderover", true, true),
    ] {
        let script = "<mspace width='5pt' height='7pt' depth='3pt'/>";
        let view = math(&format!(
            "<math><{tag}><mspace width='20pt' height='10pt' depth='2pt'/>{}{}</{tag}></math>",
            if under { script } else { "" },
            if over { script } else { "" }
        ));
        let mut ascent = BASE_ASCENT;
        let mut descent = BASE_DESCENT;
        if under {
            ascent = ascent.max(SCRIPT_ASCENT - under_shift);
            descent = descent.max(SCRIPT_DESCENT + under_shift + under_extra);
        }
        if over {
            ascent = ascent.max(SCRIPT_ASCENT + over_shift + over_extra);
            descent = descent.max(SCRIPT_DESCENT - over_shift);
        }
        let actual_ascent = view.baseline.unwrap();
        let actual_descent = view.area.height - actual_ascent;
        assert!(
            (actual_ascent - ascent).abs() < 0.01 && (actual_descent - descent).abs() < 0.01,
            "{tag}: Core 3.4.2 keeps the explicit line metrics separate from scripts positioned using the base ink extent; expected ascent/descent {ascent}/{descent}, got {actual_ascent}/{actual_descent}"
        );
    }
}

#[test]
fn intrinsic_math_widths_reserve_all_block_stretch_variants_through_native_algorithms() {
    const FONT_SIZE: f32 = 20.0;
    const RED: [u8; 3] = [255, 0, 0];
    let face = ttf_parser::Face::parse(
        include_bytes!("../../tests/fixtures/fonts/MathMetrics.otf"),
        0,
    )
    .unwrap();
    let em = FONT_SIZE / f32::from(face.units_per_em());
    let glyph = face.glyph_index('(').unwrap();
    let table = face.tables().math.unwrap();
    let variants = table
        .variants
        .unwrap()
        .vertical_constructions
        .get(glyph)
        .unwrap();
    let preferred = variants
        .variants
        .into_iter()
        .map(|record| f32::from(face.glyph_hor_advance(record.variant_glyph).unwrap()) * em)
        .fold(
            f32::from(face.glyph_hor_advance(glyph).unwrap()) * em,
            f32::max,
        );
    let constants = table.constants.unwrap();
    let digit = f32::from(
        face.glyph_hor_advance(face.glyph_index('1').unwrap())
            .unwrap(),
    ) * em;
    let script_digit = digit * f32::from(constants.script_percent_scale_down()) / 100.0;
    let script_space = f32::from(constants.space_after_script().value) * em;
    let fence = "<mo lspace='0' rspace='0'>(</mo>";
    for (expression, expected) in [
        (fence.to_owned(), preferred),
        (format!("<mrow>{fence}<mn>1</mn></mrow>"), preferred + digit),
        (
            format!("<mfrac>{fence}<mn>1</mn></mfrac>"),
            preferred * f32::from(constants.script_percent_scale_down()) / 100.0
                + 2.0 * crate::style::POINTS_PER_PIXEL,
        ),
        (
            format!("<msup>{fence}<mn>1</mn></msup>"),
            preferred + script_digit + script_space,
        ),
        (format!("<munder>{fence}<mn>1</mn></munder>"), preferred),
        (
            format!("<mmultiscripts>{fence}<mn>1</mn><mn>1</mn></mmultiscripts>"),
            preferred + script_digit + script_space,
        ),
        (
            format!(
                "<mmultiscripts>{fence}<mn>1</mn><mn>1</mn><mprescripts/><mn>1</mn><mn>1</mn></mmultiscripts>"
            ),
            preferred + 2.0 * (script_digit + script_space),
        ),
        (format!("<mphantom>{fence}</mphantom>"), preferred),
        (format!("<mpadded>{fence}</mpadded>"), preferred),
    ] {
        for keyword in ["min-content", "max-content"] {
            let output = math_output(&format!(
                "<math style='font-family:MathMetrics;width:{keyword};background:red'>{expression}</math>"
            ));
            let trace = output.layout.unwrap();
            let width = trace
                .display_lists
                .iter()
                .flatten()
                .find_map(|item| match item {
                    DisplayItem::Box(paint) if paint.decoration.background.rgb == RED => {
                        Some(paint.rect.width)
                    }
                    _ => None,
                })
                .expect("native math's CSS content sizing must retain its outer background box");
            assert!(
                (width - expected).abs() < 0.01,
                "{keyword} {expression}: Core intrinsic widths must compose the maximum advance of every vertical variant, independently of the chosen painted glyph; expected {expected}, got {width}"
            );
        }
    }
}

#[test]
fn preferred_assembly_width_and_radical_intrinsics_keep_original_font_fallback() {
    const FONT_SIZE: f32 = 20.0;
    const RED: [u8; 3] = [255, 0, 0];
    for (family, data, valid_assembly) in [
        (
            "MathTest",
            include_bytes!("../../tests/fixtures/fonts/MathTest.ttf").as_slice(),
            true,
        ),
        (
            "DejaVu Serif",
            include_bytes!("../../assets/fonts/DejaVuSerif.ttf").as_slice(),
            false,
        ),
    ] {
        let face = ttf_parser::Face::parse(data, 0).unwrap();
        let em = FONT_SIZE / f32::from(face.units_per_em());
        let advance = |character| {
            f32::from(
                face.glyph_hor_advance(face.glyph_index(character).unwrap())
                    .unwrap(),
            ) * em
        };
        let table = face.tables().math.unwrap();
        let constants = table.constants.unwrap();
        let assembly = table
            .variants
            .unwrap()
            .vertical_constructions
            .get(face.glyph_index('(').unwrap())
            .unwrap()
            .assembly
            .unwrap();
        let assembly_width = assembly
            .parts
            .into_iter()
            .map(|part| f32::from(face.glyph_hor_advance(part.glyph_id).unwrap()) * em)
            .fold(0.0, f32::max);
        let fence_width = if valid_assembly {
            assembly_width.max(advance('('))
        } else {
            advance('(')
        };
        let root_width = advance('√') + fence_width;
        let index_width =
            advance('3') * f32::from(constants.script_script_percent_scale_down()) / 100.0;
        let index_kerns = (f32::from(constants.radical_kern_before_degree().value) * em).max(0.0)
            + (f32::from(constants.radical_kern_after_degree().value) * em).max(-index_width);
        let fence = "<mo lspace='0' rspace='0'>(</mo>";
        for (expression, expected) in [
            (fence.to_owned(), fence_width),
            (format!("<msqrt>{fence}</msqrt>"), root_width),
            (
                format!("<mroot>{fence}<mn>3</mn></mroot>"),
                root_width + index_width + index_kerns,
            ),
        ] {
            let output = math_output(&format!(
                "<math style='font-family:{family};width:max-content;background:red'>{expression}</math>"
            ));
            let width = output
                .layout
                .unwrap()
                .display_lists
                .iter()
                .flatten()
                .find_map(|item| match item {
                    DisplayItem::Box(paint) if paint.decoration.background.rgb == RED => {
                        Some(paint.rect.width)
                    }
                    _ => None,
                })
                .unwrap();
            assert!(
                (width - expected).abs() < 0.01,
                "{family} {expression}: intrinsic composition must use the valid assembly's width while the immutable original face retains its invalid-assembly fallback; expected {expected}, got {width}"
            );
        }
    }
}

#[test]
fn non_math_display_math_keeps_ordinary_inline_fragmentation() {
    let output = render_html(
        "<style>@page{margin:0}body{margin:0;width:35pt;font:10pt/12pt monospace}span{display:inline math}</style>A <span>x y z</span> B",
        &local_options(),
    ).expect("non-MathML inline math computes to inline flow");
    let lines: Vec<_> = output
        .layout
        .unwrap()
        .pages
        .into_iter()
        .flatten()
        .map(|line| line.text)
        .collect();
    assert_eq!(
        lines,
        ["A x y", "z B"],
        "display:inline math on HTML must fragment with surrounding text rather than create an atomic math container"
    );
}

#[test]
fn non_math_display_math_keeps_block_first_line_propagation() {
    let output = render_html(
        "<style>p::first-line{color:red}span{display:block math}</style><p><span>First</span></p>",
        &local_options(),
    )
    .expect("non-MathML block math computes to block flow");
    let trace = output.layout.unwrap();
    let line = trace
        .pages
        .iter()
        .flatten()
        .find(|line| line.text == "First")
        .unwrap();
    assert!(
        line.runs.iter().all(|run| run.color.rgb == [255, 0, 0]),
        "HTML block math must retain ordinary first-line propagation through its block container: {:?}",
        line.runs.iter().map(|run| run.color).collect::<Vec<_>>()
    );
}

#[test]
fn math_attribute_metrics_preserve_active_font_policy_errors() {
    const RESOURCE_LIMIT_BYTES: usize = 1;
    let font = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/fonts/MetricGlyphTest.ttf");
    let resolver = crate::resources::FileResolver {
        max_resource_bytes: RESOURCE_LIMIT_BYTES,
        ..Default::default()
    };
    let source = |attribute: &str| {
        format!(
            "<style>@font-face{{font-family:BlockedMath;src:url('{}')}}math{{font-family:BlockedMath}}</style><math><mpadded lspace='{attribute}'><mspace width='10pt' height='20pt'/></mpadded></math>",
            font.display()
        )
    };
    let options = local_options();
    crate::document::StyledDocument::parse(&source("invalid"), &options.base, &[], &resolver)
        .expect("invalid MathML length attributes recover without activating unused font metrics");
    let error =
        crate::document::StyledDocument::parse(&source("1ex"), &options.base, &[], &resolver)
            .err()
            .expect(
                "valid metric-bearing MathML attributes must preserve active font policy failure",
            );
    assert!(
        matches!(
            error,
            crate::document::DocumentError::Style(crate::style::StyleError::Font(
                crate::fonts::FontError::Resource(crate::resources::ResourceError::TooLarge {
                    limit: RESOURCE_LIMIT_BYTES,
                    ..
                })
            ))
        ),
        "mpadded lspace=1ex must retain its typed resource error rather than silently replacing the valid attribute by zero: {error}"
    );
}

#[test]
fn padded_operators_forward_their_parent_stretch_constraint() {
    let bare = math(
        "<math style='font-family:MathTest'><mo>(</mo><mspace height='40pt' depth='10pt'/></math>",
    );
    let padded = math(
        "<math style='font-family:MathTest'><mpadded><mo>(</mo></mpadded><mspace height='40pt' depth='10pt'/></math>",
    );
    let glyphs = |view: &MathView| {
        view.lines
            .iter()
            .find(|line| line.text == "(")
            .expect("fixture retains its fence")
            .runs
            .iter()
            .flat_map(|run| run.glyphs().iter().map(|glyph| glyph.id))
            .collect::<Vec<_>>()
    };
    let bare_glyphs = glyphs(&bare);
    assert!(
        bare_glyphs.len() > 1,
        "the fixture must exercise a valid native fence assembly: {bare_glyphs:?}"
    );
    assert_eq!(
        glyphs(&padded),
        bare_glyphs,
        "Core3.2.4.1 keeps mpadded embellished, forwarding its parent's stretch target to the same core operator"
    );
}

#[test]
fn space_like_padding_does_not_change_neighboring_operator_form() {
    const SPACE_WIDTH: f32 = 10.0;
    let bare = math("<math><mo>+</mo><mn>1</mn></math>");
    let padded = math("<math><mpadded><mspace width='10pt'/></mpadded><mo>+</mo><mn>1</mn></math>");
    let position = |view: &MathView| view.lines.iter().find(|line| line.text == "1").unwrap().x;
    let offset = position(&padded) - position(&bare);
    assert!(
        (offset - SPACE_WIDTH).abs() < 0.01,
        "Core3.2.5.1 keeps a padding group of spaces space-like: it reserves {SPACE_WIDTH}pt without turning prefix plus into infix; got {offset}pt"
    );
}

#[test]
fn invalid_script_and_fraction_arity_keeps_the_first_child_core_operator() {
    for (element, tail) in [
        ("mfrac", "<mn>1</mn><mn>2</mn>"),
        ("msub", "<mn>1</mn><mn>2</mn>"),
        ("msup", "<mn>1</mn><mn>2</mn>"),
        ("msubsup", "<mn>1</mn>"),
        ("munder", "<mn>1</mn><mn>2</mn>"),
        ("mover", "<mn>1</mn><mn>2</mn>"),
        ("munderover", "<mn>1</mn>"),
        ("mmultiscripts", "<mn>1</mn>"),
    ] {
        let view = math(&format!(
            "<math style='font-family:MathTest'><{element}><mo>(</mo>{tail}</{element}><mspace height='60pt'/></math>"
        ));
        let fence = view.lines.iter().find(|line| line.text == "(").unwrap();
        let glyphs: Vec<_> = fence
            .runs
            .iter()
            .flat_map(|run| run.glyphs().iter().map(|glyph| glyph.id))
            .collect();
        assert!(
            glyphs.len() > 1,
            "Core3.2.4.1 keeps the first child as the core operator of an invalid-arity {element}; the outer 60pt space must stretch its fence despite row recovery: {glyphs:?}"
        );
    }
}

#[test]
fn recovered_fraction_operator_spacing_belongs_to_the_embellished_box() {
    let bare = math("<math><mn>7</mn><mo>+</mo><mn>8</mn></math>");
    let recovered = math(
        "<math><mn>7</mn><mfrac style='padding:0'><mo style='font-size:20pt'>+</mo><mn style='font-size:20pt'>1</mn><mn style='font-size:20pt'>2</mn></mfrac><mn>8</mn></math>",
    );
    let gaps = |view: &MathView, last: &str| {
        let find = |text| view.lines.iter().find(|line| line.text == text).unwrap();
        let first = find("7");
        let plus = find("+");
        let last = find(last);
        let following = find("8");
        [
            plus.x - first.x - first.width,
            following.x - last.x - last.width,
        ]
    };
    let expected = gaps(&bare, "+");
    let actual = gaps(&recovered, "2");
    for (side, (actual, expected)) in ["start", "end"]
        .into_iter()
        .zip(actual.into_iter().zip(expected))
    {
        assert!(
            (actual - expected).abs() < 0.01,
            "invalid-arity mfrac stays embellished: infix plus {side} space must surround the whole recovered box, matching a bare core operator; expected {expected}, got {actual}"
        );
    }
}

#[test]
fn recovered_script_children_keep_their_postfix_operator_form() {
    let recovered = math(
        "<math><msub><mn>1</mn><mo style='font-size:20pt'>|</mo><mn style='font-size:20pt'>2</mn></msub></math>",
    );
    let explicit = math("<math><mrow><mn>1</mn><mo form='postfix'>|</mo><mn>2</mn></mrow></math>");
    let position = |view: &MathView| view.lines.iter().find(|line| line.text == "2").unwrap().x;
    assert!(
        (position(&recovered) - position(&explicit)).abs() < 0.01,
        "Core3.2.4.2 keeps non-base scripted children postfix even when invalid arity recovers row geometry; explicit postfix digit x {}, recovered {}",
        position(&explicit),
        position(&recovered)
    );
}

#[test]
fn recovered_non_grouping_schemata_of_spaces_remain_non_space_like() {
    let control = math("<math><mn>0</mn><mo>+</mo><mn>1</mn></math>");
    let gap = |view: &MathView| {
        let plus = view.lines.iter().find(|line| line.text == "+").unwrap();
        let digit = view.lines.iter().find(|line| line.text == "1").unwrap();
        digit.x - plus.x - plus.width
    };
    let expected = gap(&control);
    for element in [
        "mroot",
        "mfrac",
        "msub",
        "msup",
        "munder",
        "mover",
        "mmultiscripts",
    ] {
        let recovered = math(&format!(
            "<math><{element}><mspace width='10pt'/><mspace width='20pt'/><mspace width='30pt'/><mspace width='40pt'/></{element}><mo>+</mo><mn>1</mn></math>"
        ));
        let actual = gap(&recovered);
        assert!(
            (actual - expected).abs() < 0.01,
            "Core3.2.5.1 does not classify {element} as space-like even when row recovery contains only spaces; following plus must remain infix with {expected}pt end space, got {actual}"
        );
    }
}

#[test]
fn recovered_root_children_keep_their_infix_operator_form() {
    let recovered =
        math("<math><mroot><mo>|</mo><mn style='font-size:20pt'>1</mn><mn>2</mn></mroot></math>");
    let explicit = math("<math><mrow><mo form='infix'>|</mo><mn>1</mn><mn>2</mn></mrow></math>");
    let position = |view: &MathView| view.lines.iter().find(|line| line.text == "2").unwrap().x;
    assert!(
        (position(&recovered) - position(&explicit)).abs() < 0.01,
        "Core3.2.4.2 leaves children of non-grouping mroot infix despite row recovery; explicit infix digit x {}, recovered {}",
        position(&explicit),
        position(&recovered)
    );
}

#[test]
fn recovered_operator_identity_uses_the_first_visible_box_including_anonymous_text() {
    for (leading, embellished) in [
        ("<mtext style='display:none'>hidden</mtext>", true),
        ("text", false),
    ] {
        let glyphs = |sibling| {
            let view = math(&format!(
                "<math style='font-family:MathTest'><mfrac>{leading}<mo>(</mo><mn>1</mn><mn>2</mn></mfrac>{sibling}</math>"
            ));
            view.lines
                .iter()
                .find(|line| line.text == "(")
                .unwrap()
                .runs
                .iter()
                .flat_map(|run| run.glyphs().iter().map(|glyph| glyph.id))
                .collect::<Vec<_>>()
        };
        let natural = glyphs("");
        let constrained = glyphs("<mspace height='60pt'/>");
        if embellished {
            assert!(
                constrained.len() > natural.len(),
                "hidden first elements have no box: the visible core operator must receive the outer stretch target; natural {natural:?}, constrained {constrained:?}"
            );
        } else {
            assert_eq!(
                constrained, natural,
                "anonymous text is the first in-flow box, so the fraction is not embellished and must not forward an outer stretch target"
            );
        }
    }
}
