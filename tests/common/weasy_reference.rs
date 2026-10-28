use super::common::local_options;
use ferri_pdf::render_html;

pub(crate) fn assert_weasy_lines(reference: &serde_json::Value) {
    const REFERENCE_TOLERANCE_PT: f32 = 0.03;
    for case in reference["cases"].as_array().unwrap() {
        let output = render_weasy_case(reference, case);
        let trace = output.layout.unwrap();
        let expected = case["pages"].as_array().unwrap();
        assert_eq!(
            trace.pages.len(),
            expected.len(),
            "{} page count",
            case["name"]
        );
        for (page, (actual, expected)) in trace.pages.iter().zip(expected).enumerate() {
            let expected = expected.as_array().unwrap();
            let labels = actual
                .iter()
                .map(|line| line.text.as_str())
                .collect::<Vec<_>>();
            assert_eq!(
                actual.len(),
                expected.len(),
                "{} page {page}: {labels:?}",
                case["name"]
            );
            for line in expected {
                let text = line["text"].as_str().unwrap();
                let matches = actual
                    .iter()
                    .filter(|actual| actual.text == text)
                    .collect::<Vec<_>>();
                assert_eq!(
                    matches.len(),
                    1,
                    "{} page {page} must paint {text} exactly once: {labels:?}",
                    case["name"]
                );
                let baseline = line["baseline"].as_f64().unwrap() as f32;
                assert!(
                    (matches[0].baseline - baseline).abs() <= REFERENCE_TOLERANCE_PT,
                    "{} page {page} {text}: actual baseline {}, WeasyPrint 70 reference {baseline}",
                    case["name"],
                    matches[0].baseline
                );
                if reference["measurement"] == "line-geometry" {
                    for (field, actual) in [("x", matches[0].x), ("width", matches[0].width)] {
                        let expected = line[field].as_f64().unwrap() as f32;
                        assert!(
                            (actual - expected).abs() <= REFERENCE_TOLERANCE_PT,
                            "{} page {page} {text}: actual {field} {actual}, WeasyPrint 70 reference {expected}",
                            case["name"]
                        );
                    }
                }
            }
        }
    }
}

pub(crate) fn render_weasy_case(
    reference: &serde_json::Value,
    case: &serde_json::Value,
) -> ferri_pdf::RenderOutput {
    let font = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(reference["font_path"].as_str().unwrap());
    let font = ferri_pdf::Url::from_file_path(font).unwrap();
    let html = format!(
        "<!doctype html><style>@font-face{{font-family:Probe;src:url('{font}')}}{}{}</style>{}",
        reference["base_css"].as_str().unwrap(),
        case["css"].as_str().unwrap(),
        case["body"].as_str().unwrap()
    );
    let output = render_html(&html, &local_options())
        .unwrap_or_else(|error| panic!("WeasyPrint 70 case {} must render: {error}", case["name"]));
    assert!(
        output.diagnostics.is_empty(),
        "{} diagnostics: {:?}",
        case["name"],
        output.diagnostics
    );
    output
}
