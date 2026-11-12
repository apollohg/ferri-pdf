// The behaviour every Node entry must have; `node-esm.test.mjs` and `node-cjs.test.mjs` run
// it against the package's `import` and `require` conditions.
import assert from 'node:assert/strict';
import { execFile } from 'node:child_process';
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { after, before, describe, test } from 'node:test';
import { pathToFileURL } from 'node:url';
import { promisify } from 'node:util';

import {
  ARTICLE,
  assertPdf,
  FONT,
  messages,
  OMITTED_BOX,
  PAGE_BREAK_CSS,
  PNG,
  refusedOrigin,
  serve,
  TWO_PARAGRAPHS,
} from './support.mjs';

/** The native renderer the byte-for-byte comparison runs, built by `cargo build`. */
const NATIVE_CLI = process.env.FERRI_PDF_CLI;
const LINKED = '<!doctype html><link rel=stylesheet href=style.css><p>one</p><p>two</p>';
/** Formatting nesting the renderer flattens; each construct recurses differently. */
const NESTING_PAST_LIMIT = 3000;
const NESTED_CONSTRUCTS = [
  ['blocks', '', '<div>', '</div>'],
  ['inline-blocks', 'span{display:inline-block}', '<span>', '</span>'],
  ['tables in cells', '', '<table><tr><td>', '</td></tr></table>'],
  ['flex containers', 'div{display:flex}', '<div>', '</div>'],
  ['grid containers', 'div{display:grid}', '<div>', '</div>'],
  ['floats', 'div{float:left}', '<div>', '</div>'],
  ['filtered stacking contexts', 'div{filter:blur(1px);position:relative;z-index:1}', '<div>', '</div>'],
  [
    'mixed layouts',
    'x-i{display:inline-block}x-l{display:block;float:left}x-p{display:block;position:absolute}' +
      'x-f{display:flex}x-g{display:grid}',
    '<x-i><x-l><x-p><x-f><x-g><div>',
    '</div></x-g></x-f></x-p></x-l></x-i>',
  ],
];

/**
 * `load()` resolves to `{ render, renderParts, RenderError, ContentLostError }` of the entry
 * under test and `createRenderWorker` starts its worker.
 */
