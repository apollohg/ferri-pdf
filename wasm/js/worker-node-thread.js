// Runs inside the worker thread `createRenderWorker` starts.
import { parentPort } from 'node:worker_threads';

import { render, renderParts } from './node.js';
import { serve } from './worker-host.js';

serve(
  { render, renderParts },
  {
    post: (message, transfer) => parentPort.postMessage(message, transfer),
    listen: (handler) => parentPort.on('message', handler),
  },
);
