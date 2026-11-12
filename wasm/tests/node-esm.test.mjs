// The package as Node resolves `import 'ferri-pdf'`.
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { test } from 'node:test';

import * as ferri from 'ferri-pdf';
import { createRenderWorker } from 'ferri-pdf/worker';

import { defineSuite } from './suite.mjs';
import { REPOSITORY } from './support.mjs';

defineSuite('ES module entry', async () => ferri, createRenderWorker);

test('the npm package has the version of the Rust workspace', async () => {
  const manifest = await readFile(new URL('Cargo.toml', REPOSITORY), 'utf8');
  const [, version] = /\[workspace\.package\]\nversion = "([^"]+)"/.exec(manifest);
  const { version: packaged } = JSON.parse(await readFile(new URL('wasm/package.json', REPOSITORY), 'utf8'));
  assert.equal(packaged, version, 'bump wasm/package.json with the workspace version');
});
