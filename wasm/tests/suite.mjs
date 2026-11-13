// The behaviour every Node entry must have; `node-esm.test.mjs` and `node-cjs.test.mjs` run
// it against the package's `import` and `require` conditions.
import assert from 'node:assert/strict';
import { execFile } from 'node:child_process';
import fsPromises, { mkdir, mkdtemp, readFile, rm, symlink, writeFile } from 'node:fs/promises';
import { syncBuiltinESMExports } from 'node:module';
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

    test('reads local files below a directory base or a file: base URL', async () => {
      const root = path.join(directory, 'site');
      await mkdir(root);
      await writeFile(path.join(root, 'style.css'), PAGE_BREAK_CSS);
      await writeFile(path.join(root, 'photo.png'), PNG);
      const html = LINKED.replace('one', 'one <img src=photo.png>');

      const local = await api.render(html, { base: root });
      assert.equal(local.pageCount, 2, messages(local.diagnostics).join('\n'));
      assert.deepEqual(messages(local.diagnostics), []);

      const byUrl = await api.render(html, { baseUrl: pathToFileURL(path.join(root, 'index.html')).href });
      assert.equal(byUrl.pageCount, 2, messages(byUrl.diagnostics).join('\n'));

      const elsewhere = await api.render(html, { base: path.join(directory, 'missing') });
      assert.equal(elsewhere.pageCount, 1);
      assert.equal(elsewhere.diagnostics.length, 2, messages(elsewhere.diagnostics).join('\n'));

      await assert.rejects(api.render(html, { base: root, baseUrl: 'https://example.com/' }), TypeError);
    });

    describe('local file confinement', () => {
      let root;
      let secret;
      /** A folder beside `root`, for a second part or a wider `fileRoot`. */
      let other;
      /** A custom fetch function that must never be handed a `file:` URL. */
      const noFiles = (responses = {}) => (url) => {
        assert.ok(!url.startsWith('file:'), `fetch was asked for ${url}`);
        return responses[url] ?? null;
      };
      /** Renders a document linking `href` and reports whether the secret sheet applied. */
      async function linking(href, options) {
        const output = await api.render(LINKED.replace('style.css', href), options);
        return { leaked: output.pageCount === 2, reported: messages(output.diagnostics).join('\n') };
      }

      before(async () => {
        root = path.join(directory, 'confined');
        await mkdir(root);
        await mkdir(path.join(root, 'folder.css'));
        secret = path.join(directory, 'secret.css');
        await writeFile(secret, PAGE_BREAK_CSS);
        await writeFile(path.join(root, 'style.css'), PAGE_BREAK_CSS);
        await symlink(secret, path.join(root, 'link.css'));
        await mkdir(path.join(root, 'sub'));
        await writeFile(path.join(root, 'sub', 'plain.css'), PAGE_BREAK_CSS);
        await writeFile(path.join(root, 'sub', 'inside.css'), "@import '../style.css';");
        await writeFile(path.join(root, 'sub', 'escape.css'), "@import '../../secret.css';");
        other = path.join(directory, 'other');
        await mkdir(other);
        await writeFile(path.join(other, 'shared.css'), PAGE_BREAK_CSS);
        await symlink(path.join(other, 'shared.css'), path.join(root, 'other.css'));
      });

      test('without a base no file is read', async () => {
        const absolute = pathToFileURL(path.join(root, 'style.css')).href;
        for (const options of [undefined, { fetch: noFiles() }]) {
          const { leaked, reported } = await linking(absolute, options);
          assert.equal(leaked, false, 'a document must not read files the caller did not point it at');
          assert.match(reported, /no local base or file root/);
          assert.ok(reported.includes(absolute), `the diagnostic names the file: ${reported}`);
        }
        const directoryBefore = process.cwd();
        process.chdir(root);
        try {
          const relative = await linking('style.css', { fetch: noFiles() });
          assert.equal(relative.leaked, false, 'a relative path in the working directory was read');
          assert.match(relative.reported, /style\.css has no base/);
        } finally {
          process.chdir(directoryBefore);
        }
        const rooted = await linking(absolute, { fileRoot: root, fetch: noFiles() });
        assert.deepEqual(rooted, { leaked: true, reported: '' }, 'fileRoot allows a file: URL inside it');
      });

      test('an absolute path, a traversal and a symlink out of the base are refused', async () => {
        for (const href of [pathToFileURL(secret).href, '../secret.css', 'link.css', 'sub/../../secret.css']) {
          for (const options of [{ base: root }, { base: root, fileRoot: root }, { base: root, fetch: noFiles() }]) {
            const { leaked, reported } = await linking(href, options);
            assert.equal(leaked, false, `${href} was read with ${JSON.stringify(Object.keys(options))}`);
            assert.match(reported, /outside the allowed folder/, href);
            assert.ok(reported.includes(href), `the diagnostic names ${href}: ${reported}`);
          }
        }
        const inside = await linking('style.css', { base: root, fileRoot: root, fetch: noFiles() });
        assert.deepEqual(inside, { leaked: true, reported: '' });
      });

      test('the folder is the document\'s, whichever sheet names the file', async () => {
        const inside = await linking('sub/inside.css', { base: root });
        assert.deepEqual(inside, { leaked: true, reported: '' }, 'a sheet in a subfolder importing ../style.css');
        const escape = await linking('sub/escape.css', { base: root });
        assert.equal(escape.leaked, false, 'a sheet in a subfolder read outside the base');
        assert.match(escape.reported, /secret\.css.*outside the allowed folder/);
      });

      test('fileRoot replaces the folder of the base', async () => {
        const wider = await linking('../secret.css', { base: root, fileRoot: directory });
        assert.equal(wider.leaked, true, wider.reported);
        const sibling = await linking('../other/shared.css', { base: root, fileRoot: directory });
        assert.equal(sibling.leaked, true, sibling.reported);
        const narrow = { base: root, fileRoot: path.join(root, 'sub') };
        assert.deepEqual(await linking('sub/plain.css', narrow), { leaked: true, reported: '' });
        const narrower = await linking('style.css', narrow);
        assert.equal(narrower.leaked, false, 'the base folder is outside the narrower fileRoot');
        assert.match(narrower.reported, /outside the allowed folder/);
      });

      test('remote content reads no file, whatever the base and fileRoot', async () => {
        const absolute = pathToFileURL(path.join(root, 'style.css')).href;
        const importing = new TextEncoder().encode(`@import '${absolute}';`);
        for (const fileRoot of [undefined, directory]) {
          const direct = await linking(absolute, { baseUrl: 'https://cdn.example/', fileRoot, fetch: noFiles() });
          assert.equal(direct.leaked, false, 'file: reference from an https document');
          assert.match(direct.reported, /the content that names it is not local/);

          for (const base of [{ baseUrl: 'https://cdn.example/' }, { base: root }]) {
            const imported = await linking('https://cdn.example/remote.css', {
              ...base,
              fileRoot,
              fetch: noFiles({ 'https://cdn.example/remote.css': importing }),
            });
            assert.equal(imported.leaked, false, `file: import from a remote stylesheet with ${JSON.stringify(base)}`);
            assert.match(imported.reported, /the content that names it is not local/);

            const redirected = await linking('https://cdn.example/moved.css', {
              ...base,
              fileRoot,
              fetch: noFiles({
                'https://cdn.example/moved.css': { data: new TextEncoder().encode("@import 'style.css';"), url: pathToFileURL(path.join(root, 'moved.css')).href },
              }),
            });
            assert.equal(redirected.leaked, false, 'a response that claims a file: location');
            assert.match(redirected.reported, /places a remote resource at a local file/);
          }
        }
      });

      test('each part of a multipart render reads only the folder of its own base', async () => {
        const part = (href, base) => ({ html: LINKED.replace('style.css', href), base });
        const pages = (output) => output.parts.map((entry) => entry.pageCount);
        const confined = await api.renderParts([part('style.css', root), part('../secret.css', root)]);
        assert.deepEqual(pages(confined), [2, 1]);

        const shared = pathToFileURL(path.join(other, 'shared.css')).href;
        // The other part's file is asked for before and after that part reads it itself.
        const parts = [part(shared, root), part('shared.css', other), part(shared, root), part('other.css', root), LINKED.replace('style.css', shared)];
        const separate = await api.renderParts(parts);
        assert.deepEqual(pages(separate), [1, 2, 1, 1, 1], 'a part read the folder of another part');
        for (const index of [0, 2, 3]) {
          assert.match(messages(separate.parts[index].diagnostics).join('\n'), /outside the allowed folder/);
        }
        assert.match(messages(separate.parts[4].diagnostics).join('\n'), /no local base or file root/);
        const rooted = await api.renderParts(parts, { fileRoot: directory });
        assert.deepEqual(pages(rooted), [2, 2, 2, 2, 2], 'fileRoot covers every part');

        const mixed = await api.renderParts([part('style.css', root), { html: LINKED, baseUrl: 'https://cdn.example/a.html' }], { fetch: noFiles() });
        assert.deepEqual(pages(mixed), [2, 1], 'a remote part reads no file and leaves the local part its folder');
      });

      test('a file replaced around the open is refused', async () => {
        const swapped = path.join(root, 'swap.css');
        // Before the open the path can only be made to lead elsewhere by a link; after it,
        // the open handle no longer is the file the checked path names.
        const replacements = {
          'a link out of the base before the open': [true, () => symlink(secret, swapped)],
          'a link out of the base after the open': [false, () => symlink(secret, swapped)],
          'another file after the open': [false, () => writeFile(swapped, 'p { color: blue }')],
        };
        const open = fsPromises.open;
        try {
          for (const [kind, [before, replace]] of Object.entries(replacements)) {
            await rm(swapped, { force: true });
            await writeFile(swapped, PAGE_BREAK_CSS);
            let swaps = 0;
            // The reader has resolved and checked the path by now; opening it is the next step.
            fsPromises.open = async (file, ...rest) => {
              const swap = async () => {
                if (String(file).endsWith('swap.css')) {
                  swaps += 1;
                  await rm(swapped);
                  await replace();
                }
              };
              if (before) {
                await swap();
              }
              const handle = await open(file, ...rest);
              if (!before) {
                await swap();
              }
              return handle;
            };
            syncBuiltinESMExports();
            const { leaked, reported } = await linking('swap.css', { base: root });
            assert.equal(swaps, 1, `${kind}: the file must be swapped once the path is checked`);
            assert.equal(leaked, false, `${kind}: the file was read`);
            assert.match(reported, /changed while it was being opened|ELOOP/, kind);
          }
        } finally {
          fsPromises.open = open;
          syncBuiltinESMExports();
        }
      });

      test('only regular files within the byte budget are read', async () => {
        const folder = await linking('folder.css', { base: root });
        assert.equal(folder.leaked, false);
        assert.match(folder.reported, /not a regular file/);

        const size = Buffer.byteLength(PAGE_BREAK_CSS);
        const over = await linking('style.css', { base: root, maxFetchBytes: size - 1 });
        assert.equal(over.leaked, false);
        assert.match(over.reported, new RegExp(`limit of ${size - 1} fetched bytes`));
        const exact = await linking('style.css', { base: root, maxFetchBytes: size });
        assert.deepEqual(exact, { leaked: true, reported: '' });
      });
    });

    test('a response over the byte budget is refused without buffering it', async () => {
      const budget = 64;
      let pulled = 0;
      let cancelled = 0;
      const endless = () =>
        new ReadableStream({
          pull(controller) {
            pulled += 1;
            controller.enqueue(new Uint8Array(16));
          },
          cancel() {
            cancelled += 1;
          },
        });
      const options = { baseUrl: 'https://cdn.example/', maxFetchBytes: budget };

      const streamed = await api.render(LINKED, { ...options, fetch: () => new Response(endless()) });
      assert.match(messages(streamed.diagnostics).join('\n'), /limit of 64 fetched bytes/);
      assert.equal(cancelled, 1, 'the stream is cancelled once it passes the budget');
      assert.ok(pulled <= budget / 16 + 2, `read ${pulled} chunks of an endless body`);

      pulled = 0;
      const declared = await api.render(LINKED, {
        ...options,
        fetch: () => new Response(endless(), { headers: { 'content-length': String(budget + 1) } }),
      });
      assert.match(messages(declared.diagnostics).join('\n'), /limit of 64 fetched bytes/);
      assert.ok(pulled <= 1, `a declared length over the budget was read (${pulled} chunks)`);

      const worker = createRenderWorker();
      try {
        pulled = 0;
        const proxied = await worker.render(LINKED, { ...options, fetch: () => new Response(endless()) });
        assert.match(messages(proxied.diagnostics).join('\n'), /limit of 64 fetched bytes/);
        assert.ok(pulled <= budget / 16 + 2, `the calling thread read ${pulled} chunks for the worker`);
        const bytes = await worker.render(LINKED, { ...options, fetch: () => new Uint8Array(budget + 1) });
        assert.match(messages(bytes.diagnostics).join('\n'), /limit of 64 fetched bytes/);
      } finally {
        await worker.terminate();
      }
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
