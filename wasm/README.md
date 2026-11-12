# Ferri PDF for JavaScript

An HTML and CSS to PDF renderer for print, written in pure Rust and compiled to WebAssembly. It runs in browsers and in Node 18 or later; no headless browser and no server are involved.

```sh
npm install ferri-pdf
```

## Rendering

In a browser, with a bundler or as a plain ES module:

```js
import { render } from 'ferri-pdf';

const output = await render('<h1>Hello</h1><img src="/logo.png">');
const url = URL.createObjectURL(new Blob([output.pdf], { type: 'application/pdf' }));
```

In Node, as an ES module or with `require`:

```js
import { writeFile } from 'node:fs/promises';
import { render } from 'ferri-pdf';

const output = await render(html, { base: 'templates/invoice' });
await writeFile('invoice.pdf', output.pdf);
```

`render` takes a string, or a `Uint8Array` decoded by the HTML encoding rules. It resolves to the PDF bytes as `pdf`, `pageCount`, the located `headings` and the `diagnostics` for recovered input. Invalid markup, unsupported CSS and missing resources never reject; they are recovered as the specifications say and listed in `diagnostics`.

The WebAssembly module loads on the first render: the browser entry fetches `ferri_pdf_bg.wasm` from beside its script and the Node entry reads it from disk.

## Remote resources

Stylesheets, images and fonts load through `fetch`. The renderer first reports the URLs a document needs, the package fetches them in parallel, and this repeats until nothing is missing, because a fetched stylesheet can name more resources. `maxFetchRounds` and `maxFetchBytes` bound it.

Relative URLs resolve against `baseUrl`, which defaults to the page's URL in a browser and to the working directory in Node. Responses are read as streams and a response that would pass `maxFetchBytes` is abandoned.

In a browser, resources from another origin need CORS headers that allow the page to read them, or a proxy on your own origin. A resource that is blocked, missing or unreachable does not fail the render: the PDF is produced without it and a diagnostic names it.

Pass your own `fetch` to add headers or route requests through a proxy. It is called with each absolute URL and returns a `Response`, bytes, `{ data, contentType, url }` or `null`:

```js
const output = await render(html, {
  baseUrl: 'https://example.com/reports/q3.html',
  fetch: (url) => fetch(`/proxy?url=${encodeURIComponent(url)}`, { headers: { authorization } }),
});
```

## Local files in Node

A document reads local files only when you point it at them: pass `base`, a directory, or a `file:` `baseUrl`. Files are then read from inside that directory and nowhere else, judged after symbolic links and `..` are resolved, so `file:///etc/passwd` or `../../secret` in the HTML or in a stylesheet is refused and reported as a diagnostic. `fileRoot` names a different directory to confine reads to, for example a shared assets folder above the base.

Without a local base no file is read, and a document whose base is an `http(s)` URL never reads one, as in a browser. Local files are read by the package itself and never passed to a custom `fetch`. Only regular files are read, within `maxFetchBytes`.

## Options

| Option | Effect |
|---|---|
| `baseUrl` | Document URL that relative URLs resolve against |
| `base` | Node only: directory that relative URLs resolve against and local files are read from, instead of `baseUrl` |
| `fileRoot` | Node only: the directory local files are read from, instead of the base's |
| `stylesheets` | Author stylesheets applied after the document's own |
| `userStylesheets` | User-origin stylesheets |
| `paper` | Paper size `[width, height]` in points when CSS sets none (default: A4) |
| `colorScheme` | `"light"` or `"dark"` for `prefers-color-scheme` |
| `contrast` | `"no-preference"`, `"less"`, `"more"` or `"custom"` for `prefers-contrast` |
| `reducedMotion` | `"no-preference"` or `"reduce"` for `prefers-reduced-motion` |
| `serif`, `sansSerif`, `monospace` | Bundled families for each generic font |
| `fallback` | Bundled families tried for missing glyphs |
| `imageDpi` | Downsample raster images to this resolution |
| `jpegQuality` | Re-encode opaque photos as JPEG at this quality, 1 to 100 |
| `includeLayout` | Return the measured layout as `output.layout` |
| `allowContentLoss` | Return the PDF even when a resource limit left content out |
| `fetch` | Function that loads every URL other than data URLs (default: the global `fetch`) |
| `maxFetchRounds` | Times missing resources are fetched before the rest stay unavailable (default: 16) |
| `maxFetchBytes` | Bytes of all fetched resources together (default: 256 MiB) |

The module has no file system, so the operating system's fonts are not available. The Liberation and DejaVu families are bundled; load any other font with `@font-face`.

## Off the main thread

A long render blocks the thread it runs on. `createRenderWorker` runs renders in a Web Worker, or in a worker thread in Node, with the same `render` and `renderParts`:

```js
import { createRenderWorker } from 'ferri-pdf/worker';

const worker = createRenderWorker();
const output = await worker.render(html, { baseUrl: location.href });
await worker.terminate();
```

A custom `fetch` still runs on the calling thread. The worker stays alive until `terminate()`.

## Multiple documents

`renderParts` renders independent documents into one PDF, each starting on a new page:

```js
import { renderParts } from 'ferri-pdf';

const output = await renderParts([
  cover,
  { html: body, baseUrl: 'https://example.com/report/body.html' },
  appendix,
]);
```

Each entry of `output.parts` has `firstPage`, `pageCount`, `headings` and `diagnostics`. The parts share the options' base for their relative URLs; a part's own `baseUrl` is the URL of that document, which links from other parts can name, and in Node a part's `base` is its directory.

## Lost content

When a resource limit leaves content out of the PDF, rendering rejects with `ContentLostError`, a subclass of `RenderError`. Its `diagnostics` list the losses and `part` is the one-based part in a multipart render. Pass `allowContentLoss: true` to get the PDF anyway.

With `require`, the functions are the same and the error classes come from the ES module: `const { ContentLostError } = await require('ferri-pdf').load()`, or test `error.name`.

## Building

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.129
npm run build
npm test
```
