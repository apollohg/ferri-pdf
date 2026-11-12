// Fixtures shared by the entry suites: a local HTTP server, documents and assertions.
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import http from 'node:http';

export const PDF_SIGNATURE = '%PDF-';
export const PAGE_BREAK_CSS = 'p { break-after: page }';
export const TWO_PARAGRAPHS = '<!doctype html><p>one</p><p>two</p>';
// font-size-adjust this large leaves a box that layout cannot form, so content is lost.
export const OMITTED_BOX =
  "<!doctype html><p>before</p><p style='font-size-adjust:1e38'>Hello</p><p>after</p>";
/** A document with no resources that exercises text, tables, lists, floats and page breaks. */
export const ARTICLE = `<!doctype html><meta charset=utf-8><title>Quarterly report</title>
<style>
  @page { size: A5; margin: 15mm; @bottom-center { content: counter(page) } }
  h1 { color: #1f4e79; border-bottom: 2pt solid currentColor }
  h2 { break-before: page }
  table { border-collapse: collapse; width: 100% }
  td, th { border: 0.5pt solid #888; padding: 2pt 4pt; text-align: right }
  aside { float: right; width: 30%; margin: 0 0 6pt 6pt; padding: 4pt; background: #eef }
  p { text-align: justify; hyphens: auto }
  ul { columns: 2 }
</style>
<h1 id=summary>Summary</h1>
<aside>Figures are <em>provisional</em> until the audit closes.</aside>
${'<p lang=en>Revenue grew across every region while operating expenditure stayed within the approved allocation, and the consolidated forecast remains unchanged for the coming year.</p>'.repeat(6)}
<ul>${Array.from({ length: 12 }, (_, index) => `<li>Initiative ${index + 1}</li>`).join('')}</ul>
<h2 id=results>Results</h2>
<table><tr><th>Region<th>Revenue<th>Change</tr>
${Array.from({ length: 40 }, (_, row) => `<tr><td>Region ${row}<td>${row * 1375}<td>${row % 7}%</tr>`).join('')}
</table>
<p style="font-family: monospace">caf\u00e9 \u2014 na\u00efve \u03a9 \u6f22\u5b57</p>`;

/** A one-pixel PNG. */
export const PNG = Buffer.from(
  'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==',
  'base64',
);
export const REPOSITORY = new URL('../../', import.meta.url);
export const FONT = await readFile(new URL('assets/fonts/DejaVuSansMono.ttf', REPOSITORY));

const NOT_FOUND = 404;

export function messages(diagnostics) {
  return diagnostics.map((diagnostic) => diagnostic.message);
}

export function assertPdf(pdf) {
  assert.ok(pdf instanceof Uint8Array, `pdf is ${Object.prototype.toString.call(pdf)}`);
  assert.equal(Buffer.from(pdf.subarray(0, PDF_SIGNATURE.length)).toString('latin1'), PDF_SIGNATURE);
}

/**
 * Serves `routes`, a map from path to `{ body, type, headers }`, on a loopback port; any other path
 * is a 404. `requests` lists the paths asked for, in order.
 */
export async function serve(routes) {
  const requests = [];
  const server = http.createServer((request, response) => {
    requests.push(request.url);
    const route = routes[request.url];
    if (!route) {
      response.writeHead(NOT_FOUND).end('missing');
      return;
    }
    response.writeHead(200, { 'content-type': route.type, ...route.headers }).end(route.body);
  });
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
  return {
    origin: `http://127.0.0.1:${server.address().port}`,
    requests,
    close: () => new Promise((resolve) => server.close(resolve)),
  };
}

/** A loopback origin nothing listens on, so connecting to it is refused. */
export async function refusedOrigin() {
  const server = await serve({});
  await server.close();
  return server.origin;
}
