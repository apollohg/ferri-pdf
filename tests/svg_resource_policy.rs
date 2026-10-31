//! SVG content that CSS `url()` references, or that a font carries, reaches resources only
//! through the caller's resolver, never the filesystem.
use ferri_pdf::{
    RenderDiagnostic, RenderOptions, RenderOutput, Resource, ResourceBase, ResourceError,
    ResourceResolver, render_with_resolver,
};
use std::{cell::RefCell, collections::HashMap, path::PathBuf};

const IMAGE: &[u8] = include_bytes!("oracle/assets/image-pattern.png");
const SVG_GLYPH_FONT: &[u8] = include_bytes!("fixtures/fonts/SvgGlyphTest.ttf");
const SVG_GLYPH_FONT_REFERENCE: &str = "SvgGlyphTest.ttf";
/// A directory no test reads, so that only absolute references name real files.
const VIRTUAL_BASE: &str = "/ferri-pdf-virtual";
/// The image the fixture's `F` glyph references by a path relative to the working directory,
/// which `cargo test` sets to the package root.
const SVG_GLYPH_FILE_IMAGE: &str = "tests/oracle/assets/image-pattern.png";
/// The fixture glyphs whose SVG documents reference an image file and a `data:` URL.
const SVG_GLYPH_FILE: char = 'F';
const SVG_GLYPH_DATA: char = 'D';
/// A glyph that draws nothing.
const BLANK_GLYPH: char = ' ';

/// Records every request and serves only the references it allows.
#[derive(Default)]
struct Policy {
    allowed: HashMap<String, Vec<u8>>,
    requests: RefCell<Vec<String>>,
}
impl Policy {
    fn allowing(entries: impl IntoIterator<Item = (String, Vec<u8>)>) -> Self {
        Self {
            allowed: entries.into_iter().collect(),
            requests: RefCell::default(),
        }
    }
    fn requests_of(&self, reference: &str) -> usize {
        self.requests
            .borrow()
            .iter()
            .filter(|request| *request == reference)
            .count()
    }
}
impl ResourceResolver for Policy {
    fn resolve(&self, reference: &str, base: &ResourceBase) -> Result<Resource, ResourceError> {
        self.requests.borrow_mut().push(reference.to_owned());
        match self.allowed.get(reference) {
            Some(bytes) => Ok(Resource {
                bytes: bytes.clone(),
                base: base.clone(),
            }),
            None => Err(ResourceError::Denied(base.resolve(reference)?)),
        }
    }
}

/// A file outside the document's base, removed when dropped.
struct SecretFile(PathBuf);
impl SecretFile {
    fn new(case: &str) -> Self {
        let path = secret_path(case, "present");
        std::fs::write(&path, IMAGE).expect("write the secret image");
        Self(path)
    }
    fn href(&self) -> String {
        self.0
            .to_str()
            .expect("temporary paths are UTF-8")
            .to_owned()
    }
}
impl Drop for SecretFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn secret_path(case: &str, state: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "ferri-pdf-svg-policy-{}-{case}-{state}.png",
        std::process::id()
    ))
}

fn render(html: &str, policy: &Policy) -> RenderOutput {
    render_with_resolver(
        html,
        &RenderOptions {
            base: VIRTUAL_BASE.into(),
            ..RenderOptions::default()
        },
        policy,
    )
    .expect("rendering never fails because of resources")
}

fn unavailable(output: &RenderOutput, reference: &str) -> bool {
    output.diagnostics.iter().any(|diagnostic| {
        matches!(diagnostic, RenderDiagnostic::UnavailableImage(failure) if failure.reference == reference)
    })
}

/// A page whose box `css` references SVG element `definition`, in which `{href}` names an image.
fn box_page(css: &str, definition: &str) -> impl Fn(&str) -> String {
    move |href| {
        format!(
            "<!doctype html><style>@page{{size:60pt 60pt;margin:0}}body{{margin:0}}\
             #target{{width:60pt;height:60pt;background:rgb(0,0,255);{css}}}</style>\
             <svg width='0' height='0' style='position:absolute'>{}</svg><div id='target'></div>",
            definition.replace("{href}", href)
        )
    }
}

/// The image that `page` draws for a reference is requested from the resolver when `drawn`,
/// never read from disk, and drawn only when the resolver allows it.
fn assert_reference_images_follow_policy(case: &str, page: impl Fn(&str) -> String, drawn: bool) {
    let secret = SecretFile::new(case);
    let missing = secret_path(case, "absent");
    assert!(
        !missing.exists(),
        "{case}: {} must not exist",
        missing.display()
    );
    let missing = missing.to_str().expect("temporary paths are UTF-8");

    let policy = Policy::default();
    let denied = render(&page(&secret.href()), &policy);
    let requests = policy.requests.borrow().clone();
    let control = render(&page(missing), &Policy::default());
    assert!(
        denied.pdf == control.pdf,
        "{case}: the denied image's file contents changed the PDF, so it was read from disk; \
         resolver requests: {requests:?}; diagnostics: {:?}",
        denied.diagnostics
    );
    assert_eq!(
        policy.requests_of(&secret.href()),
        usize::from(drawn),
        "{case}: the resolver must see one request for the image exactly when it would draw; \
         requests: {requests:?}"
    );
    assert_eq!(
        unavailable(&denied, &secret.href()),
        drawn,
        "{case}: a denied image reports its unavailability when it would draw; diagnostics: {:?}",
        denied.diagnostics
    );

    let policy = Policy::allowing([(secret.href(), IMAGE.to_vec())]);
    let allowed = render(&page(&secret.href()), &policy);
    assert_eq!(
        allowed.pdf != denied.pdf,
        drawn,
        "{case}: an allowed image draws through the resolver exactly when it would draw; \
         requests: {:?}; diagnostics: {:?}",
        policy.requests.borrow(),
        allowed.diagnostics
    );
    assert!(
        !unavailable(&allowed, &secret.href()),
        "{case}: an allowed image is available; diagnostics: {:?}",
        allowed.diagnostics
    );
}

