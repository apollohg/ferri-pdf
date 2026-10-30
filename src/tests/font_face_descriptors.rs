//! `@font-face` descriptors applied once a face is matched: `unicode-range` (CSS Fonts 4
//! §4.5), the metric overrides (§4.11), `font-display` (§4.9) and `size-adjust` (CSS Fonts 5
//! §4.4). Expected values come from the fixture fonts' own tables.
use super::common;
use crate::layout::{LayoutTrace, TextLine};
use crate::{RenderOptions, Resource, ResourceBase, ResourceError, ResourceResolver};
use std::cell::RefCell;
use std::path::Path;

const SERIF: &str = "LiberationSerif-Regular.ttf";
const SANS: &str = "DejaVuSans.ttf";
const SIZE_PT: f32 = 20.0;
const BOX_COLOR: [u8; 3] = [1, 2, 3];

/// Serves the bundled font files and records every font reference requested.
struct FontFiles(RefCell<Vec<String>>);
impl ResourceResolver for FontFiles {
    fn resolve(&self, reference: &str, _: &ResourceBase) -> Result<Resource, ResourceError> {
        self.0.borrow_mut().push(reference.into());
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("assets/fonts")
            .join(reference);
        let bytes = std::fs::read(&path).map_err(|source| ResourceError::Read {
            path: path.clone(),
            source,
        })?;
        Ok(Resource {
            bytes,
            base: path.parent().unwrap().into(),
        })
    }
}

fn render(faces: &str, body: &str) -> (LayoutTrace, Vec<String>) {
    let files = FontFiles(RefCell::new(Vec::new()));
    let html = format!(
        "<style>{faces}@page{{size:400pt 300pt;margin:20pt}}body{{margin:0}}p{{margin:0;font-size:{SIZE_PT}pt}}</style>{body}"
    );
    let options = RenderOptions {
        include_layout: true,
        ..Default::default()
    };
    let trace = crate::render_with_resolver(&html, &options, &files)
        .unwrap_or_else(|error| panic!("{faces} {body}: {error}"))
        .layout
        .unwrap();
    (trace, files.0.into_inner())
}

fn first_line(trace: &LayoutTrace) -> &TextLine {
    &trace.pages[0][0]
}

fn font(name: &str) -> ttf_parser::Face<'static> {
    let bytes = std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("assets/fonts")
            .join(name),
    )
    .unwrap()
    .leak();
    ttf_parser::Face::parse(bytes, 0).unwrap()
}

/// The horizontal advance of `ch`, in em: the two fixture faces share glyph IDs but not
/// advances.
fn advance(face: &ttf_parser::Face<'_>, ch: char) -> f32 {
    let units = face
        .glyph_hor_advance(face.glyph_index(ch).unwrap())
        .unwrap();
    f32::from(units) / f32::from(face.units_per_em())
}

fn advances(line: &TextLine) -> Vec<f32> {
    line.runs
        .iter()
        .flat_map(|run| run.glyphs().iter().map(|glyph| glyph.x_advance))
        .collect()
}

fn em(face: &ttf_parser::Face<'_>, units: i16) -> f32 {
    f32::from(units) / f32::from(face.units_per_em())
}

#[test]
fn unicode_range_composes_faces_and_never_loads_faces_out_of_range() {
    let faces = format!(
        "@font-face{{font-family:Mixed;src:url({SERIF})}}\
         @font-face{{font-family:Mixed;src:url({SANS});unicode-range:U+41,U+5A-5B}}\
         @font-face{{font-family:Mixed;src:url(Ideographs.ttf);unicode-range:U+4E00-9FFF}}"
    );
    let (trace, requested) = render(&faces, "<p style=\"font-family:Mixed\">ABZ</p>");
    let (serif, sans) = (font(SERIF), font(SANS));
    assert_eq!(
        advances(first_line(&trace)),
        [
            advance(&sans, 'A'),
            advance(&serif, 'B'),
            advance(&sans, 'Z')
        ],
        "A and Z are in the later face's range; B only in the face without a range"
    );
    assert!(
        !requested.iter().any(|name| name == "Ideographs.ttf"),
        "a face whose range excludes the text must not be loaded: {requested:?}"
    );
}

