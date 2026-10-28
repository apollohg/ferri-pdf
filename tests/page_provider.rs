mod common;

use common::{PROFILE, local_options};
use ferri_pdf_document::StyledDocument;
use ferri_pdf_graphics::BackgroundImage;
use ferri_pdf_resources::FileResolver;
use ferri_pdf_style::{PageRequest, PageSide};
use std::sync::Arc;

#[test]
fn owned_page_records_freeze_viewport_and_share_loaded_images() {
    let document = StyledDocument::parse(
        &format!("<!doctype html><style>{PROFILE}@page A{{size:300pt 400pt;margin:20pt;background-image:url('../assets/image-pattern.png')}}@page B{{size:500pt 300pt;margin:10vw;background-image:url('../assets/image-pattern.png')}}@page unused{{background-image:url('missing-page.png')}}body{{page:A;background-image:url('../assets/image-pattern.png')}}</style><p>Body</p>"),
        &local_options().base, &[], &FileResolver::default(),
    ).unwrap();
    let first = document.page_box();
    assert_eq!(first.request.name.as_deref(), Some("A"));
    assert_eq!(first.geometry.area, [260.0, 360.0]);
    let request = PageRequest {
        name: Some("B".into()),
        side: PageSide::Left,
        first: false,
        blank: false,
    };
    let later = document
        .resolve_page(request.clone(), &FileResolver::default())
        .unwrap();
    assert_eq!(later.geometry.paper, [500.0, 300.0]);
    assert_eq!(
        later.geometry.margin, [26.0; 4],
        "viewport units remain tied to the first selected page area"
    );
    assert_eq!(later.geometry.area, [448.0, 248.0]);
    let again = document
        .resolve_page(request, &FileResolver::default())
        .unwrap();
    assert!(
        Arc::ptr_eq(&later, &again),
        "identical semantic requests reuse the owned record"
    );
    let (
        BackgroundImage::Image { image: first_image },
        BackgroundImage::Image { image: later_image },
    ) = (
        &first.decoration.backgrounds.images[0],
        &later.decoration.backgrounds.images[0],
    )
    else {
        panic!("both page decorations must materialize their image");
    };
    assert!(
        Arc::ptr_eq(first_image, later_image),
        "page variants share the document image cache"
    );
    let BackgroundImage::Image {
        image: canvas_image,
    } = &document.canvas_background().backgrounds.images[0]
    else {
        panic!("body image must propagate to the canvas");
    };
    assert!(
        Arc::ptr_eq(first_image, canvas_image),
        "page resolution retains the formatting image cache"
    );
    let original = document
        .resolve_page(first.request.clone(), &FileResolver::default())
        .unwrap();
    assert!(
        std::ptr::eq(first, original.as_ref()),
        "the first page also uses the common record cache"
    );
}

#[test]
fn page_record_cache_distinguishes_side_first_and_blank_flags() {
    let document = StyledDocument::parse(
        &format!("<!doctype html><style>{PROFILE}@page B{{size:300pt 400pt;margin:10pt}}@page B:left{{margin-left:20pt}}@page B:first{{margin-top:30pt}}@page B:blank{{margin-bottom:40pt}}</style><p>Body</p>"),
        &local_options().base, &[], &FileResolver::default(),
    ).unwrap();
    for (side, first, blank, expected) in [
        (PageSide::Right, false, false, [10.0; 4]),
        (PageSide::Left, false, false, [10.0, 10.0, 10.0, 20.0]),
        (PageSide::Left, true, false, [30.0, 10.0, 10.0, 20.0]),
        (PageSide::Left, true, true, [30.0, 10.0, 40.0, 20.0]),
    ] {
        let request = PageRequest {
            name: Some("B".into()),
            side,
            first,
            blank,
        };
        let page = document
            .resolve_page(request.clone(), &FileResolver::default())
            .unwrap();
        assert_eq!(page.geometry.margin, expected, "request {request:?}");
        assert_eq!(page.request, request);
    }
}
