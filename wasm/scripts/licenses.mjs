// Copies the licence files of everything the module embeds into the package, from the
// list the Python wheel ships (`license-files` in pyproject.toml).
import { cpSync, globSync, mkdirSync, readFileSync, rmSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const PACKAGE = join(dirname(fileURLToPath(import.meta.url)), '..');
const REPOSITORY = join(PACKAGE, '..');
const TARGET = join(PACKAGE, 'licenses');
const LICENSE_FILES = /^license-files = \[\n([^\]]*)\]/m;

const listed = LICENSE_FILES.exec(readFileSync(join(REPOSITORY, 'pyproject.toml'), 'utf8'));
if (!listed) {
  throw new Error('pyproject.toml has no license-files list');
}
const patterns = [...listed[1].matchAll(/"([^"]+)"/g)].map(([, pattern]) => pattern);
rmSync(TARGET, { recursive: true, force: true });
for (const pattern of patterns) {
  const files = globSync(pattern, { cwd: REPOSITORY });
  if (files.length === 0) {
    throw new Error(`no licence file matches ${pattern}`);
  }
  for (const file of files) {
    mkdirSync(dirname(join(TARGET, file)), { recursive: true });
    cpSync(join(REPOSITORY, file), join(TARGET, file));
  }
}
