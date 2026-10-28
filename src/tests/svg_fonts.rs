use crate::{
    DisplayItem, ImageError, RenderOptions, Resource, ResourceBase, ResourceError,
    ResourceResolver, render_with_resolver,
};
use std::{cell::RefCell, path::PathBuf};

const FONT: &[u8] = include_bytes!("../../crates/fonts/assets/fonts/DejaVuSansMono.ttf");
const CHART: &str = "<svg xmlns='http://www.w3.org/2000/svg' width='100' height='40'><text x='0' y='30' font-family='ChartFace' font-size='20'>H</text></svg>";

#[derive(Default)]
struct Assets {
    calls: RefCell<Vec<(String, ResourceBase)>>,
    denied: bool,
    hidden: bool,
}
impl ResourceResolver for Assets {
    fn resolve(&self, reference: &str, base: &ResourceBase) -> Result<Resource, ResourceError> {
        self.calls
            .borrow_mut()
            .push((reference.into(), base.clone()));
        let (bytes, directory) = match reference {
            "style.css" => (b"@font-face{font-family:ChartFace;src:url('../fonts/chart.ttf')}@font-face{font-family:Unused;src:url('unused.ttf')}".to_vec(), "/virtual/styles"),
            "chart.svg" => ((if self.hidden { CHART.replace("<text ", "<text display='none' ") } else { CHART.into() }).into_bytes(), "/virtual/charts"),
            "../fonts/chart.ttf" => {
                assert_eq!(*base, PathBuf::from("/virtual/styles").into(), "SVG fonts retain the declaring stylesheet's base");
                if self.denied { return Err(ResourceError::Denied(base.resolve(reference)?)); }
                (FONT.to_vec(), "/virtual/fonts")
            }
            "pattern.png" => (include_bytes!("../../tests/oracle/assets/image-pattern.png").to_vec(), "/virtual/charts"),
            "unused.ttf" => panic!("an unrequested font must remain lazy"),
            other => panic!("unexpected resource {other:?} at {base:?}"),
        };
        Ok(Resource {
            bytes,
            base: PathBuf::from(directory).into(),
        })
    }
}

fn options() -> RenderOptions {
    RenderOptions {
        base: PathBuf::from("/virtual").into(),
        include_layout: true,
        ..Default::default()
    }
}

#[test]
fn svg_only_font_sources_share_lazy_acquisition_and_survive_viewport_reparse() {
    let assets = Assets::default();
    let output = render_with_resolver("<link rel='stylesheet' href='style.css'><style>img{display:block}</style><img src='chart.svg'><img src='chart.svg' style='width:150px'>", &options(), &assets).unwrap();
    let calls = assets.calls.borrow();
    assert_eq!(
        calls
            .iter()
            .map(|(name, _)| name.as_str())
            .collect::<Vec<_>>(),
        ["style.css", "chart.svg", "../fonts/chart.ttf"],
        "one font request serves initial conversion, repeated assets and PDF viewport conversion"
    );
    let trace = output.layout.unwrap();
    let image = trace
        .display_lists
        .iter()
        .flatten()
        .find_map(|item| match item {
            DisplayItem::Image(paint) => Some(paint.image.clone()),
            _ => None,
        })
        .unwrap();
    let ferri_pdf_images::Image::Svg(svg) = image.as_ref() else {
        panic!("expected vector chart")
    };
    for size in [[100.0, 40.0], [400.0, 160.0]] {
        let tree = svg.tree(size).unwrap();
        let selected = tree
            .fontdb()
            .faces()
            .filter(|face| {
                face.families
                    .iter()
                    .any(|(family, _)| family == "ChartFace")
            })
            .collect::<Vec<_>>();
        assert_eq!(selected.len(), 1);
        assert_eq!(
            tree.fontdb()
                .with_face_data(selected[0].id, |bytes, index| bytes == FONT && index == 0),
            Some(true)
        );
        let unused = tree
            .fontdb()
            .faces()
            .find(|face| face.families.iter().any(|(family, _)| family == "Unused"))
            .unwrap();
        assert_eq!(
            tree.fontdb()
                .with_face_data(unused.id, |bytes, _| bytes.is_empty()),
            Some(true),
            "font identities cannot eagerly acquire unused data"
        );
    }
    assert_eq!(
        assets.calls.borrow().len(),
        calls.len(),
        "viewport conversion cannot reopen application resources"
    );
}

#[test]
fn svg_font_denials_remain_typed_and_hidden_text_does_not_acquire_fonts() {
    for hidden in [false, true] {
        let assets = Assets {
            denied: true,
            hidden,
            ..Default::default()
        };
        let result = render_with_resolver(
            "<link rel='stylesheet' href='style.css'><img src='chart.svg'>",
            &options(),
            &assets,
        );
        if hidden {
            assert!(
                result.is_ok(),
                "hidden text cannot activate a denied font: {:?}",
                result.err()
            );
            assert_eq!(assets.calls.borrow().len(), 2);
        } else {
            let output = result.unwrap();
            assert!(
                matches!(output.diagnostics.as_slice(), [crate::RenderDiagnostic::UnavailableImage(failure)]
                if matches!(&failure.error, ImageError::Font(ferri_pdf_fonts::FontError::Resource(ResourceError::Denied(_))))),
                "SVG-only font denial must omit the image with its typed diagnostic"
            );
            assert_eq!(assets.calls.borrow().len(), 3);
        }
    }
}

