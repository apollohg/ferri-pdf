// Browser entry: the module is fetched beside this file and resources load through `fetch`.
import init, { RenderJob } from './generated/ferri_pdf.js';
import { budgeted, createApi } from './api.js';

export { ContentLostError, RenderError } from './errors.js';

let bindings;

/** The URL relative references resolve against by default: the page's, or a worker's own. */
export function defaultBaseUrl() {
  return globalThis.document?.baseURI ?? globalThis.location?.href;
}

const api = createApi({
  bindings() {
    bindings ??= init({
      module_or_path: new URL('./generated/ferri_pdf_bg.wasm', import.meta.url),
    }).then(() => ({ RenderJob }));
    return bindings;
  },
  prepare({ fetch = globalThis.fetch, baseUrl = defaultBaseUrl(), ...settings }) {
    return { settings: { ...settings, baseUrl }, fetch: budgeted(fetch) };
  },
  partBase: ({ baseUrl }) => ({ baseUrl }),
});

export const { render, renderParts } = api;
