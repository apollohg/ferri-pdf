// Runs inside the Web Worker `createRenderWorker` starts.
import { render, renderParts } from './browser.js';
import { serve } from './worker-host.js';

serve(
  { render, renderParts },
  {
    post: (message, transfer) => self.postMessage(message, transfer),
    listen: (handler) => self.addEventListener('message', (event) => handler(event.data)),
  },
);
