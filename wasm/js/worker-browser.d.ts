import type { Part, RenderOptions, RenderWorker } from './types.js';

export type * from './types.js';

/** Starts a Web Worker that renders, so a long render does not freeze the page. */
export function createRenderWorker(): RenderWorker<RenderOptions, Part>;
