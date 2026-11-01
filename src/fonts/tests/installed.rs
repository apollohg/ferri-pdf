use crate::fonts::{FontBook, FontPreferences};
use crate::work::{self, Work};
use crate::{RenderOptions, render_html};
use std::path::PathBuf;

const HTML: &str = "<!doctype html><p style=\"font-family:'System Test',serif\">Installed</p>";
const FIXTURE: &str = "SystemTest-Regular.ttf";

/// A new folder holding the regular installed-font fixture, which no other test scans.
fn folder(test: &str) -> PathBuf {
    let path =
        std::env::temp_dir().join(format!("ferri-pdf-installed-{}-{test}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).unwrap();
    std::fs::copy(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/fonts")
            .join(FIXTURE),
        path.join(FIXTURE),
    )
    .unwrap();
    path
}

fn scans(options: &RenderOptions) -> usize {
    let (rendered, count) = work::measured(Work::FontScan, || render_html(HTML, options));
    assert!(rendered.is_ok(), "{:?}", rendered.err());
    count
}

#[test]
fn default_preferences_never_scan_for_installed_fonts() {
    let (book, book_scans) = work::measured(Work::FontScan, || {
        FontBook::with_preferences(FontPreferences::default())
    });
    assert_eq!(
        book_scans, 0,
        "building the default font book scans nothing"
    );
    assert!(book.installed.is_none() && book.installed_faces.is_empty());
    assert_eq!(
        scans(&RenderOptions::default()),
        0,
        "a render with default options scans nothing"
    );
}

#[test]
fn one_scan_per_process_serves_every_render_with_the_same_folders() {
    let first = folder("cache-first");
    let second = folder("cache-second");
    let mut options = RenderOptions::default();
    options.print.fonts.font_dirs = vec![first.clone()];
    assert_eq!(scans(&options), 1, "the first render scans its folder");
    assert_eq!(scans(&options), 0, "a second render reuses the scan");
    options.print.fonts.font_dirs.push(second.clone());
    assert_eq!(scans(&options), 1, "another set of folders is scanned once");
    assert_eq!(scans(&options), 0);
    for path in [first, second] {
        std::fs::remove_dir_all(path).unwrap();
    }
}
