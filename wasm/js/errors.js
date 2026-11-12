/** Rendering failed; invalid markup and missing resources never throw it. */
export class RenderError extends Error {
  constructor(message) {
    super(message);
    this.name = 'RenderError';
  }
}

/**
 * A resource limit left content out of the PDF; `diagnostics` lists the losses and `part`
 * is the one-based part of a multipart render, or `null`.
 */
export class ContentLostError extends RenderError {
  constructor(message, diagnostics, part = null) {
    super(message);
    this.name = 'ContentLostError';
    this.diagnostics = diagnostics;
    this.part = part;
  }
}

/** The plain form of a render failure, which crosses a worker boundary. */
export function describeError(error) {
  return {
    name: error?.name,
    message: error instanceof Error ? error.message : String(error),
    diagnostics: error?.diagnostics,
    part: error?.part,
  };
}

/** The error `describeError` described. */
export function reviveError({ name, message, diagnostics, part }) {
  if (name === 'ContentLostError') {
    return new ContentLostError(message, diagnostics, part);
  }
  if (name === 'RenderError') {
    return new RenderError(message);
  }
  const error = new Error(message);
  if (name) {
    error.name = name;
  }
  return error;
}
