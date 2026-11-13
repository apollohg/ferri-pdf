"""Local files are read only inside the folder the base names, or `file_root`."""
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import os
from pathlib import Path
import threading

import pytest

import ferri_pdf

# Puts the second paragraph on a second page, so a loaded sheet shows in the page count.
SHEET = "p + p { break-before: page }"
OUTSIDE_FOLDER = "outside the allowed folder"
NO_ROOT = "no local base or file root"
NO_BASE = "has no base"
NOT_LOCAL = "the content that names it is not local"
LOCAL_LOCATION = "places a remote resource at a local file"
UNSUPPORTED_SCHEME = "unsupported resource scheme"


def absolute_path_reason(reason):
    """Why a bare absolute path is refused: a Windows drive letter parses as a URL scheme."""
    return UNSUPPORTED_SCHEME if os.name == "nt" else reason


def document(href):
    return f"<!doctype html><link rel=stylesheet href='{href}'><p>one</p><p>two</p>"


def messages(diagnostics):
    return "\n".join(diagnostic.message for diagnostic in diagnostics)


def linking(href, **options):
    """Whether the sheet `href` names loaded, and the diagnostics of the render."""
    output = ferri_pdf.render(document(href), ferri_pdf.RenderOptions(**options))
    assert output.page_count in (1, 2), (href, output.page_count)
    return output.page_count == 2, messages(output.diagnostics)


def assert_loaded(href, **options):
    loaded, reported = linking(href, **options)
    assert loaded, (href, options, reported)
    assert reported == "", (href, options, reported)


def assert_refused(href, reason, **options):
    loaded, reported = linking(href, **options)
    assert not loaded, f"{href} was read with {options}: {reported}"
    assert reason in reported, (href, options, reported)
    assert str(href) in reported, f"the diagnostic names {href}: {reported}"


@pytest.fixture
def tree(tmp_path: Path):
    """`root` is the document's folder, with `secret.css` beside it and a `sibling` folder."""
    directory = tmp_path.resolve()
    root = directory / "root"
    (root / "sub").mkdir(parents=True)
    (directory / "sibling").mkdir()
    for sheet in (directory / "secret.css", root / "style.css", directory / "sibling" / "shared.css"):
        sheet.write_text(SHEET)
    (root / "sub" / "inside.css").write_text("@import '../style.css';")
    (root / "sub" / "escape.css").write_text("@import '../../secret.css';")
    return directory


@pytest.fixture
def server(tree: Path):
    """Serves a sheet that imports the local secret and a redirect to it; yields its URL."""
    secret = (tree / "secret.css").as_uri()

    class Handler(BaseHTTPRequestHandler):
        def do_GET(self):
            if self.path == "/moved.css":
                self.send_response(302)
                self.send_header("Location", secret)
                self.send_header("Content-Length", "0")
                self.end_headers()
                return
            body = f"@import url('{secret}');".encode()
            self.send_response(200)
            self.send_header("Content-Type", "text/css")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def log_message(self, *arguments):
            pass

    httpd = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    thread = threading.Thread(target=httpd.serve_forever, daemon=True)
    thread.start()
    yield f"http://127.0.0.1:{httpd.server_address[1]}"
    httpd.shutdown()
    httpd.server_close()
    thread.join()


def test_only_files_inside_the_base_folder_are_read(tree: Path):
    root, secret = tree / "root", tree / "secret.css"
    assert_loaded("style.css", base=root)
    assert_loaded((root / "style.css").as_uri(), base=root)
    assert_loaded("style.css", base_url=(root / "index.html").as_uri())
    refused = [(secret, absolute_path_reason(OUTSIDE_FOLDER))]
    refused += [(href, OUTSIDE_FOLDER) for href in (secret.as_uri(), "../secret.css", "sub/../../secret.css")]
    for href, reason in refused:
        assert_refused(href, reason, base=root)
        assert_refused(href, reason, base=root, fetch=lambda url: None)
        assert_refused(href, reason, base=root, allow_http=False)
    assert_refused("../secret.css", OUTSIDE_FOLDER, base_url=(root / "index.html").as_uri())


@pytest.mark.skipif(os.name != "posix", reason="links and /etc/hosts are POSIX")
def test_system_files_and_links_out_of_the_base_folder_are_refused(tree: Path):
    root = tree / "root"
    (root / "link.css").symlink_to(tree / "secret.css")
    (root / "up").symlink_to(tree)
    for href in ("link.css", "up/secret.css", "/etc/hosts", "file:///etc/hosts"):
        assert_refused(href, OUTSIDE_FOLDER, base=root)
    output = ferri_pdf.render("<img src='/etc/hosts'>", ferri_pdf.RenderOptions(base=root))
    reported = messages(output.diagnostics)
    assert "/etc/hosts" in reported and OUTSIDE_FOLDER in reported, reported


