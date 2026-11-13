// Node entry: the module is read from disk, `file:` resources are read by this wrapper (the
// module has no file system) and every other URL loads through `fetch`.
import { constants } from 'node:fs';
import { lstat, open, readFile, realpath } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

import { initSync, RenderJob } from './generated/ferri_pdf.js';
import { budgeted, createApi } from './api.js';

export { ContentLostError, RenderError } from './errors.js';

const FILE_SCHEME = 'file:';
const WASM_FILE = new URL('./generated/ferri_pdf_bg.wasm', import.meta.url);
// The reasons match the native library's, so a refusal reads the same everywhere.
const NO_FILE_ROOT = 'the render has no local base or file root';
const OUTSIDE_ROOT = 'local file is outside the allowed folder';
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

/** Whether `file`, with links resolved, lies in `root` once its links are resolved too. */
async function contains(root, file) {
  const inside = path.relative(await realpath(root), file);
  return inside !== '' && !inside.startsWith('..') && !path.isAbsolute(inside);
}

/**
 * Reads a local file within `budget`. Only a regular file inside the folder at the URL
 * `root`, judged by real paths, is read, and only through the handle that was verified; a
 * device, a directory, a longer file or a file replaced after the check is refused unread.
 * The renderer names the root: the caller's `fileRoot`, or else the folder of the base of
 * the document that asks, and it asks for no file on behalf of remote content.
 */
async function readLocal(url, root, budget) {
  if (root === undefined) {
    throw new Error(NO_FILE_ROOT);
  }
  const file = await realpath(fileURLToPath(url));
  if (!(await contains(fileURLToPath(root), file))) {
    throw new Error(`${OUTSIDE_ROOT}: ${file}`);
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
  prepare({ fetch = globalThis.fetch, base, baseUrl, fileRoot, ...settings }) {
    const named = baseOf({ base, baseUrl });
    const remote = budgeted(fetch);
    return {
      settings: {
        ...settings,
        // Without a base, relative references do not resolve and no local file is read.
        baseUrl: named.baseUrl,
        baseDirectory: named.directory ?? false,
        fileRoot: fileRoot === undefined ? undefined : directoryUrl(fileRoot),
      },
      // Local files never reach a custom fetch function, which could read around the root.
      fetch: (url, budget, root) =>
        url.startsWith(FILE_SCHEME) ? readLocal(url, root, budget) : remote(url, budget),
    };
  },
  partBase: baseOf,
});

export const { render, renderParts } = api;
