// The browser entry: no local-file options.
import { defaultBaseUrl, render, renderParts, RenderError } from 'ferri-pdf';
import { createRenderWorker } from 'ferri-pdf/worker';

async function main(): Promise<Blob> {
  const output = await render('<p>x</p>', {
    baseUrl: defaultBaseUrl(),
    fetch: (url) => fetch(url, { credentials: 'include' }),
    colorScheme: 'dark',
  });
  await renderParts([{ html: '<p>x</p>', baseUrl: 'https://example.com/' }]);
  // @ts-expect-error the browser has no directories
  await render('<p>x</p>', { base: 'templates' });
  const worker = createRenderWorker();
  await worker.render('<p>x</p>').catch((error: unknown) => error instanceof RenderError);
  await worker.terminate();
  return new Blob([output.pdf as BlobPart], { type: 'application/pdf' });
}
void main;