#[test]
fn css_referenced_mask_images_load_only_through_the_resolver() {
    assert_reference_images_follow_policy(
        "mask",
        box_page(
            "mask:url(#m)",
            "<mask id='m'><image href='{href}' width='80' height='80'/></mask>",
        ),
        true,
    );
}

#[test]
fn css_referenced_filter_images_load_only_through_the_resolver() {
    assert_reference_images_follow_policy(
        "filter",
        box_page(
            "filter:url(#f)",
            "<filter id='f'><feImage href='{href}'/></filter>",
        ),
        true,
    );
}

/// A `clipPath` draws no images (CSS Masking 1 §6.1 content model), so its image is neither
/// fetched nor read.
#[test]
fn css_referenced_clip_path_images_are_never_loaded() {
    assert_reference_images_follow_policy(
        "clip-path",
        box_page(
            "clip-path:url(#c)",
            "<clipPath id='c'><rect width='30' height='30'/><image href='{href}' width='80' height='80'/></clipPath>",
        ),
        false,
    );
}

/// CSS mask layers of SVG content, which paint as the document's do, reference the mask element
/// while the document's SVG definitions are still being collected.
#[test]
fn css_mask_layers_of_svg_content_load_their_images_only_through_the_resolver() {
    assert_reference_images_follow_policy(
        "svg-content-mask",
        |href| {
            format!(
                "<!doctype html><style>@page{{size:60pt 60pt;margin:0}}body{{margin:0}}\
                 rect{{mask:url(#m),linear-gradient(white,white)}}</style><svg width='80' height='80'>\
                 <mask id='m'><image href='{href}' width='80' height='80'/></mask>\
                 <rect width='80' height='80' fill='blue'/></svg>"
            )
        },
        true,
    );
}

/// A page whose SVG text shows `glyph` in the fixture's face. A filter rasterizes its content
/// from the text's flattened glyphs, SVG glyph documents included; the rectangle keeps the
/// filtered group's bounds when the glyph draws nothing.
fn svg_glyph_page(glyph: char) -> String {
    format!(
        "<!doctype html><style>@font-face{{font-family:SvgGlyph;src:url({SVG_GLYPH_FONT_REFERENCE})}}\
         @page{{size:60pt 60pt;margin:0}}body{{margin:0}}</style>\
         <svg width='60' height='60'><filter id='r' filterUnits='userSpaceOnUse' x='0' y='0' width='60' height='60'><feOffset/></filter>\
         <g filter='url(#r)'><rect width='1' height='1'/><text x='0' y='40' font-family='SvgGlyph' font-size='40'>{glyph}</text></g></svg>"
    )
}

/// OpenType SVG glyph documents may not reference external resources (OpenType `SVG ` table,
/// "Glyph semantics and metrics"): only `data:` URLs draw, without the resolver.
#[test]
fn svg_glyph_images_draw_only_from_data_urls() {
    assert!(
        std::path::Path::new(SVG_GLYPH_FILE_IMAGE).exists(),
        "the test runs from the package root, where the glyph's relative image path names a file"
    );
    let render_glyph = |glyph| {
        let policy =
            Policy::allowing([(SVG_GLYPH_FONT_REFERENCE.to_owned(), SVG_GLYPH_FONT.to_vec())]);
        let output = render(&svg_glyph_page(glyph), &policy);
        assert_eq!(
            policy.requests.borrow().as_slice(),
            [SVG_GLYPH_FONT_REFERENCE],
            "{glyph:?}: an SVG glyph's image reference never reaches the resolver"
        );
        output
    };
    let blank = render_glyph(BLANK_GLYPH);
    let file = render_glyph(SVG_GLYPH_FILE);
    assert!(
        file.pdf == blank.pdf,
        "the {SVG_GLYPH_FILE:?} glyph drew {SVG_GLYPH_FILE_IMAGE} from disk; diagnostics: {:?}",
        file.diagnostics
    );
    let data = render_glyph(SVG_GLYPH_DATA);
    assert!(
        data.pdf != blank.pdf,
        "the {SVG_GLYPH_DATA:?} glyph draws its data URL image; diagnostics: {:?}",
        data.diagnostics
    );
}
