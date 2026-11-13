# Ferri PDF for Python

An HTML and CSS to PDF renderer for print, written in pure Rust. No browser and no JavaScript.

```sh
pip install ferri-pdf
```

Wheels are prebuilt for CPython 3.10 and later on Linux, macOS and Windows.

## Rendering

```python
import ferri_pdf

output = ferri_pdf.render("<h1>Hello</h1>")
with open("hello.pdf", "wb") as file:
    file.write(output.pdf)
```

`render` takes a `str`, or `bytes` decoded by the HTML encoding rules. It returns the PDF bytes, `page_count`, the located `headings` and the `diagnostics` for recovered input. Invalid markup, unsupported CSS and missing resources never raise; they are recovered as the specifications say and listed in `diagnostics`.

The interpreter lock is released while rendering, so threads render in parallel.

## Options

```python
options = ferri_pdf.RenderOptions(
    base="templates/invoice",
    stylesheets=["@page { size: A4; margin: 2cm }"],
    serif=["Georgia"],
    jpeg_quality=85,
    image_dpi=150,
    allow_http=False,
)
output = ferri_pdf.render(html, options)
```

| Option | Effect |
|---|---|
| `base` | Directory that relative URLs resolve against and local files are read inside (default: none, so no local file is read) |
| `base_url` | Document URL that relative URLs resolve against, instead of `base` |
| `stylesheets` | Author stylesheets applied after the document's own |
| `user_stylesheets` | User-origin stylesheets |
| `paper` | Paper size `(width, height)` in points when CSS sets none (default: A4) |
| `color_scheme` | `"light"` or `"dark"` for `prefers-color-scheme` |
| `contrast` | `"no-preference"`, `"less"`, `"more"` or `"custom"` for `prefers-contrast` |
| `reduced_motion` | `"no-preference"` or `"reduce"` for `prefers-reduced-motion` |
| `serif`, `sans_serif`, `monospace` | Installed families for each generic font |
| `fallback` | Installed families tried for missing glyphs |
| `system_fonts` | Let documents and the options above use the operating system's installed fonts (default: `False`) |
| `font_dirs` | Folders whose fonts are installed families too |
| `image_dpi` | Downsample raster images to this resolution |
| `jpeg_quality` | Re-encode opaque photos as JPEG at this quality, 1 to 100 |
| `include_layout` | Return the measured layout as `output.layout` |
| `allow_content_loss` | Return the PDF even when a resource limit left content out |
| `allow_http` | Fetch HTTP(S) resources with the built-in client (default: `True`) |
| `file_root` | Directory local files are read inside, instead of the base's |
| `fetch` | Callable that loads every URL other than local files and data URLs |

A document reads local files only inside its base folder: `base`, or the folder holding the `file:` document that `base_url` names. Without a base no local file is read, and a document or stylesheet with a remote base such as `https:` never reads one. `file_root` names another folder to read inside instead, for example a shared assets folder above the base:

```python
ferri_pdf.RenderOptions(base="templates/invoice", file_root="templates")
```

Files outside the folder, judged after symbolic links and `..` are resolved, are refused and listed in `diagnostics`. Rendering untrusted HTML? Also set `allow_http=False` so a document cannot reach your network, or route requests through `fetch` and decide there.

## Fetching resources yourself

`fetch` receives each absolute URL that is not a `file:` or `data:` URL: HTTP(S), and any scheme your `base_url` uses, such as `app://`. It replaces the built-in HTTP client. Return the body as `bytes`, a `ferri_pdf.Resource` to add a content type or the final URL after redirects, or `None` when the resource is unavailable.

```python
import httpx
import ferri_pdf

client = httpx.Client(headers={"Authorization": f"Bearer {token}"}, timeout=10)

def fetch(url: str) -> ferri_pdf.Resource | None:
    if not url.startswith("https://assets.example.com/"):
        return None
    response = client.get(url, follow_redirects=True)
    response.raise_for_status()
    return ferri_pdf.Resource(
        response.content,
        content_type=response.headers.get("content-type"),
        url=str(response.url),
    )

output = ferri_pdf.render(html, ferri_pdf.RenderOptions(fetch=fetch))
```

Relative URLs inside a fetched resource resolve against its `url`. An `Exception` raised by `fetch`, like `None`, leaves that resource unavailable and is reported in `diagnostics`; rendering carries on. `KeyboardInterrupt`, `SystemExit` and a return value of the wrong type stop further fetches and are raised from `render`. Local files still load from disk, inside the base folder or `file_root`, and never for a resource `fetch` returned.

## Several documents in one PDF

```python
output = ferri_pdf.render_parts([
    ferri_pdf.Part(cover_html, base="cover"),
    ferri_pdf.Part(body_html, base="body"),
])
for part in output.parts:
    print(part.first_page, part.page_count)
```

Each part starts on a new page. Parts can also be plain `str` or `bytes`, which use the options' base. Each part reads local files only inside its own base folder, unless `file_root` names one folder for all.

## Errors

`ferri_pdf.RenderError` is raised when rendering fails. Its subclass `ferri_pdf.ContentLostError` is raised when a resource limit left content out of the PDF; its `diagnostics` list the losses and `part` names the one-based part of a multipart render. Pass `allow_content_loss=True` to get the PDF anyway. Invalid option values raise `ValueError`.

## Licence

MIT. The bundled fonts, hyphenation patterns and specification data keep their own licences; see the [repository](https://github.com/apollohg/ferri-pdf).