#[test]
fn the_first_available_font_is_the_first_face_whose_range_includes_space() {
    let faces = format!(
        "@font-face{{font-family:Mixed;src:url({SERIF})}}\
         @font-face{{font-family:Mixed;src:url({SANS});unicode-range:U+21-7E}}"
    );
    let (trace, _) = render(
        &faces,
        &format!(
            "<div style=\"font:{SIZE_PT}pt Mixed;width:10ex;height:5pt;background:rgb(1,2,3)\"></div>"
        ),
    );
    let serif = font(SERIF);
    let expected = 10.0 * em(&serif, serif.x_height().unwrap()) * SIZE_PT;
    common::assert_labelled_pt(
        "10ex from the serif face, the first whose range includes U+0020",
        common::painted_box(&trace, BOX_COLOR).width,
        expected,
    );
}

#[test]
fn size_adjust_scales_glyphs_and_metrics_but_not_the_computed_size() {
    let faces = format!(
        "@font-face{{font-family:Half;src:url({SERIF});size-adjust:50%}}\
         @font-face{{font-family:Whole;src:url({SERIF})}}"
    );
    let line = |family: &str, extra: &str| {
        let (trace, _) = render(
            &faces,
            &format!("<p style=\"font-family:{family};line-height:normal;{extra}\">H</p>"),
        );
        first_line(&trace).clone()
    };
    let (half, whole) = (line("Half", ""), line("Whole", ""));
    common::assert_labelled_pt("used glyph size", half.runs[0].font_size, SIZE_PT / 2.0);
    common::assert_labelled_pt("advance", half.width, whole.width / 2.0);
    common::assert_labelled_pt("normal line height", half.height, whole.height / 2.0);
    let (trace, _) = render(
        &faces,
        "<div style=\"font:20pt Half\"><div style=\"width:1em;height:1ex;background:rgb(1,2,3)\"></div></div>",
    );
    let serif = font(SERIF);
    let painted = common::painted_box(&trace, BOX_COLOR);
    common::assert_labelled_pt("em keeps the computed size", painted.width, SIZE_PT);
    common::assert_labelled_pt(
        "ex follows the adjusted face",
        painted.height,
        em(&serif, serif.x_height().unwrap()) * SIZE_PT / 2.0,
    );
    let adjust = "font-size-adjust:0.5";
    common::assert_labelled_pt(
        "font-size-adjust applies after size-adjust",
        line("Half", adjust).runs[0].font_size,
        line("Whole", adjust).runs[0].font_size,
    );
}

#[test]
fn metric_overrides_replace_line_metrics_at_the_used_size() {
    let faces = format!(
        "@font-face{{font-family:Overridden;src:url({SERIF});ascent-override:100%;descent-override:50%;line-gap-override:0%}}\
         @font-face{{font-family:Scaled;src:url({SERIF});ascent-override:100%;descent-override:50%;line-gap-override:normal;size-adjust:50%}}"
    );
    let (trace, _) = render(
        &faces,
        "<p style=\"font-family:Overridden;line-height:normal\">H</p>",
    );
    common::assert_labelled_pt(
        "ascent + descent + gap",
        first_line(&trace).height,
        SIZE_PT * 1.5,
    );
    let (trace, _) = render(
        &faces,
        "<p style=\"font-family:Scaled;line-height:normal\">H</p>",
    );
    let serif = font(SERIF);
    common::assert_labelled_pt(
        "overrides resolve against the adjusted size; normal keeps the font's gap",
        first_line(&trace).height,
        SIZE_PT / 2.0 * (1.5 + em(&serif, serif.line_gap())),
    );
}

#[test]
fn every_font_display_value_renders_the_loaded_face() {
    for value in ["auto", "block", "swap", "fallback", "optional"] {
        let (trace, requested) = render(
            &format!("@font-face{{font-family:Shown;src:url({SANS});font-display:{value}}}"),
            "<p style=\"font-family:Shown,serif\">B</p>",
        );
        assert_eq!(requested, [SANS], "font-display:{value}");
        let run = &first_line(&trace).runs[0];
        assert_eq!(
            trace.fonts[run.font_id.index()].family,
            "Shown",
            "font-display:{value} must render with the loaded face, not a fallback"
        );
    }
}
