# Ferri PDF

An HTML and CSS to PDF renderer for print, written in pure Rust. No browser and no JavaScript.

It covers the CSS print stack: paged media, page-margin boxes and named pages, fragmentation, floats, flex, grid, tables, multi-column, text shaping and bidi, and SVG and raster images (PNG, JPEG, GIF, WebP, AVIF, HEIC).

## Command line

```sh
cargo install ferri-pdf
ferri-pdf input.html output.pdf
```

Use `-` for stdin or stdout. Options:

| Option | Effect |
|---|---|
| `--image-dpi DPI` | Downsample raster images to this resolution |
| `--jpeg-quality 1..100` | Re-encode opaque photos as JPEG at this quality |
| `--font-serif`, `--font-sans-serif`, `--font-monospace FAMILY` | Choose the family for a generic font |
| `--font-fallback FAMILY` | Family used for missing glyphs |
| `--part INPUT.html` | Append another document to the same PDF (repeatable) |
| `--headings HEADINGS.json` | Write heading page numbers as JSON |
| `--trace` | Write the measured layout next to the PDF |

## Library

```toml
[dependencies]
ferri-pdf = "0.1"
```

```rust
use ferri_pdf::{render_html, RenderOptions};

let options = RenderOptions {
    base: std::path::Path::new("assets").into(),
    ..Default::default()
};
let output = render_html("<h1>Hello</h1>", &options)?;
std::fs::write("hello.pdf", output.pdf)?;
```

Relative URLs resolve against `base`. HTTP(S) fetching is behind the default `http` feature. To control resource loading, pass your own resolver to `render_with_resolver`.

## Development

Requires Rust 1.97 or newer.

```sh
cargo test --workspace
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
```

The PDF checks in `tests/*.py` and the standards oracle in `tests/oracle` also need Python with Pillow and fontTools, Poppler and GNU FriBidi. See [tests/oracle/README.md](tests/oracle/README.md).

## Licence

MIT. Forked dependencies in `vendor/` and the SVG libraries inlined under `src/svg/` keep their original licences, as do the bundled fonts in `assets/fonts/` and the specification data in `spec/`; see [vendor/FORKS.md](vendor/FORKS.md).
