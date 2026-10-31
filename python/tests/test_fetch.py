"""The `fetch` callback that loads every URL other than local files and data URLs."""
from pathlib import Path

import pytest

import ferri_pdf

PAGE_BREAK_CSS = b"p { break-after: page }"
STYLESHEET_URL = "https://cdn.example/assets/style.css"
LINKED = f"<!doctype html><link rel=stylesheet href={STYLESHEET_URL}><p>one</p><p>two</p>"


class Recorder:
    """A fetch callback that serves fixed responses and records every URL it is asked for."""

    def __init__(self, responses):
        self.responses = responses
        self.requested = []

    def __call__(self, url):
        self.requested.append(url)
        response = self.responses[url]
        if isinstance(response, BaseException):
            raise response
        return response


def messages(output):
    return [diagnostic.message for diagnostic in output.diagnostics]


def render(html, fetch, **options):
    return ferri_pdf.render(html, ferri_pdf.RenderOptions(fetch=fetch, **options))


def test_fetch_supplies_remote_resources_by_absolute_url():
    fetch = Recorder({STYLESHEET_URL: PAGE_BREAK_CSS})
    output = render(LINKED, fetch)
    assert fetch.requested == [STYLESHEET_URL]
    assert output.page_count == 2, messages(output)
    assert output.diagnostics == (), messages(output)


def test_resource_url_is_the_base_for_relative_urls_inside_it():
    final = "https://mirror.example/v2/style.css"
    imported = "https://mirror.example/v2/breaks.css"
    fetch = Recorder({
        STYLESHEET_URL: ferri_pdf.Resource(b"@import 'breaks.css';", url=final),
        imported: PAGE_BREAK_CSS,
    })
    output = render(LINKED, fetch)
    assert fetch.requested == [STYLESHEET_URL, imported]
    assert output.page_count == 2, messages(output)


def test_resource_content_type_is_checked_like_a_response_header():
    for content_type, pages in [("text/css", 2), ("text/plain", 1)]:
        fetch = Recorder({STYLESHEET_URL: ferri_pdf.Resource(PAGE_BREAK_CSS, content_type=content_type)})
        output = render(LINKED, fetch)
        assert output.page_count == pages, (content_type, messages(output))
    assert any("text/plain" in message for message in messages(output)), messages(output)


def test_custom_schemes_reach_fetch_through_base_url():
    fetch = Recorder({"app://templates/style.css": PAGE_BREAK_CSS})
    html = "<!doctype html><link rel=stylesheet href=style.css><p>one</p><p>two</p>"
    output = render(html, fetch, base_url="app://templates/invoice.html")
    assert fetch.requested == ["app://templates/style.css"]
    assert output.page_count == 2, messages(output)


def test_local_files_and_data_urls_never_reach_fetch(tmp_path: Path):
    (tmp_path / "style.css").write_bytes(PAGE_BREAK_CSS)
    fetch = Recorder({})
    html = (
        "<!doctype html><link rel=stylesheet href=style.css>"
        "<img src='data:image/svg+xml,<svg xmlns=\"http://www.w3.org/2000/svg\"/>'><p>one</p><p>two</p>"
    )
    output = render(html, fetch, base=tmp_path)
    assert fetch.requested == []
    assert output.page_count == 2, messages(output)
    assert output.diagnostics == (), messages(output)


def test_file_root_still_applies_with_fetch(tmp_path: Path):
    root = tmp_path / "assets"
    root.mkdir()
    (tmp_path / "outside.css").write_bytes(PAGE_BREAK_CSS)
    html = "<!doctype html><link rel=stylesheet href=../outside.css><p>one</p><p>two</p>"
    output = render(html, Recorder({}), base=root, file_root=root)
    assert output.page_count == 1, messages(output)
    assert any("outside" in message for message in messages(output)), messages(output)


def test_none_leaves_the_resource_unavailable():
    output = render(LINKED, Recorder({STYLESHEET_URL: None}))
    assert output.page_count == 1
    assert any("fetch returned None" in message for message in messages(output)), messages(output)


def test_an_exception_leaves_the_resource_unavailable_and_rendering_continues():
    second = "https://cdn.example/assets/second.css"
    fetch = Recorder({STYLESHEET_URL: ConnectionError("connection refused"), second: PAGE_BREAK_CSS})
    html = LINKED.replace("<p>", f"<link rel=stylesheet href={second}><p>", 1)
    output = render(html, fetch)
    assert fetch.requested == [STYLESHEET_URL, second]
    assert output.page_count == 2, messages(output)
    found = messages(output)
    assert any(STYLESHEET_URL in m and "ConnectionError: connection refused" in m for m in found), found
    assert not any(diagnostic.loses_content for diagnostic in output.diagnostics), found


def test_keyboard_interrupt_stops_fetching_and_is_raised():
    second = "https://cdn.example/assets/second.css"
    fetch = Recorder({STYLESHEET_URL: KeyboardInterrupt(), second: PAGE_BREAK_CSS})
    html = LINKED.replace("<p>", f"<link rel=stylesheet href={second}><p>", 1)
    with pytest.raises(KeyboardInterrupt):
        render(html, fetch)
    assert fetch.requested == [STYLESHEET_URL], "no fetch may follow an interrupt"


def test_a_result_of_the_wrong_type_is_raised_as_type_error():
    with pytest.raises(TypeError, match="fetch must return bytes, Resource or None, not str"):
        render(LINKED, Recorder({STYLESHEET_URL: "p { break-after: page }"}))


def test_fetch_is_used_by_render_parts():
    fetch = Recorder({STYLESHEET_URL: PAGE_BREAK_CSS})
    output = ferri_pdf.render_parts(
        ["<!doctype html><h1>Cover</h1>", LINKED], ferri_pdf.RenderOptions(fetch=fetch)
    )
    assert fetch.requested == [STYLESHEET_URL]
    assert [part.page_count for part in output.parts] == [1, 2]


def test_fetch_and_resource_arguments_are_validated():
    with pytest.raises(TypeError, match="fetch must be callable, not str"):
        ferri_pdf.RenderOptions(fetch="https://cdn.example")  # type: ignore[arg-type]
    with pytest.raises(ValueError, match="url"):
        ferri_pdf.Resource(b"", url="relative/path.css")
    with pytest.raises(TypeError):
        ferri_pdf.Resource("text")  # type: ignore[arg-type]
