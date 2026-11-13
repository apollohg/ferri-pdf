/** HTML as text, or as bytes decoded by the HTML encoding rules. */
export type Html = string | Uint8Array | ArrayBuffer;

/** A recovered input problem, or with `losesContent`, content a resource limit left out. */
export interface Diagnostic {
  message: string;
  losesContent: boolean;
}

/** A located HTML heading; `page` is one-based and `top` is in points from the page top. */
export interface Heading {
  id: string | null;
  level: number;
  text: string;
  page: number;
  top: number;
}

/** A completed render. */
export interface RenderOutput {
  pdf: Uint8Array;
  pageCount: number;
  headings: Heading[];
  diagnostics: Diagnostic[];
  /** The measured layout, with `includeLayout`. */
  layout?: unknown;
}

/** One part of a multipart render; heading pages count from the start of the combined PDF. */
export interface PartOutput {
  /** One-based first page in the combined PDF. */
  firstPage: number;
  pageCount: number;
  headings: Heading[];
  diagnostics: Diagnostic[];
  layout?: unknown;
}

/** A completed multipart render: the combined PDF and each part's results in input order. */
export interface MultipartOutput {
  pdf: Uint8Array;
  pageCount: number;
  parts: PartOutput[];
}

/** A resource a fetch function loaded; `url` is where redirects ended. */
export interface FetchedResource {
  data: Uint8Array | ArrayBuffer;
  contentType?: string;
  url?: string;
}

export type FetchResult = Response | Uint8Array | ArrayBuffer | FetchedResource | null | undefined;

/**
 * Loads one absolute URL. A rejection, no resource or an HTTP error status leaves the
 * resource unavailable, which the render reports as a diagnostic.
 */
export type FetchFunction = (url: string) => FetchResult | Promise<FetchResult>;

export interface RenderOptions {
  /** Document URL that relative URLs resolve against. */
  baseUrl?: string;
  /** Author stylesheets applied after the document's own. */
  stylesheets?: string[];
  /** User-origin stylesheets. */
  userStylesheets?: string[];
  /** Paper size `[width, height]` in points when CSS sets none (default: A4). */
  paper?: [number, number];
  colorScheme?: 'light' | 'dark';
  contrast?: 'no-preference' | 'less' | 'more' | 'custom';
  reducedMotion?: 'no-preference' | 'reduce';
  /** Bundled families for each generic font. */
  serif?: string[];
  sansSerif?: string[];
  monospace?: string[];
  /** Bundled families tried for missing glyphs. */
  fallback?: string[];
  /** Downsample raster images to this resolution. */
  imageDpi?: number;
  /** Re-encode opaque photos as JPEG at this quality, 1 to 100. */
  jpegQuality?: number;
  /** Return the measured layout as `layout`. */
  includeLayout?: boolean;
  /** Return the PDF even when a resource limit left content out. */
  allowContentLoss?: boolean;
  /** Loads every URL other than data URLs (default: the global `fetch`). */
  fetch?: FetchFunction;
  /** Times resources found missing are fetched before the rest stay unavailable (default: 16). */
  maxFetchRounds?: number;
  /** Bytes of all fetched resources together (default: 256 MiB). */
  maxFetchBytes?: number;
}

/** One document of a multipart render, with its own base URL. */
export interface Part {
  html: Html;
  baseUrl?: string;
}

/** Rendering failed; invalid markup and missing resources never throw it. */
export class RenderError extends Error {}

/** A resource limit left content out of the PDF. */
export class ContentLostError extends RenderError {
  /** The losses. */
  diagnostics: Diagnostic[];
  /** The one-based part of a multipart render, or `null`. */
  part: number | null;
}

/** The render functions of one entry, for its options and parts. */
export interface Renderer<Options, PartInput> {
  /** Render one HTML document to PDF. */
  render(html: Html, options?: Options): Promise<RenderOutput>;
  /** Render independent documents into one PDF; each part starts a page. */
  renderParts(parts: Iterable<Html | PartInput>, options?: Options): Promise<MultipartOutput>;
}

/** A worker thread that renders. */
export interface RenderWorker<Options, PartInput> extends Renderer<Options, PartInput> {
  /** Stops the worker; renders still running reject. */
  terminate(): Promise<void>;
}

/** Options of the Node entry, which also reads local files. */
export interface NodeRenderOptions extends RenderOptions {
  /**
   * Directory that relative URLs resolve against. Local files are read only inside the
   * base's directory, never for content with a remote base, and not at all without a base.
   */
  base?: string;
  /** The directory local files are read inside, instead of the base's. */
  fileRoot?: string;
}

/** A part of the Node entry, whose base may be a directory. */
export interface NodePart extends Part {
  base?: string;
}
