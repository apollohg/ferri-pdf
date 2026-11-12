// CommonJS worker entry; see node.cjs for why the ES module loads on first use.
'use strict';

function createRenderWorker() {
  const worker = import('./worker-node.js').then((module) => module.createRenderWorker());
  return {
    render: async (html, options) => (await worker).render(html, options),
    renderParts: async (parts, options) => (await worker).renderParts(parts, options),
    terminate: async () => (await worker).terminate(),
  };
}

module.exports = { createRenderWorker };
