// The render loop shared by the browser and Node entries. The WebAssembly renderer resolves
// resources synchronously, so each attempt reports the URLs it lacked; they are fetched in
// parallel and the render repeats until nothing is missing.
import { ContentLostError, RenderError } from './errors.js';

const NO_RESOURCE = 'fetch returned no resource';

function bytesOf(value, name) {
  if (value instanceof Uint8Array) {
    return value;
  }
  if (value instanceof ArrayBuffer) {
    return new Uint8Array(value);
  }
  throw new TypeError(`${name} must be a Uint8Array or ArrayBuffer`);
}

/**
 * Marks a fetch function that takes the byte budget, `{ bytes, reason }`, as its second
 * argument and enforces it itself, as the worker's proxy to the calling thread does.
 */
export const BUDGETED = Symbol('budgeted');

/** Throws the budget's reason if `length` bytes are more than it allows. */
function checkBudget(length, budget) {
  if (length > budget.bytes) {
    throw new Error(budget.reason);
  }
}

/**
 * Reads a response body as a stream, giving up once it is longer than the budget; a declared
 * length over the budget is refused unread.
 */
async function readBody(response, budget) {
  const declared = Number(response.headers?.get('content-length'));
  const reader = response.body?.getReader?.();
  if (declared > budget.bytes) {
    await reader?.cancel();
    throw new Error(budget.reason);
  }
  if (!reader) {
    // A response-like object without a stream has its bytes in memory already.
    const data = new Uint8Array(await response.arrayBuffer());
    checkBudget(data.length, budget);
    return data;
  }
  const chunks = [];
  let length = 0;
  for (;;) {
    const { done, value } = await reader.read();
    if (done) {
      break;
    }
    length += value.length;
    if (length > budget.bytes) {
      await reader.cancel();
      throw new Error(budget.reason);
    }
    chunks.push(value);
  }
  const data = new Uint8Array(length);
  let offset = 0;
  for (const chunk of chunks) {
    data.set(chunk, offset);
    offset += chunk.length;
  }
  return data;
}

/**
 * What a fetch function returned, as `{ data, contentType, url }`: a `Response`, bytes, or
 * that object itself. Throws for no resource, an HTTP error status and data over `budget`.
 */
export async function fetchedResource(result, budget) {
  if (result === null || result === undefined) {
    throw new Error(NO_RESOURCE);
  }
  if (typeof result.arrayBuffer === 'function') {
    if (result.ok === false) {
      await result.body?.cancel?.();
      throw new Error(`HTTP ${result.status}`);
    }
    return {
      data: await readBody(result, budget),
      contentType: result.headers?.get('content-type') ?? undefined,
      url: result.url || undefined,
    };
  }
  const resource =
    result instanceof Uint8Array || result instanceof ArrayBuffer
      ? { data: bytesOf(result, 'a fetched resource') }
      : {
          data: bytesOf(result.data, "a fetched resource's data"),
          contentType: result.contentType ?? undefined,
          url: result.url ?? undefined,
        };
  checkBudget(resource.data.length, budget);
  return resource;
}

/** `fetch` as a function of a URL and the byte budget its result must fit. */
export function budgeted(fetch) {
  return fetch[BUDGETED] ? fetch : (url) => fetch(url);
}

/** Fetches `url` into `job`; a failure of any kind leaves the resource unavailable. */
async function answer(job, url, fetch) {
  try {
    const budget = { bytes: job.remainingBytes(), reason: job.byteLimitReason() };
    const { data, contentType, url: location } = await fetchedResource(
      await fetch(url, budget),
      budget,
    );
    job.supply(url, data, contentType, location);
  } catch (error) {
    job.fail(url, error instanceof Error ? error.message : String(error));
  }
}

/** Whether the WebAssembly module itself failed, by a trap or by exhausting the stack. */
function isTrap(error) {
  return error instanceof WebAssembly.RuntimeError || error instanceof RangeError;
}

function failure({ message, lost, part }) {
  return lost ? new ContentLostError(message, lost, part ?? null) : new RenderError(message);
}

/**
 * `render` and `renderParts` for one environment, which supplies:
 * - `bindings()`: resolves to the initialised WebAssembly exports;
 * - `prepare(options, documents)`: `{ settings, fetch }`, the renderer's settings and the
 *   function that loads a URL within a byte budget;
 * - `partBase(part)`: `{ baseUrl, directory }`, the base a part object names, if any, and
 *   whether it is a folder rather than the part's own URL.
 */
export function createApi(environment) {
  async function run(documents, multipart, options = {}) {
    const { settings, fetch } = environment.prepare(options, documents);
    const { RenderJob } = await environment.bindings();
    let trapped = false;
    const job = new RenderJob(JSON.stringify(settings), multipart);
    try {
      for (const { html, baseUrl, directory = false } of documents) {
        if (typeof html === 'string') {
          job.addText(html, baseUrl, directory);
        } else {
          job.addBytes(bytesOf(html, 'HTML'), baseUrl, directory);
        }
      }
      for (let urls = job.advance(); urls.length > 0; urls = job.advance()) {
        await Promise.all(urls.map((url) => answer(job, url, fetch)));
      }
      const report = JSON.parse(job.report());
      if (report.error) {
        throw failure(report.error);
      }
      return { pdf: job.takePdf(), ...(report.document ?? report.multipart) };
    } catch (error) {
      trapped = isTrap(error);
      throw error;
    } finally {
      // A trap leaves the job borrowed inside the module, which then refuses to free it.
      if (!trapped) {
        job.free();
      }
    }
  }

  function document(part) {
    if (typeof part === 'string' || part instanceof Uint8Array || part instanceof ArrayBuffer) {
      return { html: part };
    }
    if (part === null || typeof part !== 'object') {
      throw new TypeError('a part must be a string, bytes or a { html } object');
    }
    return { html: part.html, ...environment.partBase(part) };
  }

  return {
    render: (html, options) => run([{ html }], false, options),
    renderParts: async (parts, options) => run(Array.from(parts, document), true, options),
  };
}
