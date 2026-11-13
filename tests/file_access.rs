//! Local files are read only inside the folder the document's base names, or `file_root`.
use ferri_pdf::{
    FileResolver, HtmlPart, RenderOptions, ResourceBase, ResourceError, ResourceResolver, Url,
    render_html, render_parts,
};
use std::{
    fs,
    path::{Path, PathBuf},
};

/// Puts the second paragraph on a second page, so a loaded sheet shows in the page count.
const SHEET: &str = "p + p { break-before: page }";
const PAGES_WITH_SHEET: usize = 2;
const PAGES_WITHOUT_SHEET: usize = 1;
const OUTSIDE_FOLDER: &str = "outside the allowed folder";
const NO_ROOT: &str = "no local base or file root";
const NOT_LOCAL: &str = "the content that names it is not local";

/// A temporary tree: `root` is the document's folder, with `secret.css` beside it and a
/// `sibling` folder; a sheet anywhere in it breaks the page when it loads.
struct Sandbox {
    directory: PathBuf,
    root: PathBuf,
    secret: PathBuf,
}
impl Sandbox {
    fn new(case: &str) -> Self {
        let directory = new_directory(&std::env::temp_dir().join(format!(
            "ferri-pdf-file-access-{case}-{}",
            std::process::id()
        )));
        let root = directory.join("root");
        fs::create_dir_all(root.join("sub")).unwrap();
        fs::create_dir_all(directory.join("sibling")).unwrap();
        let secret = directory.join("secret.css");
        for sheet in [
            &secret,
            &root.join("style.css"),
            &directory.join("sibling/shared.css"),
        ] {
            fs::write(sheet, SHEET).unwrap();
        }
        fs::write(root.join("sub/inside.css"), "@import '../style.css';").unwrap();
        fs::write(root.join("sub/escape.css"), "@import '../../secret.css';").unwrap();
        Self {
            directory,
            root,
            secret,
        }
    }
    fn options(&self) -> RenderOptions {
        RenderOptions {
            base: self.root.as_path().into(),
            ..RenderOptions::default()
        }
    }
}
impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}

/// Creates `directory` anew and resolves the links in the temporary folder's own path.
fn new_directory(directory: &Path) -> PathBuf {
    let _ = fs::remove_dir_all(directory);
    fs::create_dir_all(directory).unwrap();
    directory.canonicalize().unwrap()
}

fn document(href: &str) -> String {
    format!("<link rel=stylesheet href='{href}'><p>one</p><p>two</p>")
}

fn file_url(path: &Path) -> String {
    Url::from_file_path(path).unwrap().into()
}

/// Whether the sheet `href` names loaded, and the diagnostics of the render.
fn linking(href: &str, options: &RenderOptions) -> (bool, String) {
    let output = render_html(&document(href), options).expect("a refused file never fails");
    let diagnostics = output
        .diagnostics
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        [PAGES_WITH_SHEET, PAGES_WITHOUT_SHEET].contains(&output.page_count),
        "{href}: {} pages; {diagnostics}",
        output.page_count
    );
    (output.page_count == PAGES_WITH_SHEET, diagnostics)
}

fn assert_refused(href: &str, options: &RenderOptions, reason: &str, case: &str) {
    let (loaded, diagnostics) = linking(href, options);
    assert!(!loaded, "{case}: {href} was read; {diagnostics}");
    assert!(
        diagnostics.contains(href) && diagnostics.contains(reason),
        "{case}: the diagnostic must name {href} and say {reason:?}: {diagnostics}"
    );
}

fn assert_loaded(href: &str, options: &RenderOptions, case: &str) {
    let (loaded, diagnostics) = linking(href, options);
    assert!(loaded, "{case}: {href} was not read; {diagnostics}");
    assert_eq!(diagnostics, "", "{case}: {href}");
}

#[test]
fn only_files_inside_the_base_folder_are_read() {
    let sandbox = Sandbox::new("base");
    let options = sandbox.options();
    assert_loaded("style.css", &options, "a file in the base folder");
    assert_loaded(
        &file_url(&sandbox.root.join("style.css")),
        &options,
        "a file: URL into the base folder",
    );
    let secret = sandbox.secret.to_str().unwrap();
    for href in [
        secret,
        &file_url(&sandbox.secret),
        "../secret.css",
        "sub/../../secret.css",
    ] {
        assert_refused(href, &options, OUTSIDE_FOLDER, "outside the base folder");
    }
    let by_url = RenderOptions {
        base: Url::from_file_path(sandbox.root.join("document.html"))
            .unwrap()
            .into(),
        ..RenderOptions::default()
    };
    assert_loaded("style.css", &by_url, "beside a file: document");
    assert_refused(
        "../secret.css",
        &by_url,
        OUTSIDE_FOLDER,
        "above a file: document",
    );
}

