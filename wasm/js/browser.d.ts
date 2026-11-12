import type { Part, Renderer, RenderOptions } from './types.js';

export * from './types.js';

declare const api: Renderer<RenderOptions, Part>;
export const render: typeof api.render;
export const renderParts: typeof api.renderParts;

/** The URL relative references resolve against by default: the page's, or a worker's own. */
export function defaultBaseUrl(): string | undefined;
