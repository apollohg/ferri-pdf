// Node entry: the module is read from disk, `file:` resources are read by this wrapper (the
// module has no file system) and every other URL loads through `fetch`.
import { readFile, realpath } from 'node:fs/promises';
import path from 'node:path';
import process from 'node:process';
import { fileURLToPath, pathToFileURL } from 'node:url';

import { initSync, RenderJob } from './generated/ferri_pdf.js';
import { createApi } from './api.js';

export { ContentLostError, RenderError } from './errors.js';

const FILE_SCHEME = 'file:';
const WASM_FILE = new URL('./generated/ferri_pdf_bg.wasm', import.meta.url);

let bindings;

/** The `file:` URL of `directory`, which relative references resolve inside. */
function directoryUrl(directory) {
  return pathToFileURL(path.resolve(directory) + path.sep).href;
}

/** The base URL `source` names by `baseUrl` or by its `base` directory, if either. */
function baseUrlOf({ base, baseUrl }) {
  if (base !== undefined && baseUrl !== undefined) {
    throw new TypeError('pass either base or baseUrl, not both');
  }
  return base === undefined ? baseUrl : directoryUrl(base);
}

/** Reads a local file, refusing one outside `fileRoot` once links are resolved. */
async function readLocal(url, fileRoot) {
  const file = await realpath(fileURLToPath(url));
  if (fileRoot !== undefined) {
    const inside = path.relative(await realpath(fileRoot), file);
    if (inside.startsWith('..') || path.isAbsolute(inside)) {
      throw new Error(`resource escapes the allowed root: ${file}`);
    }
  }
  return { data: await readFile(file), url: pathToFileURL(file).href };
}

const api = createApi({
  bindings() {
    bindings ??= readFile(WASM_FILE).then((module) => {
      initSync({ module });
      return { RenderJob };
    });
    return bindings;
  },
  prepare({ fetch = globalThis.fetch, base, baseUrl, fileRoot, ...settings }) {
    return {
      settings: {
        ...settings,
        baseUrl: baseUrlOf({ base, baseUrl }) ?? directoryUrl(process.cwd()),
      },
      fetch: (url) => (url.startsWith(FILE_SCHEME) ? readLocal(url, fileRoot) : fetch(url)),
    };
  },
  partBaseUrl: baseUrlOf,
});

export const { render, renderParts } = api;
