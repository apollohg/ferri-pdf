// The CommonJS entry: the same functions, with the error classes behind `load()`.
import ferri = require('ferri-pdf');
import workers = require('ferri-pdf/worker');

async function main(): Promise<number> {
  const output = await ferri.render('<p>x</p>', { base: 'templates', jpegQuality: 80 });
  const { ContentLostError } = await ferri.load();
  const lost: boolean = output instanceof ContentLostError;
  const worker = workers.createRenderWorker();
  const parts = await worker.renderParts([{ html: '<p>x</p>', base: '.' }]);
  await worker.terminate();
  return lost ? 0 : parts.pageCount + output.pageCount;
}
void main;
