import type { NodePart, NodeRenderOptions, Renderer } from './types.js';

export * from './types.js';

declare const api: Renderer<NodeRenderOptions, NodePart>;
export const render: typeof api.render;
export const renderParts: typeof api.renderParts;