#[cfg(unix)]
#[test]
fn system_files_and_links_out_of_the_base_folder_are_refused() {
    const SYSTEM_FILE: &str = "/etc/hosts";
    let sandbox = Sandbox::new("links");
    std::os::unix::fs::symlink(&sandbox.secret, sandbox.root.join("link.css")).unwrap();
    std::os::unix::fs::symlink(&sandbox.directory, sandbox.root.join("up")).unwrap();
    let options = sandbox.options();
    for href in [
        "link.css",
        "up/secret.css",
        SYSTEM_FILE,
        "file:///etc/hosts",
    ] {
        assert_refused(
            href,
            &options,
            OUTSIDE_FOLDER,
            "a path leading out of the base",
        );
    }
    let output = render_html(&format!("<img src='{SYSTEM_FILE}'>"), &options).unwrap();
    let diagnostics: Vec<_> = output.diagnostics.iter().map(ToString::to_string).collect();
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.contains(SYSTEM_FILE)
                && diagnostic.contains(OUTSIDE_FOLDER)),
        "an image outside the base folder is reported, not read: {diagnostics:?}"
    );
    // The folder itself may be named through a link: only where files really are counts.
    let linked_root = sandbox.directory.join("linked-root");
    std::os::unix::fs::symlink(&sandbox.root, &linked_root).unwrap();
    let through_link = RenderOptions {
        base: linked_root.into(),
        ..RenderOptions::default()
    };
    assert_loaded("style.css", &through_link, "a base named through a link");
}

#[test]
fn the_folder_is_the_documents_whichever_resource_names_the_file() {
    let sandbox = Sandbox::new("nested");
    let options = sandbox.options();
    assert_loaded(
        "sub/inside.css",
        &options,
        "a sheet in a subfolder importing ../style.css inside the base",
    );
    let (loaded, diagnostics) = linking("sub/escape.css", &options);
    assert!(
        !loaded,
        "a sheet in a subfolder read outside the base: {diagnostics}"
    );
    assert!(
        diagnostics.contains("../../secret.css") && diagnostics.contains(OUTSIDE_FOLDER),
        "the import out of the base is reported: {diagnostics}"
    );
}

#[test]
fn a_render_without_a_base_reads_no_local_file() {
    /// A sheet that exists relative to the directory the tests run in.
    const RELATIVE: &str = "tests/oracle/fixtures/profile.css";
    assert!(
        Path::new(RELATIVE).is_file(),
        "{RELATIVE} must exist for the test to mean anything"
    );
    let sandbox = Sandbox::new("no-base");
    let options = RenderOptions::default();
    let (_, diagnostics) = linking(RELATIVE, &options);
    assert!(
        diagnostics.contains(RELATIVE) && diagnostics.contains("has no base"),
        "a relative path in the current directory must not be read: {diagnostics}"
    );
    let style = sandbox.root.join("style.css");
    assert_refused(
        &file_url(&style),
        &options,
        NO_ROOT,
        "a file: URL without a base",
    );
    assert_refused(
        style.to_str().unwrap(),
        &options,
        "has no base",
        "an absolute path without a base",
    );
    let rooted = RenderOptions {
        file_root: Some(sandbox.root.clone()),
        ..RenderOptions::default()
    };
    assert_loaded(
        &file_url(&style),
        &rooted,
        "a file: URL inside an explicit root",
    );
    assert_refused(
        &file_url(&sandbox.secret),
        &rooted,
        OUTSIDE_FOLDER,
        "a file: URL outside an explicit root",
    );
}

#[test]
fn a_document_with_a_remote_base_reads_no_local_file() {
    let sandbox = Sandbox::new("remote");
    let style = file_url(&sandbox.root.join("style.css"));
    for file_root in [None, Some(sandbox.root.clone())] {
        for base in ["https://example.invalid/a.html", "http://example.invalid/"] {
            let options = RenderOptions {
                base: Url::parse(base).unwrap().into(),
                file_root: file_root.clone(),
                ..RenderOptions::default()
            };
            assert_refused(
                &style,
                &options,
                NOT_LOCAL,
                &format!("base {base}, root {file_root:?}"),
            );
        }
    }
}

#[test]
fn file_root_replaces_the_base_folder() {
    let sandbox = Sandbox::new("root");
    let wider = RenderOptions {
        file_root: Some(sandbox.directory.clone()),
        ..sandbox.options()
    };
    assert_loaded("../sibling/shared.css", &wider, "a sibling of the base");
    assert_loaded("../secret.css", &wider, "the parent of the base");
    fs::write(sandbox.root.join("sub/plain.css"), SHEET).unwrap();
    let narrower = RenderOptions {
        file_root: Some(sandbox.root.join("sub")),
        ..sandbox.options()
    };
    assert_loaded("sub/plain.css", &narrower, "inside the narrower root");
    assert_refused(
        "style.css",
        &narrower,
        OUTSIDE_FOLDER,
        "the base folder, outside the narrower root",
    );
}

