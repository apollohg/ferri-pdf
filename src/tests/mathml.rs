use super::common::local_options;
use crate::{DisplayItem, render_html};

fn math(body: &str) -> std::sync::Arc<crate::layout::ViewportPaint> {
    let output = render_html(
        &format!("<style>math{{font-family:DejaVu Serif;font-size:20pt}}</style>{body}"),
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
