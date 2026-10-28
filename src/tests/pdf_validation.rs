use super::common;

use crate::document::{DocumentMetadata, Navigation};
use crate::writer::PdfError;
use ferri_pdf_fonts::FontBook;
use ferri_pdf_graphics::{Clip, DisplayItem, Rect};

#[test]
fn pdf_errors_unwind_nested_display_list_scopes() {
    let html = format!(
        "<!doctype html><style>{}</style><body></body>",
        common::PROFILE
    );
    let mut trace = common::render_trace(&html);
    assert!(trace.pages[0].is_empty());
    trace.display_lists[0] = vec![
        DisplayItem::PushOpacity(0.5),
        DisplayItem::PushTransform([1.0, 0.0, 0.0, 1.0, 10.0, 10.0]),
        DisplayItem::PushClip(Clip {
            rect: Rect {
                x: 0.0,
                y: 0.0,
                width: 20.0,
                height: 20.0,
            },
            radii: [[0.0; 2]; 4],
            axes: [true; 2],
            reference_box: None,
        }),
        DisplayItem::Text { line: 0 },
    ];
    let result = crate::writer::encode(
        &trace,
        &FontBook::default(),
        &Navigation::default(),
        &DocumentMetadata::default(),
        &Default::default(),
    );
    assert!(
        matches!(
            result,
            Err(PdfError::InvalidTextReference { page: 0, line: 0 })
        ),
        "invalid text must preserve its typed error while unwinding clips, transforms and opacity groups: {result:?}"
    );
}

#[test]
fn writer_uses_physical_page_records_and_validates_their_count() {
    let html = format!(
        "<!doctype html><style>{}</style><body></body>",
        common::PROFILE
    );
    let mut trace = common::render_trace(&html);
    let fonts = FontBook::default();
    let navigation = Navigation::default();
    let metadata = DocumentMetadata::default();
    trace.pages.push(Vec::new());
    trace.display_lists.push(Vec::new());
    trace.links.push(Vec::new());
    assert!(
        matches!(
            crate::writer::encode(&trace, &fonts, &navigation, &metadata, &Default::default()),
            Err(PdfError::PageRecordCount {
                pages: 2,
                records: 1
            })
        ),
        "missing physical geometry must fail before serialization"
    );
    let mut second = trace.page_records[0].clone();
    second.geometry.paper = [500.0, 300.0];
    second.output.media = Rect {
        x: 0.0,
        y: 0.0,
        width: 500.0,
        height: 300.0,
    };
    second.output.trim = second.output.media;
    second.output.bleed = second.output.media;
    trace.page_records.push(second);
    trace.page_width = f32::NAN;
    trace.page_height = f32::NAN;
    trace.page_area.width = f32::NAN;
    trace.page_output.media.width = f32::NAN;
    crate::writer::encode(&trace, &fonts, &navigation, &metadata, &Default::default())
        .expect("first-page compatibility fields must not drive physical page serialization");
    trace.page_records[1].geometry.paper[0] = f32::NAN;
    assert!(
        matches!(crate::writer::encode(&trace, &fonts, &navigation, &metadata, &Default::default()), Err(PdfError::InvalidPageSize { width, height: 300.0 }) if width.is_nan()),
        "each physical page validates its own dimensions"
    );
}