#[test]
fn each_part_reads_only_its_own_folder() {
    let sandbox = Sandbox::new("parts");
    let other = sandbox.directory.join("sibling");
    let own = document("style.css");
    let others = document(&file_url(&other.join("shared.css")));
    let shared = document("shared.css");
    let parts = || {
        [
            HtmlPart::from_html(&own).with_base(sandbox.root.as_path()),
            HtmlPart::from_html(&others).with_base(sandbox.root.as_path()),
            HtmlPart::from_html(&shared).with_base(other.as_path()),
        ]
    };
    let pages = |options: &RenderOptions| -> Vec<(usize, Vec<String>)> {
        render_parts(&parts(), options)
            .expect("a refused file never fails")
            .parts
            .iter()
            .map(|part| {
                (
                    part.page_count,
                    part.diagnostics.iter().map(ToString::to_string).collect(),
                )
            })
            .collect()
    };
    let confined = pages(&RenderOptions::default());
    assert_eq!(
        confined.iter().map(|(pages, _)| *pages).collect::<Vec<_>>(),
        [PAGES_WITH_SHEET, PAGES_WITHOUT_SHEET, PAGES_WITH_SHEET],
        "a part reads its own folder and not another part's: {confined:?}"
    );
    assert!(
        confined[1]
            .1
            .iter()
            .any(|diagnostic| diagnostic.contains(OUTSIDE_FOLDER)),
        "the refused part reports why: {confined:?}"
    );
    let rooted = pages(&RenderOptions {
        file_root: Some(sandbox.directory.clone()),
        ..RenderOptions::default()
    });
    assert_eq!(
        rooted.iter().map(|(pages, _)| *pages).collect::<Vec<_>>(),
        [PAGES_WITH_SHEET; 3],
        "an explicit root covers every part: {rooted:?}"
    );
}

#[test]
fn a_resolver_without_a_root_reads_nothing() {
    let sandbox = Sandbox::new("default");
    let base: ResourceBase = sandbox.root.as_path().into();
    for resolver in [
        FileResolver::default(),
        FileResolver::for_base(&ResourceBase::default()),
    ] {
        assert!(
            matches!(
                resolver.resolve("style.css", &base),
                Err(ResourceError::NoFileRoot(_))
            ),
            "{resolver:?} must not read a file"
        );
    }
    assert_eq!(
        FileResolver::for_base(&base)
            .resolve("style.css", &base)
            .expect("the base's folder is readable")
            .bytes,
        SHEET.as_bytes()
    );
}

#[test]
fn only_regular_files_within_the_byte_limit_are_read() {
    /// Far past the limit, and sparse: it is refused by its size, not by reading it.
    const HUGE: u64 = 1 << 40;
    let sandbox = Sandbox::new("kinds");
    let base: ResourceBase = sandbox.root.as_path().into();
    let resolver = FileResolver::for_base(&base);
    assert!(
        matches!(
            resolver.resolve("sub", &base),
            Err(ResourceError::NotRegularFile(_))
        ),
        "a directory is not a resource"
    );
    #[cfg(unix)]
    {
        let fifo = sandbox.root.join("pipe.css");
        let made = std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap();
        assert!(made.success(), "mkfifo {}", fifo.display());
        assert!(
            matches!(
                resolver.resolve("pipe.css", &base),
                Err(ResourceError::NotRegularFile(_))
            ),
            "a FIFO is refused without waiting for a writer"
        );
    }
    fs::File::create(sandbox.root.join("huge.css"))
        .unwrap()
        .set_len(HUGE)
        .unwrap();
    assert!(
        matches!(
            resolver.resolve("huge.css", &base),
            Err(ResourceError::TooLarge { .. })
        ),
        "a file past the byte limit is refused"
    );
    let exact = FileResolver {
        max_resource_bytes: SHEET.len(),
        ..resolver.clone()
    };
    assert_eq!(
        exact.resolve("style.css", &base).unwrap().bytes.len(),
        SHEET.len()
    );
    let short = FileResolver {
        max_resource_bytes: SHEET.len() - 1,
        ..resolver
    };
    assert!(
        matches!(
            short.resolve("style.css", &base),
            Err(ResourceError::TooLarge { limit, .. }) if limit == SHEET.len() - 1
        ),
        "one byte past the limit is refused"
    );
}
