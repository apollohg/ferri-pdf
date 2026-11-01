mod common;

use common::local_options;
use ferri_pdf::{
    FontDescriptor, FontFaceStyle, FontWeight, LayoutTrace, RenderDiagnostic, RenderOptions,
    render_html,
};
use std::path::{Path, PathBuf};

const FAMILY: &str = "System Test";
const FACES: [&str; 4] = [
    "SystemTest-Regular.ttf",
    "SystemTest-Bold.ttf",
    "SystemTest-Italic.ttf",
    "SystemTest-BoldItalic.ttf",
];
const COLLECTION: &str = "SystemTest.ttc";
const TEXT: &str = "Hamburg";
const STYLE: &str = "@page{size:400pt 400pt;margin:10pt}body{margin:0}p{margin:0;font-size:20pt}";
/// The paragraph styles of the family's four faces, in `FACES` order.
const FACE_STYLES: [&str; 4] = [
    "",
    "font-weight:bold",
    "font-style:italic",
    "font-weight:bold;font-style:italic",
];

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/fonts")
        .join(name)
}

/// A folder of its own for `test`, holding copies of the fixture fonts `files`; removed on drop.
struct FontFolder(PathBuf);
impl FontFolder {
    fn new(test: &str, files: &[&str]) -> Self {
        let path =
            std::env::temp_dir().join(format!("ferri-pdf-fonts-{}-{test}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        for file in files {
            std::fs::copy(fixture(file), path.join(file)).unwrap();
        }
        Self(path)
    }
    fn options(&self) -> RenderOptions {
        let mut options = local_options();
        options.print.fonts.font_dirs = vec![self.0.clone()];
        options
    }
}
impl Drop for FontFolder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn paragraphs(family: &str) -> String {
    FACE_STYLES
        .iter()
        .map(|style| format!("<p style=\"font-family:{family};{style}\">{TEXT}</p>"))
        .collect()
}

/// The face and width of each line's single run.
fn lines(trace: &LayoutTrace) -> Vec<(FontDescriptor, f32)> {
    trace.pages[0]
        .iter()
        .map(|line| {
            assert_eq!(line.runs.len(), 1, "one run per line: {line:?}");
            let run = &line.runs[0];
            (trace.fonts[run.font_id.index()].clone(), run.width)
        })
        .collect()
}

fn weight(descriptor: &FontDescriptor) -> f32 {
    descriptor.weight.bounds(FontWeight::NORMAL).0.value()
}

fn messages(diagnostics: &[RenderDiagnostic]) -> Vec<String> {
    diagnostics.iter().map(ToString::to_string).collect()
}

fn missing_families(diagnostics: &[RenderDiagnostic]) -> Vec<&str> {
    diagnostics
        .iter()
        .filter_map(|diagnostic| match diagnostic {
            RenderDiagnostic::MissingFontFamily { family } => Some(family.as_str()),
            _ => None,
        })
        .collect()
}

/// The four faces in `FACES` order, each line's face named `family` with the expected
/// weight and style, and the widths of the lines.
fn assert_family_faces(trace: &LayoutTrace, family: &str) -> Vec<f32> {
    let expected = [
        (400.0, FontFaceStyle::Normal),
        (700.0, FontFaceStyle::Normal),
        (400.0, FontFaceStyle::Left),
        (700.0, FontFaceStyle::Left),
    ];
    let lines = lines(trace);
    assert_eq!(lines.len(), expected.len(), "{lines:?}");
    for ((descriptor, _), (face, (weight_value, style))) in
        lines.iter().zip(FACES.iter().zip(expected))
    {
        assert_eq!(descriptor.family, family, "{face}: {descriptor:?}");
        assert_eq!(weight(descriptor), weight_value, "{face}: {descriptor:?}");
        assert_eq!(descriptor.style, style, "{face}: {descriptor:?}");
    }
    lines.into_iter().map(|(_, width)| width).collect()
}

#[test]
fn installed_family_selects_its_faces_by_weight_and_style() {
    let folder = FontFolder::new("family", &FACES);
    let html = format!(
        "<!doctype html><style>{STYLE}</style>{}",
        paragraphs("'System Test'")
    );
    let output = render_html(&html, &folder.options()).unwrap();
    assert!(
        output.diagnostics.is_empty(),
        "{:?}",
        messages(&output.diagnostics)
    );
    let widths = assert_family_faces(&output.layout.unwrap(), FAMILY);
    assert!(
        widths[1] > widths[0] && widths[3] > widths[2],
        "bold faces have wider advances than regular ones: {widths:?}"
    );

    let without = render_html(&html, &local_options()).unwrap();
    assert_eq!(
        missing_families(&without.diagnostics),
        [FAMILY],
        "without the folder the family is missing: {:?}",
        messages(&without.diagnostics)
    );
}

#[test]
fn collection_faces_render_at_their_index_as_separate_fonts_do() {
    let separate = FontFolder::new("separate", &FACES);
    let collection = FontFolder::new("collection", &[COLLECTION]);
    let html = format!(
        "<!doctype html><style>{STYLE}</style>{}",
        paragraphs("'System Test'")
    );
    let expected = assert_family_faces(
        &render_html(&html, &separate.options())
            .unwrap()
            .layout
            .unwrap(),
        FAMILY,
    );
    let output = render_html(&html, &collection.options()).unwrap();
    assert!(
        output.diagnostics.is_empty(),
        "{:?}",
        messages(&output.diagnostics)
    );
    assert_eq!(
        assert_family_faces(&output.layout.unwrap(), FAMILY),
        expected,
        "each collection face shapes with its own advances"
    );
}

#[test]
fn local_sources_match_installed_full_and_postscript_names() {
    let folder = FontFolder::new("local", &FACES);
    let html = format!(
        "<!doctype html><style>{STYLE}\
         @font-face{{font-family:ByFullName;src:local('Absent Face'),local('system test bold')}}\
         @font-face{{font-family:ByPostScript;src:local(SystemTest-Italic)}}</style>\
         <p style=\"font-family:ByFullName\">{TEXT}</p>\
         <p style=\"font-family:ByPostScript\">{TEXT}</p>\
         <p style=\"font-family:'System Test';font-weight:bold\">{TEXT}</p>\
         <p style=\"font-family:'System Test';font-style:italic\">{TEXT}</p>"
    );
    let output = render_html(&html, &folder.options()).unwrap();
    assert!(
        output.diagnostics.is_empty(),
        "{:?}",
        messages(&output.diagnostics)
    );
    let lines = lines(&output.layout.unwrap());
    let families: Vec<_> = lines.iter().map(|(face, _)| face.family.as_str()).collect();
    assert_eq!(families, ["ByFullName", "ByPostScript", FAMILY, FAMILY]);
    assert_eq!(
        lines[0].1, lines[2].1,
        "local() by full name loads the bold face"
    );
    assert_eq!(
        lines[1].1, lines[3].1,
        "local() by PostScript name loads the italic face"
    );
    assert_ne!(lines[0].1, lines[1].1, "{lines:?}");
}

#[test]
fn author_faces_hide_the_installed_family_of_the_same_name() {
    let folder = FontFolder::new("hidden", &FACES);
    let mut options = folder.options();
    options.base = fixture("").into();
    let html = format!(
        "<!doctype html><style>{STYLE}\
         @font-face{{font-family:'system test';font-weight:300;src:url(WoffTest-Regular.ttf)}}\
         </style>{}",
        paragraphs("'System Test'")
    );
    let output = render_html(&html, &options).unwrap();
    assert!(
        output.diagnostics.is_empty(),
        "{:?}",
        messages(&output.diagnostics)
    );
    for (descriptor, _) in lines(&output.layout.unwrap()) {
        assert_eq!(
            (descriptor.family.as_str(), weight(&descriptor)),
            ("system test", 300.0),
            "the @font-face rule defines the family, even for bold requests (CSS Fonts 4 §4.2)"
        );
    }
}

#[test]
fn installed_families_serve_generic_and_fallback_preferences() {
    let folder = FontFolder::new("preferences", &FACES);
    let mut options = folder.options();
    options.print.fonts.sans_serif = vec![FAMILY.into()];
    options.print.fonts.fallback = vec![FAMILY.into()];
    let html = format!(
        "<!doctype html><style>{STYLE}</style>\
         <p style=\"font-family:sans-serif;font-weight:bold\">{TEXT}</p>\
         <p style=\"font-family:serif\">{TEXT}</p>"
    );
    let output = render_html(&html, &options).unwrap();
    let lines = lines(&output.layout.unwrap());
    assert_eq!(
        (lines[0].0.family.as_str(), weight(&lines[0].0)),
        (FAMILY, 700.0)
    );
    assert_eq!(
        lines[1].0.family, "Liberation Serif",
        "a fallback family does not replace a generic family that covers the text"
    );
}

#[test]
fn missing_families_warn_once_only_while_no_earlier_family_matched() {
    let folder = FontFolder::new("missing", &FACES);
    let html = format!(
        "<!doctype html><style>{STYLE}</style>\
         <p style=\"font-family:'Absent One','Absent Two',serif\">{TEXT}</p>\
         <p style=\"font-family:'absent one',serif\">{TEXT}</p>\
         <p style=\"font-family:'System Test','Absent Three'\">{TEXT}</p>\
         <p style=\"font-family:serif,'Absent Four'\">{TEXT}</p>\
         <p style=\"font-family:'Absent Five','System Test','Absent Six'\">{TEXT}</p>\
         <p style=\"font-family:sans-serif\">{TEXT}</p>"
    );
    let output = render_html(&html, &folder.options()).unwrap();
    assert_eq!(
        missing_families(&output.diagnostics),
        ["Absent One", "Absent Two", "Absent Five"],
        "{:?}",
        messages(&output.diagnostics)
    );
    assert!(
        output
            .diagnostics
            .iter()
            .all(|diagnostic| !diagnostic.loses_content())
    );
    let families: Vec<_> = lines(&output.layout.unwrap())
        .into_iter()
        .map(|(face, _)| face.family)
        .collect();
    assert_eq!(
        families,
        [
            "Liberation Serif",
            "Liberation Serif",
            FAMILY,
            "Liberation Serif",
            FAMILY,
            "Liberation Sans"
        ]
    );
}

#[test]
fn an_unreadable_font_folder_warns_and_the_pdf_is_still_written() {
    let folder = FontFolder::new("unreadable", &FACES);
    let missing = folder.0.join("absent");
    let mut options = folder.options();
    options.print.fonts.font_dirs.push(missing.clone());
    let html = format!(
        "<!doctype html><style>{STYLE}</style>{}",
        paragraphs("'System Test'")
    );
    let output = render_html(&html, &options).unwrap();
    assert!(output.pdf.starts_with(b"%PDF-"));
    let unreadable: Vec<_> = output
        .diagnostics
        .iter()
        .filter_map(|diagnostic| match diagnostic {
            RenderDiagnostic::UnreadableFontDirectory { path, .. } => Some(path),
            _ => None,
        })
        .collect();
    assert_eq!(
        unreadable,
        [&missing],
        "{:?}",
        messages(&output.diagnostics)
    );
    assert!(
        output
            .diagnostics
            .iter()
            .all(|diagnostic| !diagnostic.loses_content())
    );
    assert_family_faces(&output.layout.unwrap(), FAMILY);
}

#[test]
fn an_installed_file_that_no_longer_loads_falls_back_with_a_font_failure() {
    let folder = FontFolder::new("vanished", &[FACES[0]]);
    let options = folder.options();
    let html = format!(
        "<!doctype html><style>{STYLE}</style><p style=\"font-family:'System Test',serif\">{TEXT}</p>"
    );
    // The first render scans the folder; the file then changes before a render reads it.
    render_html("<!doctype html><p>scan</p>", &options).unwrap();
    std::fs::write(folder.0.join(FACES[0]), b"not a font").unwrap();
    let output = render_html(&html, &options).unwrap();
    let failures: Vec<_> = output
        .diagnostics
        .iter()
        .filter(|diagnostic| matches!(diagnostic, RenderDiagnostic::UnavailableFont(_)))
        .map(ToString::to_string)
        .collect();
    assert_eq!(failures.len(), 1, "{:?}", messages(&output.diagnostics));
    assert!(failures[0].contains(FACES[0]), "{failures:?}");
    assert!(
        missing_families(&output.diagnostics).is_empty(),
        "the family exists, though its file failed: {:?}",
        messages(&output.diagnostics)
    );
    let lines = lines(&output.layout.unwrap());
    assert_eq!(lines[0].0.family, "Liberation Serif");
}

#[test]
fn system_fonts_render() {
    let mut options = local_options();
    options.print.fonts.system_fonts = true;
    let html = format!(
        "<!doctype html><style>{STYLE}</style>{}",
        paragraphs("sans-serif")
    );
    let output = render_html(&html, &options).unwrap();
    assert_eq!(output.page_count, 1);
}
