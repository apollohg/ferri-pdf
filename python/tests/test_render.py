"""Behaviour of the ferri_pdf extension through its public Python API."""
from pathlib import Path
import shutil

import pytest

import ferri_pdf

PDF_SIGNATURE = b"%PDF-"
PAGE_BREAK_CSS = "p { break-after: page }"
TWO_PARAGRAPHS = "<!doctype html><p>one</p><p>two</p>"
# font-size-adjust this large leaves a box that layout cannot form, so content is lost.
OMITTED_BOX = "<!doctype html><p>before</p><p style='font-size-adjust:1e38'>Hello</p><p>after</p>"
UNREACHABLE_HTTP_IMAGE = "<!doctype html><img src='http://127.0.0.1:9/missing.png' alt=x>"


def messages(diagnostics):
    return [diagnostic.message for diagnostic in diagnostics]


def test_render_str_returns_pdf_pages_and_located_headings():
    output = ferri_pdf.render(
        "<!doctype html><h1 id=intro>Intro</h1><p style='break-after:page'>x</p><h2>Next</h2>"
    )
    assert output.pdf.startswith(PDF_SIGNATURE), output.pdf[:16]
    assert output.page_count == 2
    located = [(h.id, h.level, h.text, h.page) for h in output.headings]
    assert located == [("intro", 1, "Intro", 1), (None, 2, "Next", 2)], located
    assert output.diagnostics == (), messages(output.diagnostics)
    assert output.layout is None


def test_render_bytes_detects_the_declared_encoding():
    html = "<!doctype html><meta charset=windows-1252><h1>café</h1>".encode("cp1252")
    output = ferri_pdf.render(html)
    assert [h.text for h in output.headings] == ["café"], output.headings


def test_base_directory_resolves_relative_stylesheets(tmp_path: Path):
    (tmp_path / "style.css").write_text(PAGE_BREAK_CSS)
    html = "<!doctype html><link rel=stylesheet href=style.css><p>one</p><p>two</p>"
    with_base = ferri_pdf.render(html, ferri_pdf.RenderOptions(base=tmp_path))
    assert with_base.page_count == 2, messages(with_base.diagnostics)
    assert with_base.diagnostics == (), messages(with_base.diagnostics)
    elsewhere = ferri_pdf.render(html, ferri_pdf.RenderOptions(base=tmp_path / "missing"))
    assert elsewhere.page_count == 1
    assert any("style.css" in m for m in messages(elsewhere.diagnostics)), messages(
        elsewhere.diagnostics
    )


def test_base_url_resolves_against_a_document_url(tmp_path: Path):
    (tmp_path / "style.css").write_text(PAGE_BREAK_CSS)
    html = "<!doctype html><link rel=stylesheet href=style.css><p>one</p><p>two</p>"
    options = ferri_pdf.RenderOptions(base_url=(tmp_path / "index.html").as_uri())
    assert ferri_pdf.render(html, options).page_count == 2


def test_file_root_refuses_files_outside_it(tmp_path: Path):
    root = tmp_path / "assets"
    root.mkdir()
    (tmp_path / "outside.css").write_text(PAGE_BREAK_CSS)
    html = "<!doctype html><link rel=stylesheet href=../outside.css><p>one</p><p>two</p>"
    unrestricted = ferri_pdf.render(html, ferri_pdf.RenderOptions(base=root))
    assert unrestricted.page_count == 2, messages(unrestricted.diagnostics)
    restricted = ferri_pdf.render(html, ferri_pdf.RenderOptions(base=root, file_root=root))
    assert restricted.page_count == 1
    assert any("outside" in m for m in messages(restricted.diagnostics)), messages(
        restricted.diagnostics
    )


def test_allow_http_false_never_requests_remote_resources():
    output = ferri_pdf.render(UNREACHABLE_HTTP_IMAGE, ferri_pdf.RenderOptions(allow_http=False))
    found = messages(output.diagnostics)
    assert any("unsupported resource scheme: http" in m for m in found), found
    assert not any(d.loses_content for d in output.diagnostics), found


def test_print_preferences_drive_media_queries():
    html = TWO_PARAGRAPHS + f"<style>@media (prefers-color-scheme: dark) {{ {PAGE_BREAK_CSS} }}</style>"
    assert ferri_pdf.render(html).page_count == 1
    dark = ferri_pdf.render(html, ferri_pdf.RenderOptions(color_scheme="dark"))
    assert dark.page_count == 2


def test_stylesheets_apply_after_the_document():
    options = ferri_pdf.RenderOptions(stylesheets=[PAGE_BREAK_CSS])
    assert ferri_pdf.render(TWO_PARAGRAPHS, options).page_count == 2


def test_paper_sets_the_page_size_when_css_sets_none():
    letter = (612.0, 792.0)
    output = ferri_pdf.render(TWO_PARAGRAPHS, ferri_pdf.RenderOptions(paper=letter))
    assert b"/MediaBox[0 0 612 792]" in output.pdf, output.pdf[:2000]


def test_include_layout_returns_the_measured_layout():
    output = ferri_pdf.render(TWO_PARAGRAPHS, ferri_pdf.RenderOptions(include_layout=True))
    assert isinstance(output.layout, dict), type(output.layout)
    assert len(output.layout["pages"]) == output.page_count, output.layout.keys()


