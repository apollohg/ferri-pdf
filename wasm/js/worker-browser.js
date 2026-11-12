// Browser worker entry: renders run in a Web Worker so a long one does not freeze the page.
import { defaultBaseUrl } from './browser.js';
import { connect } from './worker-client.js';

/** Starts a Web Worker that renders; `terminate()` stops it. */
export function createRenderWorker() {
  const worker = new Worker(new URL('./worker-browser-thread.js', import.meta.url), {
    type: 'module',
  });
  return connect({
    post: (message) => worker.postMessage(message),
    listen: (handler) => worker.addEventListener('message', (event) => handler(event.data)),
    failed: (handler) =>
      worker.addEventListener('error', (event) =>
        handler(new Error(event.message || 'the render worker failed to start')),
      ),
    close: () => worker.terminate(),
    // Relative URLs resolve against the page, not the worker's script.
    settings: (options) => ({ ...options, baseUrl: options.baseUrl ?? defaultBaseUrl() }),
  });
}