#[test]
fn fallback_does_not_acquire_fonts_for_text_owned_by_a_different_svg_span() {
    struct Spans {
        denied: std::cell::Cell<usize>,
    }
    impl ResourceResolver for Spans {
        fn resolve(&self, reference: &str, base: &ResourceBase) -> Result<Resource, ResourceError> {
            let bytes = match reference {
                "primary.ttf" => include_bytes!("../../tests/fixtures/fonts/VariationSequenceTest.ttf").to_vec(),
                "denied.ttf" => { self.denied.set(self.denied.get() + 1); return Err(ResourceError::Denied(base.resolve(reference)?)); },
                "chart.svg" => b"<svg xmlns='http://www.w3.org/2000/svg' width='100' height='40'><text x='0' y='30' font-family='Primary,Denied'>A<tspan font-family='DejaVu Sans Mono'>\xE2\x86\x92</tspan>A</text></svg>".to_vec(),
                other => panic!("unexpected request {other}"),
            };
            Ok(Resource {
                bytes,
                base: base.clone(),
            })
        }
    }
    let assets = Spans {
        denied: std::cell::Cell::new(0),
    };
    let result = render_with_resolver(
        "<style>@font-face{font-family:Primary;src:url(primary.ttf)}@font-face{font-family:Denied;src:url(denied.ttf)}</style><img src='chart.svg'>",
        &options(),
        &assets,
    );
    assert!(
        result.is_ok(),
        "a differently styled arrow cannot activate the first span's denied fallback: {:?}",
        result.err()
    );
    assert_eq!(assets.denied.get(), 0);
}

#[test]
fn font_scopes_share_svg_source_bytes_and_decoded_raster_assets() {
    let assets = Assets::default();
    let output = render_with_resolver("<link rel='stylesheet' href='style.css'><style>img{display:block}</style><img src='chart.svg'><img src='pattern.png'><div><template shadowrootmode='open'><img src='chart.svg'><img src='pattern.png'></template></div>", &options(), &assets).unwrap();
    let calls = assets.calls.borrow();
    for reference in [
        "style.css",
        "chart.svg",
        "../fonts/chart.ttf",
        "pattern.png",
    ] {
        assert_eq!(
            calls.iter().filter(|(name, _)| name == reference).count(),
            1,
            "font contexts cannot duplicate acquisition of {reference}"
        );
    }
    let trace = output.layout.unwrap();
    let rasters = trace
        .display_lists
        .iter()
        .flatten()
        .filter_map(|item| match item {
            DisplayItem::Image(paint)
                if matches!(paint.image.as_ref(), ferri_pdf_images::Image::Raster(_)) =>
            {
                Some(&paint.image)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(rasters.len(), 2);
    assert!(
        std::sync::Arc::ptr_eq(rasters[0], rasters[1]),
        "a font namespace cannot duplicate a decoded raster"
    );
}

#[test]
fn svg_initial_and_unavailable_generic_classes_cannot_activate_css_aliases() {
    struct Denied {
        calls: std::cell::Cell<usize>,
    }
    impl ResourceResolver for Denied {
        fn resolve(&self, reference: &str, base: &ResourceBase) -> Result<Resource, ResourceError> {
            self.calls.set(self.calls.get() + 1);
            Err(ResourceError::Denied(base.resolve(reference)?))
        }
    }
    for (alias, attribute) in [
        ("serif", ""),
        ("Liberation Serif", ""),
        ("serif", "font-family='serif'"),
        ("cursive", "font-family='cursive'"),
        ("fantasy", "font-family='fantasy'"),
    ] {
        let assets = Denied {
            calls: std::cell::Cell::new(0),
        };
        let html = format!(
            "<style>@font-face{{font-family:'{alias}';src:url(denied.ttf)}}</style><svg width='100' height='40'><text x='0' y='30' {attribute}>H</text></svg>"
        );
        let result = render_with_resolver(&html, &options(), &assets);
        assert!(
            result.is_ok(),
            "initial/generic family {attribute:?} cannot activate alias {alias}: {:?}",
            result.err()
        );
        assert_eq!(assets.calls.get(), 0);
    }
    let assets = Denied {
        calls: std::cell::Cell::new(0),
    };
    let result = render_with_resolver(
        r#"<style>@font-face{font-family:'Liberation Serif';src:url(denied.ttf)}</style><svg width='100' height='80'><text x='0' y='30'>H</text><text x='0' y='60' font-family="'Liberation Serif'">H</text></svg>"#,
        &options(),
        &assets,
    );
    assert!(
        matches!(
            result,
            Err(crate::Error::Image(ImageError::Font(
                ferri_pdf_fonts::FontError::Resource(ResourceError::Denied(_))
            )))
        ),
        "an explicitly quoted family still acquires its requested source: {:?}",
        result.err()
    );
    assert_eq!(assets.calls.get(), 1);
}
