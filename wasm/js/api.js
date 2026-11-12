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

/** Reads a response body, giving up once it is longer than `budget` bytes. */
async function readBody(response, budget, limitReason) {
  const reader = response.body?.getReader?.();
  if (!reader) {
    const data = new Uint8Array(await response.arrayBuffer());
    if (data.length > budget) {
      throw new Error(limitReason);
    }
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
    if (length > budget) {
      await reader.cancel();
      throw new Error(limitReason);
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
 * that object itself. Throws for no resource and for an HTTP error status.
 */
export async function fetchedResource(result, budget = Infinity, limitReason = '') {
  if (result === null || result === undefined) {
    throw new Error(NO_RESOURCE);
  }
  if (typeof result.arrayBuffer === 'function') {
    if (result.ok === false) {
      throw new Error(`HTTP ${result.status}`);
    }
    return {
      data: await readBody(result, budget, limitReason),
      contentType: result.headers?.get('content-type') ?? undefined,
      url: result.url || undefined,
    };
  }
  if (result instanceof Uint8Array || result instanceof ArrayBuffer) {
    return { data: bytesOf(result, 'a fetched resource') };
  }
  return {
    data: bytesOf(result.data, "a fetched resource's data"),
    contentType: result.contentType ?? undefined,
    url: result.url ?? undefined,
  };
}

/** Fetches `url` into `job`; a failure of any kind leaves the resource unavailable. */
async function answer(job, url, fetch) {
  try {
    const { data, contentType, url: location } = await fetchedResource(
      await fetch(url),
      job.remainingBytes(),
      job.byteLimitReason(),
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
 * - `prepare(options)`: `{ settings, fetch }`, the renderer's settings and the function
 *   that loads a URL;
 * - `partBaseUrl(part)`: the base URL a part object names, if any.
 */
export function createApi(environment) {
  async function run(documents, multipart, options = {}) {
    const { settings, fetch } = environment.prepare(options);
    const { RenderJob } = await environment.bindings();
    let trapped = false;
    const job = new RenderJob(JSON.stringify(settings), multipart);
    try {
      for (const { html, baseUrl } of documents) {
        if (typeof html === 'string') {
          job.addText(html, baseUrl);
        } else {
          job.addBytes(bytesOf(html, 'HTML'), baseUrl);
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
    return { html: part.html, baseUrl: environment.partBaseUrl(part) };
  }

  return {
    render: (html, options) => run([{ html }], false, options),
    renderParts: async (parts, options) => run(Array.from(parts, document), true, options),
  };
}
