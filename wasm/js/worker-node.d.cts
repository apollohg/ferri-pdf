import type { NodePart, NodeRenderOptions, RenderWorker } from './types.js' with { 'resolution-mode': 'import' };

export type * from './types.js' with { 'resolution-mode': 'import' };

/** Starts a worker thread that renders, so a long render does not block the event loop. */
export function createRenderWorker(): RenderWorker<NodeRenderOptions, NodePart>;
