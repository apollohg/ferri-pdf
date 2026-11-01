use crate::fonts::{FontBook, FontData, FontPreferences, installed};
use crate::work::{self, Work};
use crate::{RenderOptions, render_html};
use std::path::{Path, PathBuf};

const HTML: &str = "<!doctype html><p style=\"font-family:'System Test',serif\">Installed</p>";
const FIXTURE: &str = "SystemTest-Regular.ttf";
const FAMILY: &str = "System Test";
/// The installed-font fixture family: four faces in files of their own, then all four in one
/// collection.
const FAMILY_FILES: [&str; 5] = [
    "SystemTest-Regular.ttf",
    "SystemTest-Bold.ttf",
    "SystemTest-Italic.ttf",
    "SystemTest-BoldItalic.ttf",
    "SystemTest.ttc",
];
const COLLECTION_FACES: usize = 4;
/// A scan of the fixture family reads at most this fraction of its bytes (one over this):
/// headers, table directories and descriptive tables, but no outlines or layout tables.
const SCANNED_FRACTION_DIVISOR: u64 = 8;
/// Where the italic fixture is cut inside its table directory, which ends at byte 236.
const DIRECTORY_CUT: usize = 100;
/// Where the italic fixture is cut before its `name` table, which starts at byte 8704.
const TABLES_CUT: usize = 1024;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/fonts")
        .join(name)
}

/// A new folder holding copies of the fixture fonts `files`, which no other test scans.
fn folder(test: &str, files: &[&str]) -> PathBuf {
    let path =
        std::env::temp_dir().join(format!("ferri-pdf-installed-{}-{test}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).unwrap();
    for file in files {
        std::fs::copy(fixture(file), path.join(file)).unwrap();
    }
    path
}

fn scans(options: &RenderOptions) -> usize {
    let (rendered, count) = work::measured(Work::FontScan, || render_html(HTML, options));
    assert!(rendered.is_ok(), "{:?}", rendered.err());
    count
}

/// The faces a scan of `folder` finds, as file, face index and family, and the bytes it read.
fn scanned(folder: &Path) -> (Vec<(PathBuf, u32, String)>, u64) {
    let preferences = FontPreferences {
        font_dirs: vec![folder.to_path_buf()],
        ..FontPreferences::default()
    };
    let (fonts, bytes) = work::measured(Work::FontScanBytes, || {
        installed::installed(&preferences).expect("a font folder is scanned")
    });
    let faces = fonts
        .book_faces()
        .iter()
        .map(|face| {
            let FontData::Installed(source) = &face.data else {
                panic!(
                    "an installed face is read from its file: {:?}",
                    face.descriptor
                );
            };
            (
                source.path.to_path_buf(),
                source.index,
                face.descriptor.family.clone(),
            )
        })
        .collect();
    (faces, u64::try_from(bytes).unwrap())
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
    let first = folder("cache-first", &[FIXTURE]);
    let second = folder("cache-second", &[FIXTURE]);
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

#[test]
fn a_scan_reads_only_the_headers_and_descriptive_tables_of_each_face() {
    let folder = folder("headers", &FAMILY_FILES);
    let total: u64 = FAMILY_FILES
        .iter()
        .map(|file| std::fs::metadata(folder.join(file)).unwrap().len())
        .sum();
    let (faces, bytes) = scanned(&folder);
    assert_eq!(
        faces.len(),
        FAMILY_FILES.len() - 1 + COLLECTION_FACES,
        "every face of every file is found: {faces:#?}"
    );
    assert!(
        faces.iter().all(|(_, _, family)| family == FAMILY),
        "every face is of the fixture family: {faces:#?}"
    );
    assert!(
        bytes > 0 && bytes * SCANNED_FRACTION_DIVISOR <= total,
        "the scan read {bytes} of the {total} bytes of {} files, more than 1/\
         {SCANNED_FRACTION_DIVISOR} of them",
        FAMILY_FILES.len()
    );
    std::fs::remove_dir_all(folder).unwrap();
}

#[cfg(unix)]
#[test]
fn a_scan_skips_broken_files_and_judges_links_by_their_targets() {
    use std::os::unix::fs::symlink;
    let folder = folder("broken", &[FIXTURE]);
    let targets = folder.with_extension("targets");
    let _ = std::fs::remove_dir_all(&targets);
    std::fs::create_dir_all(&targets).unwrap();
    std::fs::copy(
        fixture("SystemTest-Bold.ttf"),
        targets.join("SystemTest-Bold.ttf"),
    )
    .unwrap();
    std::fs::copy(
        fixture("SystemTest-Italic.ttf"),
        targets.join("italic.data"),
    )
    .unwrap();
    let sub = folder.join("sub");
    std::fs::create_dir(&sub).unwrap();
    std::fs::copy(
        fixture("SystemTest-BoldItalic.ttf"),
        sub.join("BOLDITALIC.TTF"),
    )
    .unwrap();
    std::fs::copy(fixture("SystemTest-BoldItalic.ttf"), sub.join("mixed.Ttf")).unwrap();
    let italic = std::fs::read(fixture("SystemTest-Italic.ttf")).unwrap();
    std::fs::write(folder.join("directory-cut.ttf"), &italic[..DIRECTORY_CUT]).unwrap();
    std::fs::write(folder.join("tables-cut.ttf"), &italic[..TABLES_CUT]).unwrap();
    std::fs::write(
        folder.join("garbage.otf"),
        b"not a font, though named like one",
    )
    .unwrap();
    // A collection claiming 2^32 - 1 faces, in a file too short to list them.
    std::fs::write(
        folder.join("endless.ttc"),
        [b"ttcf".as_slice(), &[0, 1, 0, 0], &u32::MAX.to_be_bytes()].concat(),
    )
    .unwrap();
    symlink(&folder, sub.join("loop")).unwrap();
    symlink(folder.join("missing.ttf"), folder.join("dangling.ttf")).unwrap();
    symlink(
        targets.join("SystemTest-Bold.ttf"),
        folder.join("bold.link"),
    )
    .unwrap();
    symlink(targets.join("italic.data"), folder.join("italic.ttf")).unwrap();

    let (faces, _) = scanned(&folder);
    let canonical = |path: PathBuf| std::fs::canonicalize(path).unwrap();
    let mut expected: Vec<(PathBuf, u32, String)> = [
        canonical(folder.join(FIXTURE)),
        canonical(sub.join("BOLDITALIC.TTF")),
        canonical(targets.join("SystemTest-Bold.ttf")),
    ]
    .into_iter()
    .map(|path| (path, 0, FAMILY.to_owned()))
    .collect();
    expected.sort();
    assert_eq!(
        faces, expected,
        "the scan keeps the valid fixture, the upper-case extension and the linked .ttf target, \
         ordered by path; it skips the cut, garbage and endless files, the mixed-case \
         extension, the dangling link, the link to a .data file, and ends at the folder loop"
    );
    std::fs::remove_dir_all(folder).unwrap();
    std::fs::remove_dir_all(targets).unwrap();
}