def test_the_folder_is_the_documents_whichever_sheet_names_the_file(tree: Path):
    root = tree / "root"
    assert_loaded("sub/inside.css", base=root)
    loaded, reported = linking("sub/escape.css", base=root)
    assert not loaded, reported
    assert "../../secret.css" in reported and OUTSIDE_FOLDER in reported, reported


def test_without_a_base_no_local_file_is_read(tree: Path, monkeypatch: pytest.MonkeyPatch):
    root = tree / "root"
    monkeypatch.chdir(root)
    assert_refused("style.css", NO_BASE)
    assert_refused(root / "style.css", absolute_path_reason(NO_BASE))
    assert_refused((root / "style.css").as_uri(), NO_ROOT)
    assert_refused((root / "style.css").as_uri(), NO_ROOT, fetch=lambda url: None)
    assert_loaded((root / "style.css").as_uri(), file_root=root)
    assert_refused((tree / "secret.css").as_uri(), OUTSIDE_FOLDER, file_root=root)


def test_a_document_with_a_remote_base_reads_no_local_file(tree: Path):
    root = tree / "root"
    style = (root / "style.css").as_uri()
    for file_root in (None, root):
        for base_url in ("https://example.invalid/a.html", "app://templates/a.html"):
            assert_refused(
                style, NOT_LOCAL, base_url=base_url, file_root=file_root, fetch=lambda url: None
            )
        assert_refused(
            style, NOT_LOCAL, base_url="http://example.invalid/", file_root=file_root
        )


def test_remote_sheets_and_redirects_never_reach_local_files(tree: Path, server: str):
    secret = (tree / "secret.css").as_uri()
    for name in ("importing.css", "moved.css"):
        loaded, reported = linking(f"{server}/{name}", base=tree)
        assert not loaded, f"{name} read the secret inside the root: {reported}"
        assert secret in reported and NOT_LOCAL in reported, reported


def test_fetched_content_never_reaches_local_files(tree: Path):
    secret = (tree / "secret.css").as_uri()
    url = "https://cdn.example/sheet.css"
    importing = {url: f"@import url('{secret}');".encode()}
    loaded, reported = linking(url, base=tree, fetch=importing.get)
    assert not loaded, reported
    assert secret in reported and NOT_LOCAL in reported, reported
    placed = {
        url: ferri_pdf.Resource(b"@import 'secret.css';", url=(tree / "sheet.css").as_uri())
    }
    loaded, reported = linking(url, base=tree, fetch=placed.get)
    assert not loaded, "a response that claims a local location read a local file"
    assert LOCAL_LOCATION in reported, reported


def test_file_root_replaces_the_base_folder(tree: Path):
    root = tree / "root"
    assert_loaded("../sibling/shared.css", base=root, file_root=tree)
    assert_loaded("../secret.css", base=root, file_root=tree)
    (root / "sub" / "plain.css").write_text(SHEET)
    assert_loaded("sub/plain.css", base=root, file_root=root / "sub")
    assert_refused("style.css", OUTSIDE_FOLDER, base=root, file_root=root / "sub")


def test_each_part_reads_only_its_own_folder(tree: Path):
    root, other = tree / "root", tree / "sibling"
    parts = [
        ferri_pdf.Part(document("style.css"), base=root),
        ferri_pdf.Part(document((other / "shared.css").as_uri()), base=root),
        ferri_pdf.Part(document("shared.css"), base_url=(other / "index.html").as_uri()),
        document((root / "style.css").as_uri()),
    ]
    for options in ({}, {"fetch": lambda url: None}, {"allow_http": False}):
        confined = ferri_pdf.render_parts(parts, ferri_pdf.RenderOptions(**options))
        pages = [part.page_count for part in confined.parts]
        assert pages == [2, 1, 2, 1], (options, [messages(p.diagnostics) for p in confined.parts])
        assert OUTSIDE_FOLDER in messages(confined.parts[1].diagnostics)
        assert NO_ROOT in messages(confined.parts[3].diagnostics)
    rooted = ferri_pdf.render_parts(parts, ferri_pdf.RenderOptions(file_root=tree))
    assert [part.page_count for part in rooted.parts] == [2, 2, 2, 2]


@pytest.mark.skipif(not hasattr(os, "mkfifo"), reason="needs FIFOs")
def test_only_regular_files_are_read(tree: Path):
    root = tree / "root"
    os.mkfifo(root / "pipe.css")
    for href in ("sub", "pipe.css"):
        assert_refused(href, "not a regular file", base=root)
