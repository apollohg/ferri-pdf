// The calling side of `createRenderWorker`: sends renders to a worker thread and runs the
// caller's fetch function for it.
import { fetchedResource } from './api.js';
import { describeError, reviveError } from './errors.js';

const TERMINATED = 'the render worker was terminated';

/**
 * A render API over `port`, which has `post(message)`, `listen(handler)`,
 * `failed(handler)` for a worker that stops on its own, `close()` and `settings(options)`
 * for what the calling thread must decide before the options leave it.
 */
export function connect(port) {
  const calls = new Map();
  let nextCall = 0;

  function abandon(error) {
    for (const { reject } of calls.values()) {
      reject(error);
    }
    calls.clear();
  }

  async function fetchFor({ id, call, url, budget }) {
    try {
      const resource = await fetchedResource(await calls.get(call).fetch(url), budget);
      port.post({ fetched: { id, resource } });
    } catch (error) {
      port.post({ fetched: { id, error: describeError(error) } });
    }
  }

  function finished({ id, value, error }) {
    const { resolve, reject } = calls.get(id);
    calls.delete(id);
    if (error) {
      reject(reviveError(error));
    } else {
      resolve(value);
    }
  }

  port.listen((message) => (message.fetch ? fetchFor(message.fetch) : finished(message.result)));
  port.failed(abandon);

  const method =
    (name) =>
    (input, { fetch, ...options } = {}) =>
      new Promise((resolve, reject) => {
        const id = nextCall++;
        calls.set(id, { resolve, reject, fetch });
        port.post({
          call: {
            id,
            method: name,
            input,
            options: port.settings(options),
            proxiesFetch: fetch !== undefined,
          },
        });
      });

  return {
    render: method('render'),
    renderParts: method('renderParts'),
    async terminate() {
      await port.close();
      abandon(new Error(TERMINATED));
    },
  };
}