export function defineSuite(label, load, createRenderWorker) {
  describe(label, () => {
    let api;
    let directory;

    before(async () => {
      api = await load();
      directory = await mkdtemp(path.join(tmpdir(), 'ferri-pdf-'));
    });
    after(() => rm(directory, { recursive: true }));

    test('renders plain HTML with its headings and no diagnostics', async () => {
      const output = await api.render('<!doctype html><h1 id=top>Title</h1><p>Body</p>');
      assertPdf(output.pdf);
      assert.equal(output.pageCount, 1);
      assert.deepEqual(messages(output.diagnostics), []);
      assert.equal(output.headings.length, 1);
      const [{ id, level, text, page, top }] = output.headings;
      assert.deepEqual({ id, level, text, page }, { id: 'top', level: 1, text: 'Title', page: 1 });
      assert.ok(top > 0, `heading top is ${top}`);
      assert.equal(output.layout, undefined);
    });

    test('decodes HTML bytes by the HTML encoding rules', async () => {
      const html = Buffer.from('<!doctype html><meta charset=windows-1252><h1>caf\xe9</h1>', 'latin1');
      const output = await api.render(new Uint8Array(html));
      assert.equal(output.headings[0].text, 'café');
    });

    test('fetches a stylesheet, its import, a font and images, each once', async () => {
      const server = await serve({
        '/site/main.css': { type: 'text/css', body: "@import 'more/extra.css'; h1 { background: url(bg.png) }" },
        '/site/more/extra.css': {
          type: 'text/css',
          body: `@font-face { font-family: Remote; src: url(../mono.ttf) } body { font-family: Remote } ${PAGE_BREAK_CSS}`,
        },
        '/site/mono.ttf': { type: 'font/ttf', body: FONT },
        '/site/bg.png': { type: 'image/png', body: PNG },
        '/site/photo.png': { type: 'image/png', body: PNG },
      });
      try {
        const html =
          '<!doctype html><link rel=stylesheet href=main.css><h1>Title</h1><p>one <img src=photo.png></p><p>two</p>';
        const output = await api.render(html, { baseUrl: `${server.origin}/site/index.html` });
        assertPdf(output.pdf);
        assert.deepEqual(messages(output.diagnostics), []);
        assert.equal(output.pageCount, 2, 'the imported sheet breaks after each paragraph');
        assert.deepEqual([...server.requests].sort(), [
          '/site/bg.png',
          '/site/main.css',
          '/site/mono.ttf',
          '/site/more/extra.css',
          '/site/photo.png',
        ]);
        // A sheet is fetched before what it names, so discovery took one round per level.
        const order = (file) => server.requests.indexOf(`/site/${file}`);
        assert.ok(order('main.css') < order('more/extra.css'), server.requests.join(' '));
        assert.ok(order('more/extra.css') < order('mono.ttf'), server.requests.join(' '));

        const withoutFont = await api.render(html.replace('main.css', 'missing.css'), {
          baseUrl: `${server.origin}/site/index.html`,
        });
        assert.notEqual(
          Buffer.compare(output.pdf, withoutFont.pdf),
          0,
          'the fetched font and styles must change the PDF',
        );
      } finally {
        await server.close();
      }
    });

    test('a 404 and a refused connection become diagnostics and still a PDF', async () => {
      const server = await serve({});
      const refused = await refusedOrigin();
      try {
        const output = await api.render(
          `<!doctype html><link rel=stylesheet href=missing.css><p>one <img src="${refused}/gone.png" alt=gone></p>`,
          { baseUrl: `${server.origin}/` },
        );
        assertPdf(output.pdf);
        assert.equal(output.pageCount, 1);
        const reported = messages(output.diagnostics);
        assert.ok(
          reported.some((message) => message.includes('missing.css') && message.includes('HTTP 404')),
          reported.join('\n'),
        );
        assert.ok(
          reported.some((message) => message.includes(`${refused}/gone.png`)),
          reported.join('\n'),
        );
        assert.ok(output.diagnostics.every((diagnostic) => !diagnostic.losesContent));
      } finally {
        await server.close();
      }
    });

    test('a custom fetch function loads every remote URL', async () => {
      const requested = [];
      const responses = {
        'https://cdn.example/assets/style.css': () =>
          new Response("@import 'breaks.css';", { headers: { 'content-type': 'text/css' } }),
        'https://cdn.example/assets/breaks.css': () => new TextEncoder().encode(PAGE_BREAK_CSS),
        'https://cdn.example/assets/photo.png': () => ({ data: PNG, contentType: 'image/png' }),
        'https://cdn.example/assets/denied.png': () => {
          throw new Error('403 from the proxy');
        },
        'https://cdn.example/assets/absent.png': () => null,
      };
      const output = await api.render(
        '<!doctype html><link rel=stylesheet href=style.css><p>one <img src=photo.png></p>' +
          '<p>two <img src=denied.png alt=denied><img src=absent.png alt=absent></p>',
        {
          baseUrl: 'https://cdn.example/assets/page.html',
          fetch: async (url) => {
            requested.push(url);
            return responses[url]();
          },
        },
      );
      assert.deepEqual([...requested].sort(), Object.keys(responses).sort());
      assert.equal(output.pageCount, 2, messages(output.diagnostics).join('\n'));
      const reported = messages(output.diagnostics);
      assert.equal(reported.length, 2, reported.join('\n'));
      assert.ok(reported.some((message) => message.includes('403 from the proxy')), reported.join('\n'));
      assert.ok(reported.some((message) => message.includes('absent.png')), reported.join('\n'));
    });

    test("a fetched resource's final URL is the base for the URLs inside it", async () => {
      const requested = [];
      const responses = {
        'https://cdn.example/style.css': { data: new TextEncoder().encode("@import 'breaks.css';"), url: 'https://mirror.example/v2/style.css' },
        'https://mirror.example/v2/breaks.css': new TextEncoder().encode(PAGE_BREAK_CSS),
      };
      const output = await api.render(LINKED, {
        baseUrl: 'https://cdn.example/',
        fetch: (url) => {
          requested.push(url);
          return responses[url];
        },
      });
      assert.deepEqual(requested, Object.keys(responses));
      assert.equal(output.pageCount, 2, messages(output.diagnostics).join('\n'));
    });

    test('fetch round and byte limits leave resources unavailable with a diagnostic', async () => {
      const css = new TextEncoder().encode(PAGE_BREAK_CSS);
      const options = { baseUrl: 'https://cdn.example/', fetch: () => css };

      const offline = await api.render(LINKED, { ...options, maxFetchRounds: 0 });
      assert.equal(offline.pageCount, 1);
      assert.match(messages(offline.diagnostics).join('\n'), /limit of 0 fetch rounds/);

      const small = await api.render(LINKED, { ...options, maxFetchBytes: css.length - 1 });
      assert.equal(small.pageCount, 1);
      assert.match(messages(small.diagnostics).join('\n'), new RegExp(`limit of ${css.length - 1} fetched bytes`));

      const exact = await api.render(LINKED, { ...options, maxFetchBytes: css.length });
      assert.equal(exact.pageCount, 2, messages(exact.diagnostics).join('\n'));
    });

    test('reads local files below a directory base and refuses those outside fileRoot', async () => {
      const root = path.join(directory, 'site');
      await mkdir(root);
      await writeFile(path.join(root, 'style.css'), PAGE_BREAK_CSS);
      await writeFile(path.join(root, 'photo.png'), PNG);
      await writeFile(path.join(directory, 'outside.css'), PAGE_BREAK_CSS);
      const html = LINKED.replace('one', 'one <img src=photo.png>');

      const local = await api.render(html, { base: root });
      assert.equal(local.pageCount, 2, messages(local.diagnostics).join('\n'));
      assert.deepEqual(messages(local.diagnostics), []);

      const byUrl = await api.render(html, { baseUrl: pathToFileURL(path.join(root, 'index.html')).href });
      assert.equal(byUrl.pageCount, 2, messages(byUrl.diagnostics).join('\n'));

      const elsewhere = await api.render(html, { base: path.join(directory, 'missing') });
      assert.equal(elsewhere.pageCount, 1);
      assert.equal(elsewhere.diagnostics.length, 2, messages(elsewhere.diagnostics).join('\n'));

      const escaping = LINKED.replace('style.css', '../outside.css');
      const unrestricted = await api.render(escaping, { base: root });
      assert.equal(unrestricted.pageCount, 2, messages(unrestricted.diagnostics).join('\n'));
      const restricted = await api.render(escaping, { base: root, fileRoot: root });
      assert.equal(restricted.pageCount, 1);
      assert.match(messages(restricted.diagnostics).join('\n'), /escapes the allowed root/);

      await assert.rejects(api.render(html, { base: root, baseUrl: 'https://example.com/' }), TypeError);
    });

    test('options change the render and invalid ones are rejected', async () => {
      const dark = `${TWO_PARAGRAPHS}<style>@media (prefers-color-scheme: dark) { ${PAGE_BREAK_CSS} }</style>`;
      assert.equal((await api.render(dark)).pageCount, 1);
      assert.equal((await api.render(dark, { colorScheme: 'dark' })).pageCount, 2);
      assert.equal((await api.render(TWO_PARAGRAPHS, { stylesheets: [PAGE_BREAK_CSS] })).pageCount, 2);

      const letter = await api.render(TWO_PARAGRAPHS, { paper: [612, 792] });
      assert.ok(Buffer.from(letter.pdf).includes('/MediaBox[0 0 612 792]'), 'letter paper sets the media box');

      const traced = await api.render(TWO_PARAGRAPHS, { includeLayout: true });
      assert.equal(traced.layout.pages.length, traced.pageCount);

      await assert.rejects(api.render(TWO_PARAGRAPHS, { jpegQuality: 0 }), /jpegQuality/);
      await assert.rejects(api.render(TWO_PARAGRAPHS, { systemFonts: true }), /systemFonts/);
      await assert.rejects(api.render(TWO_PARAGRAPHS, { paper: [0, 10] }), Error);
      await assert.rejects(api.render(42), TypeError);
    });

    test('lost content rejects with the losses unless allowed', async () => {
      await assert.rejects(api.render(OMITTED_BOX), (error) => {
        assert.ok(error instanceof api.ContentLostError, `${error.name}: ${error.message}`);
        assert.ok(error instanceof api.RenderError);
        assert.equal(error.name, 'ContentLostError');
        assert.equal(error.part, null);
        assert.equal(error.diagnostics.length, 1, messages(error.diagnostics).join('\n'));
        assert.ok(error.diagnostics[0].losesContent);
        assert.match(error.message, /content left out of the PDF/);
        return true;
      });
      const output = await api.render(OMITTED_BOX, { allowContentLoss: true });
      assertPdf(output.pdf);
      assert.ok(output.diagnostics.some((diagnostic) => diagnostic.losesContent));
    });

    test('renders parts into one PDF, each with its own base', async () => {
      const root = path.join(directory, 'parts');
      await mkdir(root);
      await writeFile(path.join(root, 'style.css'), PAGE_BREAK_CSS);
      const output = await api.renderParts([
        '<!doctype html><h1>Cover</h1>',
        { html: '<!doctype html><link rel=stylesheet href=style.css><h1>Body</h1><p>a</p><p>b</p>', base: root },
        new TextEncoder().encode('<!doctype html><h1>Appendix</h1>'),
      ]);
      assertPdf(output.pdf);
      assert.deepEqual(
        output.parts.map((part) => [part.firstPage, part.pageCount]),
        [[1, 1], [2, 2], [4, 1]],
      );
      assert.equal(output.pageCount, 4);
      assert.deepEqual(
        output.parts.flatMap((part) => part.headings.map((heading) => heading.page)),
        [1, 2, 4],
      );

      await assert.rejects(api.renderParts(['<!doctype html><p>fine</p>', OMITTED_BOX]), (error) => {
        assert.ok(error instanceof api.ContentLostError, `${error.name}: ${error.message}`);
        assert.equal(error.part, 2);
        return true;
      });
      await assert.rejects(api.renderParts([]), (error) => {
        assert.ok(error instanceof api.RenderError, `${error.name}: ${error.message}`);
        assert.match(error.message, /at least one part/);
        return true;
      });
    });

    for (const [name, css, open, close] of NESTED_CONSTRUCTS) {
      test(`${name} nested past the formatting limit render without a trap`, async () => {
        const html = `<!doctype html><style>${css}</style>${open.repeat(NESTING_PAST_LIMIT)}Deepest${close.repeat(NESTING_PAST_LIMIT)}`;
        const output = await api.render(html, { allowContentLoss: true });
        assertPdf(output.pdf);
        assert.match(messages(output.diagnostics).join('\n'), /nesting exceeds/);
      });
    }

    test('a document without resources renders the same bytes as the native renderer', { skip: !NATIVE_CLI && 'set FERRI_PDF_CLI to the native binary' }, async () => {
      const input = path.join(directory, 'native.html');
      const native = path.join(directory, 'native.pdf');
      await writeFile(input, ARTICLE);
      await promisify(execFile)(NATIVE_CLI, [input, native, '--jpeg-quality', '80', '--font-serif', 'DejaVu Serif']);
      const output = await api.render(new TextEncoder().encode(ARTICLE), {
        base: directory,
        jpegQuality: 80,
        serif: ['DejaVu Serif'],
      });
      assert.ok(output.pageCount > 2, `the article has ${output.pageCount} pages`);
      const expected = await readFile(native);
      assert.equal(output.pdf.length, expected.length, 'PDF lengths differ');
      assert.equal(Buffer.compare(output.pdf, expected), 0, 'PDF bytes differ');
    });

    test('a worker thread renders, calling a custom fetch function on the calling thread', async () => {
      const worker = createRenderWorker();
      try {
        const requested = [];
        const output = await worker.render(LINKED, {
          baseUrl: 'https://cdn.example/',
          fetch: (url) => {
            requested.push(url);
            return new TextEncoder().encode(PAGE_BREAK_CSS);
          },
        });
        assertPdf(output.pdf);
        assert.deepEqual(requested, ['https://cdn.example/style.css']);
        assert.equal(output.pageCount, 2, messages(output.diagnostics).join('\n'));

        const parts = await worker.renderParts([TWO_PARAGRAPHS, TWO_PARAGRAPHS]);
        assert.equal(parts.pageCount, 2);

        await assert.rejects(worker.render(OMITTED_BOX), (error) => {
          assert.equal(error.name, 'ContentLostError');
          assert.equal(error.diagnostics.length, 1);
          return true;
        });

        const [name, css, open, close] = NESTED_CONSTRUCTS.at(-1);
        const nested = await worker.render(
          `<!doctype html><style>${css}</style>${open.repeat(NESTING_PAST_LIMIT)}Deepest${close.repeat(NESTING_PAST_LIMIT)}`,
          { allowContentLoss: true },
        );
        assert.match(messages(nested.diagnostics).join('\n'), /nesting exceeds/, name);
      } finally {
        await worker.terminate();
      }
    });
  });
}
