// The worker side of `createRenderWorker`: runs the renders its client asks for. A custom
// fetch function cannot cross the thread boundary, so its calls are sent back to the client.
import { describeError, reviveError } from './errors.js';

const METHODS = ['render', 'renderParts'];

/**
 * Serves `api` over `port`, which has `post(message, transfer)` and `listen(handler)`.
 */
export function serve(api, port) {
  const fetches = new Map();
  let nextFetch = 0;

  const clientFetch = (call) => (url) =>
    new Promise((resolve, reject) => {
      const id = nextFetch++;
      fetches.set(id, { resolve, reject });
      port.post({ fetch: { id, call, url } });
    });

  function fetched({ id, resource, error }) {
    const { resolve, reject } = fetches.get(id);
    fetches.delete(id);
    if (error) {
      reject(reviveError(error));
    } else {
      resolve(resource);
    }
  }

  async function call({ id, method, input, options, proxiesFetch }) {
    try {
      if (!METHODS.includes(method)) {
        throw new TypeError(`unknown render method ${method}`);
      }
      const value = await api[method](
        input,
        proxiesFetch ? { ...options, fetch: clientFetch(id) } : options,
      );
      port.post({ result: { id, value } }, [value.pdf.buffer]);
    } catch (error) {
      port.post({ result: { id, error: describeError(error) } });
    }
  }

  port.listen((message) => (message.fetched ? fetched(message.fetched) : call(message.call)));
}
