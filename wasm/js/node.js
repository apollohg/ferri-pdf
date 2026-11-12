// Node entry: the module is read from disk, `file:` resources are read by this wrapper (the
// module has no file system) and every other URL loads through `fetch`.
import { constants } from 'node:fs';
import { lstat, open, readFile, realpath } from 'node:fs/promises';
import path from 'node:path';
import process from 'node:process';
import { fileURLToPath, pathToFileURL } from 'node:url';

import { initSync, RenderJob } from './generated/ferri_pdf.js';
import { budgeted, createApi } from './api.js';

export { ContentLostError, RenderError } from './errors.js';

const FILE_SCHEME = 'file:';
const WASM_FILE = new URL('./generated/ferri_pdf_bg.wasm', import.meta.url);
const NO_LOCAL_BASE = 'local files are read only for a render with a local base';
const NOT_A_FILE = 'not a regular file';
const CHANGED = 'the file changed while it was being opened';
/** Absent on Windows, where the checks on the open handle stand alone. */
const NO_FOLLOW = constants.O_NOFOLLOW ?? 0;

let bindings;

/** The `file:` URL of `directory`, which relative references resolve inside. */
function directoryUrl(directory) {
  return pathToFileURL(path.resolve(directory) + path.sep).href;
}

/**
 * The base `source` names, as `{ baseUrl, directory }`: its `baseUrl`, the URL of a document,
 * or its `base` directory, which like a native directory base is no document's URL.
 */
function baseOf({ base, baseUrl }) {
  if (base !== undefined && baseUrl !== undefined) {
    throw new TypeError('pass either base or baseUrl, not both');
  }
  return base === undefined ? { baseUrl } : { baseUrl: directoryUrl(base), directory: true };
}

/** The directory a `file:` base URL names: itself, or the one holding its document. */
function baseDirectory(baseUrl) {
  const location = fileURLToPath(baseUrl);
  return baseUrl.endsWith('/') ? location : path.dirname(location);
}

/**
 * The directories a render may read local files in: `fileRoot` if given, else those of the
 * local bases the caller named. None when the document's base is remote or the caller named
 * no local base, so a document cannot read files the caller did not point it at.
 */
function readableRoots(baseUrl, fileRoot, documents) {
  const named = [baseUrl, ...documents.map((document) => document.baseUrl)].filter(
    (url) => url !== undefined,
  );
  if (named.some((url) => !url.startsWith(FILE_SCHEME))) {
    return [];
  }
  if (fileRoot !== undefined) {
    return [fileRoot];
  }
  return named.map(baseDirectory);
}

/** Whether `file`, with links resolved, lies in `root` once its links are resolved too. */
async function contains(root, file) {
  const inside = path.relative(await realpath(root), file);
  return inside !== '' && !inside.startsWith('..') && !path.isAbsolute(inside);
}

/**
 * Reads a local file within `budget`. Only a regular file inside one of `roots`, judged by
 * real paths, is read, and only through the handle that was verified; a device, a directory,
 * a longer file or a file replaced after the check is refused unread.
 */
async function readLocal(url, roots, budget) {
  if (roots.length === 0) {
    throw new Error(NO_LOCAL_BASE);
  }
  const file = await realpath(fileURLToPath(url));
  const allowed = await Promise.all(roots.map((root) => contains(root, file)));
  if (!allowed.includes(true)) {
    throw new Error(`resource escapes the allowed root: ${file}`);
  }
  // Opening without blocking returns at once for a FIFO, which is then refused; a link put
  // in the file's place since the check is not followed where the platform can refuse it.
  const handle = await open(file, constants.O_RDONLY | constants.O_NONBLOCK | NO_FOLLOW);
  try {
    // The path was checked and then opened by name, so the open file must still be the one
    // the checked path names: the same real path, device and inode, without following a link.
    const stats = await handle.stat({ bigint: true });
    const named = await lstat(file, { bigint: true });
    if ((await realpath(file)) !== file || stats.dev !== named.dev || stats.ino !== named.ino) {
      throw new Error(`${CHANGED}: ${file}`);
    }
    if (!stats.isFile()) {
      throw new Error(`${NOT_A_FILE}: ${file}`);
    }
    if (stats.size > BigInt(budget.bytes)) {
      throw new Error(budget.reason);
    }
    // One byte more than the budget shows a file that grew past it since its size was read.
    const buffer = Buffer.alloc(Number(stats.size) + 1);
    let length = 0;
    for (;;) {
      const { bytesRead } = await handle.read(buffer, length, buffer.length - length, length);
      length += bytesRead;
      if (bytesRead === 0 || length === buffer.length) {
        break;
      }
    }
    if (length > budget.bytes) {
      throw new Error(budget.reason);
    }
    return { data: buffer.subarray(0, length), url: pathToFileURL(file).href };
  } finally {
    await handle.close();
  }
}

const api = createApi({
  bindings() {
    bindings ??= readFile(WASM_FILE).then((module) => {
      initSync({ module });
      return { RenderJob };
    });
    return bindings;
  },
  prepare({ fetch = globalThis.fetch, base, baseUrl, fileRoot, ...settings }, documents) {
    const named = baseOf({ base, baseUrl });
    const roots = readableRoots(named.baseUrl, fileRoot, documents);
    const remote = budgeted(fetch);
    return {
      settings: {
        ...settings,
        baseUrl: named.baseUrl ?? directoryUrl(process.cwd()),
        baseDirectory: named.directory ?? named.baseUrl === undefined,
      },
      // Local files never reach a custom fetch function, which could read around the roots.
      fetch: (url, budget) =>
        url.startsWith(FILE_SCHEME) ? readLocal(url, roots, budget) : remote(url, budget),
    };
  },
  partBase: baseOf,
});

export const { render, renderParts } = api;
