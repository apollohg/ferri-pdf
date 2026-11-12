// Node worker entry: renders run in a worker thread so a long one does not block the event
// loop.
import { Worker } from 'node:worker_threads';

import { connect } from './worker-client.js';

const EXITED = 'the render worker exited';

/** Starts a worker thread that renders; `terminate()` stops it. */
export function createRenderWorker() {
  const worker = new Worker(new URL('./worker-node-thread.js', import.meta.url));
  return connect({
    post: (message) => worker.postMessage(message),
    listen: (handler) => worker.on('message', handler),
    failed(handler) {
      worker.on('error', handler);
      worker.on('exit', () => handler(new Error(EXITED)));
    },
    close: () => worker.terminate(),
    settings: (options) => options,
  });
}
