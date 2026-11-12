// Smoke test of the browser entry in headless Chrome: set FERRI_PDF_CHROME to a Chrome or
// chrome-headless-shell binary. The page below runs the renders and posts what happened.
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { readFile } from 'node:fs/promises';
import http from 'node:http';
import path from 'node:path';
import { test } from 'node:test';

import { OMITTED_BOX, PAGE_BREAK_CSS, PDF_SIGNATURE, PNG, serve } from './support.mjs';

const CHROME = process.env.FERRI_PDF_CHROME;
const PACKAGE_FILES = new URL('../js/', import.meta.url);
const PACKAGE_ROUTE = '/package/';
const CONTENT_TYPES = { '.js': 'text/javascript', '.wasm': 'application/wasm' };
const RESULT_ROUTE = '/result';
const PAGE_TIMEOUT_MS = 120_000;
const NESTING = 3000;

/** The page under test; `foreign` is another origin that serves images. */
const page = (foreign) => `<!doctype html><title>ferri-pdf browser smoke test</title>
<script type=module>
  import { ContentLostError, render, renderParts } from '${PACKAGE_ROUTE}browser.js';
  import { createRenderWorker } from '${PACKAGE_ROUTE}worker-browser.js';

  const summary = ({ pdf, pageCount, diagnostics }) => ({
    signature: new TextDecoder().decode(pdf.subarray(0, ${PDF_SIGNATURE.length})),
    pageCount,
    diagnostics: diagnostics.map((diagnostic) => diagnostic.message),
  });
  const linked = '<!doctype html><link rel=stylesheet href=assets/style.css><p>one <img src=assets/photo.png></p><p>two</p>';
  const nested = '<!doctype html><style>div{display:flex}</style>' + '<div>'.repeat(${NESTING}) + 'Deepest' + '</div>'.repeat(${NESTING});
  const result = {};
  try {
    result.sameOrigin = summary(await render(linked));
    result.crossOriginWithCors = summary(await render('<!doctype html><img src="${foreign}/cors.png">'));
    result.crossOriginBlocked = summary(await render('<!doctype html><img src="${foreign}/blocked.png" alt=blocked>'));
    result.partPages = (await renderParts([linked, linked])).parts.map((part) => part.pageCount);
    result.nested = summary(await render(nested, { allowContentLoss: true }));
    result.lost = await render(${JSON.stringify(OMITTED_BOX)}).then(
      () => 'rendered',
      (error) => (error instanceof ContentLostError ? error.diagnostics.length : String(error)),
    );
    const worker = createRenderWorker();
    result.worker = summary(await worker.render(linked));
    result.workerNested = summary(await worker.render(nested, { allowContentLoss: true }));
    const fetched = [];
    result.workerFetch = summary(
      await worker.render(linked, {
        fetch: (url) => {
          fetched.push(new URL(url).pathname);
          return fetch(url);
        },
      }),
    );
    result.workerFetched = fetched.sort();
    await worker.terminate();
  } catch (error) {
    result.error = String(error?.stack ?? error);
  }
  await fetch('${RESULT_ROUTE}', { method: 'POST', body: JSON.stringify(result) });
</script>`;

/**
 * Serves the package's files, the page and its same-origin assets; `result` resolves to
 * what the page posts.
 */
async function servePage(foreign) {
  const routes = {
    '/page.html': { type: 'text/html', body: page(foreign) },
    '/assets/style.css': { type: 'text/css', body: PAGE_BREAK_CSS },
    '/assets/photo.png': { type: 'image/png', body: PNG },
  };
  let posted;
  const result = new Promise((resolve) => {
    posted = resolve;
  });
  const server = http.createServer(async (request, response) => {
    const route = routes[request.url];
    if (request.url === RESULT_ROUTE) {
      const chunks = [];
      for await (const chunk of request) {
        chunks.push(chunk);
      }
      response.writeHead(204).end();
      posted(JSON.parse(Buffer.concat(chunks).toString()));
    } else if (route) {
      response.writeHead(200, { 'content-type': route.type }).end(route.body);
    } else if (request.url.startsWith(PACKAGE_ROUTE)) {
      const file = new URL(request.url.slice(PACKAGE_ROUTE.length), PACKAGE_FILES);
      response.writeHead(200, { 'content-type': CONTENT_TYPES[path.extname(file.pathname)] });
      response.end(await readFile(file));
    } else {
      response.writeHead(404).end();
    }
  });
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
  return {
    origin: `http://127.0.0.1:${server.address().port}`,
    result,
    close: () => new Promise((resolve) => server.close(resolve)),
  };
}

test('the browser entry renders on the page and in a Web Worker', { skip: !CHROME && 'set FERRI_PDF_CHROME to a Chrome binary' }, async () => {
  const foreign = await serve({
    '/cors.png': { type: 'image/png', body: PNG, headers: { 'access-control-allow-origin': '*' } },
    '/blocked.png': { type: 'image/png', body: PNG },
  });
  const site = await servePage(foreign.origin);
  // A debugging port keeps headless Chrome running after the page loads.
  const chrome = spawn(CHROME, ['--headless', '--disable-gpu', '--remote-debugging-port=0', `${site.origin}/page.html`], {
    stdio: 'ignore',
  });
  let timer;
  try {
    const result = await Promise.race([
      site.result,
      new Promise((_, reject) => {
        timer = setTimeout(() => reject(new Error('the page posted no result')), PAGE_TIMEOUT_MS);
      }),
    ]);
    assert.equal(result.error, undefined, result.error);

    const rendered = { signature: PDF_SIGNATURE, pageCount: 2, diagnostics: [] };
    assert.deepEqual(result.sameOrigin, rendered, 'relative URLs resolve against the page');
    assert.deepEqual(result.worker, rendered, 'a worker resolves relative URLs against the page too');
    assert.deepEqual(result.workerFetch, rendered);
    assert.deepEqual(result.workerFetched, ['/assets/photo.png', '/assets/style.css']);
    assert.deepEqual(result.crossOriginWithCors, { ...rendered, pageCount: 1 });
    assert.equal(result.crossOriginBlocked.signature, PDF_SIGNATURE);
    assert.equal(result.crossOriginBlocked.diagnostics.length, 1, 'a response without CORS headers is unavailable');
    assert.match(result.crossOriginBlocked.diagnostics[0], /blocked\.png/);
    assert.deepEqual(result.partPages, [2, 2]);
    assert.equal(result.lost, 1);
    for (const nested of [result.nested, result.workerNested]) {
      assert.equal(nested.signature, PDF_SIGNATURE);
      assert.match(nested.diagnostics.join('\n'), /nesting exceeds/);
    }
  } finally {
    clearTimeout(timer);
    chrome.kill();
    await site.close();
    await foreign.close();
  }
});
