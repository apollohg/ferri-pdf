// The package as Node resolves `require('ferri-pdf')`, whose error classes come from `load()`.
import { createRequire } from 'node:module';

import { defineSuite } from './suite.mjs';

const require = createRequire(import.meta.url);
const ferri = require('ferri-pdf');
const { createRenderWorker } = require('ferri-pdf/worker');

defineSuite(
  'CommonJS entry',
  async () => {
    const { ContentLostError, RenderError } = await ferri.load();
    return { render: ferri.render, renderParts: ferri.renderParts, ContentLostError, RenderError };
  },
  createRenderWorker,
);