def test_lost_content_raises_with_the_losses_by_default():
    with pytest.raises(ferri_pdf.ContentLostError) as raised:
        ferri_pdf.render(OMITTED_BOX)
    error = raised.value
    assert isinstance(error, ferri_pdf.RenderError)
    assert error.part is None
    assert len(error.diagnostics) == 1, messages(error.diagnostics)
    assert error.diagnostics[0].loses_content
    assert "content left out of the PDF" in str(error), str(error)


def test_allow_content_loss_returns_the_pdf_and_reports_the_loss():
    output = ferri_pdf.render(OMITTED_BOX, ferri_pdf.RenderOptions(allow_content_loss=True))
    assert output.pdf.startswith(PDF_SIGNATURE)
    assert any(d.loses_content for d in output.diagnostics), messages(output.diagnostics)


def test_render_parts_offsets_pages_and_headings_across_parts(tmp_path: Path):
    (tmp_path / "style.css").write_text(PAGE_BREAK_CSS)
    output = ferri_pdf.render_parts([
        "<!doctype html><h1>Cover</h1>",
        ferri_pdf.Part(
            "<!doctype html><link rel=stylesheet href=style.css><h1>Body</h1><p>a</p><p>b</p>",
            base=tmp_path,
        ),
        "<!doctype html><h1>Appendix</h1>".encode(),
    ])
    pages = [(part.first_page, part.page_count) for part in output.parts]
    assert pages == [(1, 1), (2, 2), (4, 1)], pages
    assert output.page_count == 4
    assert [h.page for part in output.parts for h in part.headings] == [1, 2, 4]


def test_render_parts_names_the_part_that_lost_content():
    with pytest.raises(ferri_pdf.ContentLostError) as raised:
        ferri_pdf.render_parts(["<!doctype html><p>fine</p>", OMITTED_BOX])
    assert raised.value.part == 2, str(raised.value)
    assert raised.value.diagnostics[0].loses_content


def test_render_parts_requires_a_part():
    with pytest.raises(ferri_pdf.RenderError, match="at least one part"):
        ferri_pdf.render_parts([])


@pytest.mark.parametrize(
    ("arguments", "message"),
    [
        ({"jpeg_quality": 0}, "jpeg_quality must be from 1 to 100"),
        ({"jpeg_quality": 1000}, "jpeg_quality must be from 1 to 100"),
        ({"image_dpi": -1.0}, "image_dpi"),
        ({"paper": (0.0, 792.0)}, "paper"),
        ({"color_scheme": "sepia"}, "color_scheme"),
        ({"contrast": "high"}, "contrast"),
        ({"reduced_motion": "yes"}, "reduced_motion"),
        ({"base": ".", "base_url": "file:///"}, "either base or base_url"),
        ({"base_url": "not a url"}, "base_url"),
    ],
)
def test_invalid_options_raise_value_error(arguments, message):
    with pytest.raises(ValueError, match=message):
        ferri_pdf.RenderOptions(**arguments)


def test_html_must_be_text_or_bytes():
    with pytest.raises(TypeError, match="str or bytes, not int"):
        ferri_pdf.render(42)  # type: ignore[arg-type]
    with pytest.raises(TypeError, match="str or bytes, not list"):
        ferri_pdf.render_parts([["<p>x</p>"]])  # type: ignore[list-item]


FONT_FIXTURES = Path(__file__).resolve().parents[2] / "tests/fixtures/fonts"
INSTALLED_FAMILY = "System Test"
INSTALLED_FAMILY_HTML = f"<!doctype html><p style=\"font-family:'{INSTALLED_FAMILY}',serif\">x</p>"


def first_run_family(output):
    layout = output.layout
    run = layout["pages"][0][0]["runs"][0]
    return layout["fonts"][run["font_id"]]["family"]


def test_font_dirs_add_installed_families(tmp_path: Path):
    shutil.copy(FONT_FIXTURES / "SystemTest-Regular.ttf", tmp_path)
    output = ferri_pdf.render(
        INSTALLED_FAMILY_HTML, ferri_pdf.RenderOptions(font_dirs=[tmp_path], include_layout=True)
    )
    assert output.diagnostics == (), messages(output.diagnostics)
    assert first_run_family(output) == INSTALLED_FAMILY
    without = ferri_pdf.render(INSTALLED_FAMILY_HTML, ferri_pdf.RenderOptions(include_layout=True))
    assert first_run_family(without) == "Liberation Serif"
    found = messages(without.diagnostics)
    assert any(INSTALLED_FAMILY in m for m in found), found


def test_unreadable_font_dirs_warn(tmp_path: Path):
    missing = tmp_path / "absent"
    output = ferri_pdf.render(TWO_PARAGRAPHS, ferri_pdf.RenderOptions(font_dirs=[missing]))
    assert output.pdf.startswith(PDF_SIGNATURE)
    found = messages(output.diagnostics)
    assert any(str(missing) in m for m in found), found
    assert not any(d.loses_content for d in output.diagnostics), found


def test_system_fonts_render():
    output = ferri_pdf.render(TWO_PARAGRAPHS, ferri_pdf.RenderOptions(system_fonts=True))
    assert output.pdf.startswith(PDF_SIGNATURE)
