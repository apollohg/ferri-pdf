use crate::style::{Style, Stylesheet, StylesheetSource};
use ferri_pdf_resources::FileResolver;

const TOLERANCE_PT: f32 = 0.005;
const A5_WIDTH_PT: f32 = 419.528;

fn parse(css: &str) -> Result<Stylesheet, crate::style::StyleError> {
    let mut sheet = Stylesheet::parse(&[StylesheetSource {
        text: css.into(),
        base: Default::default(),
    }])?;
    sheet.resolve_page_geometry_with_resolver(&Style::default(), &FileResolver::default())?;
    Ok(sheet)
}

#[test]
fn named_page_sizes_and_configured_orientation_follow_paged_media() {
    let cases = [
        ("A3", [841.890, 1190.551]),
        ("A4", [595.276, 841.890]),
        ("A5", [419.528, 595.276]),
        ("B4", [708.661, 1_000.63]),
        ("B5", [498.898, 708.661]),
        ("letter", [612.0, 792.0]),
        ("legal", [612.0, 1008.0]),
        ("ledger", [792.0, 1224.0]),
        ("A4 landscape", [841.890, 595.276]),
        ("landscape A4", [841.890, 595.276]),
        ("LeDgEr portrait", [792.0, 1224.0]),
        ("auto", [595.276, 841.890]),
        ("portrait", [595.276, 841.890]),
        ("landscape", [841.890, 595.276]),
        ("240pt", [240.0, 240.0]),
        ("240pt 360pt", [240.0, 360.0]),
        (r"\41 4/**/landscape", [841.890, 595.276]),
    ];
    for (value, expected) in cases {
        let sheet = parse(&format!("@page {{size:{value};margin:10pt}}"))
            .unwrap_or_else(|e| panic!("page size {value}: {e}"));
        for (actual, expected) in sheet.page.into_iter().zip(expected) {
            assert!(
                (actual - expected).abs() < TOLERANCE_PT,
                "page size {value}: expected {expected}pt, got {actual}pt"
            );
        }
    }
}

#[test]
fn page_size_cascade_keeps_importance_and_rejects_invalid_combinations() {
    let sheet = parse("@page {size:landscape A4!important} @page {size:letter}").unwrap();
    assert!(
        sheet.page[0] > sheet.page[1],
        "later normal size must not override important landscape size"
    );
    // Values outside the `size` grammar are dropped, leaving the earlier A5.
    for value in [
        "A4 A5",
        "auto landscape",
        "portrait landscape",
        "A4 10pt",
        "10pt landscape",
        "-1pt",
        "1pt 2pt 3pt",
        "unknown",
    ] {
        let sheet = parse(&format!("@page {{size:A5;size:{value};margin:0}}"))
            .unwrap_or_else(|e| panic!("invalid size {value} must be dropped: {e}"));
        assert!(
            (sheet.page[0] - A5_WIDTH_PT).abs() < TOLERANCE_PT,
            "invalid size {value} replaced A5 with {:?}",
            sheet.page
        );
    }
    let sheet = parse("@page {size:0;margin:0}").expect("zero is a valid page size");
    let area = sheet.page_content_box();
    assert_eq!([area.width, area.height], [0.0, 0.0]);
    assert_eq!(sheet.page, crate::style::PrintSettings::default().paper);
}
