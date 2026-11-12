import type { NodePart, NodeRenderOptions, Renderer } from './types.js' with { 'resolution-mode': 'import' };

export type * from './types.js' with { 'resolution-mode': 'import' };

declare const api: Renderer<NodeRenderOptions, NodePart>;
export const render: typeof api.render;
export const renderParts: typeof api.renderParts;

/** Resolves to the ES module, which also exports the error classes. */
export function load(): Promise<typeof import('./node.js', { with: { 'resolution-mode': 'import' } })>;
