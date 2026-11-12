// The Node ES module entry: local-file options exist and outputs are typed.
import { ContentLostError, render, renderParts, type RenderOutput } from 'ferri-pdf';
import { createRenderWorker } from 'ferri-pdf/worker';

const output: RenderOutput = await render('<p>x</p>', {
  base: 'templates',
  fileRoot: 'templates',
  paper: [612, 792],
  fetch: async (url) => new Uint8Array(url.length),
});
const pdf: Uint8Array = output.pdf;
const top: number | undefined = output.headings[0]?.top;
const parts = await renderParts(['<p>a</p>', { html: pdf, base: 'templates' }]);
const firstPage: number | undefined = parts.parts[0]?.firstPage;
try {
  await render(new ArrayBuffer(0), { allowContentLoss: false });
} catch (error) {
  if (error instanceof ContentLostError) {
    const part: number | null = error.part;
    const lost: boolean | undefined = error.diagnostics[0]?.losesContent;
    void [part, lost];
  }
}
const worker = createRenderWorker();
const pages: number = (await worker.render('<p>x</p>', { base: '.' })).pageCount;
await worker.terminate();
// @ts-expect-error unknown options are rejected
await render('<p>x</p>', { systemFonts: true });
void [top, firstPage, pages];
