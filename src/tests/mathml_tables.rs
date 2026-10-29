use super::common::local_options;
use crate::document::mathml::MAX_DEPTH;
use crate::render_html;

#[test]
fn nested_math_tables_keep_canonical_text_on_the_default_library_stack() {
    const ELEMENTS_PER_TABLE: usize = 3;
    const ROOT_AND_TOKEN: usize = 2;
    const SHALLOW_TABLES: usize = 8;
    let limit = (MAX_DEPTH - ROOT_AND_TOKEN) / ELEMENTS_PER_TABLE;
    let font =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fonts/MathTest.ttf");
    for levels in [1, SHALLOW_TABLES, limit, limit + 1] {
        eprintln!(
            "checking {levels} nested MathML tables (semantic depth {})",
            ELEMENTS_PER_TABLE * levels + ROOT_AND_TOKEN
        );
        let html = format!(
            "<style>@font-face{{font-family:MathTest;src:url('{}')}}math{{font:10pt MathTest}}mtd{{padding:0}}</style><math>{}<mn>1</mn>{}</math>",
            font.display(),
            "<mtable><mtr><mtd>".repeat(levels),
            "</mtd></mtr></mtable>".repeat(levels),
        );
        let result = render_html(&html, &local_options());
        if levels <= limit {
            let output = result.unwrap_or_else(|error| {
                panic!("{levels} valid nested tables must render: {error}")
            });
            let layout = output.layout.expect("layout tracing is enabled");
            assert_eq!(
                layout
                    .pages
                    .iter()
                    .flatten()
                    .filter(|line| line.text == "1")
                    .count(),
                1,
                "{levels} nested tables retain the canonical token once"
            );
        } else {
            let error = result
                .err()
                .expect("the first excessive semantic table depth must fail");
            assert!(
                error
                    .to_string()
                    .contains(&format!("MathML nesting exceeds {MAX_DEPTH}")),
                "anonymous cell rows do not replace the established semantic depth limit: {error}"
            );
        }
    }
}
