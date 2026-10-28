"""Verify the published data archive without access to sibling checkout files."""
import os
from pathlib import Path
import subprocess
import tarfile
import tempfile
import tomllib
import unittest

ROOT = Path(__file__).resolve().parents[1]
PACKAGE = 'ferri-pdf-spec-data'
VERSION = tomllib.loads((ROOT / 'Cargo.toml').read_text())['workspace']['package']['version']


class SpecPackage(unittest.TestCase):
    def test_archive_preserves_frozen_inputs_and_compiles_in_isolation(self):
        environment = dict(os.environ, CARGO_PROFILE_DEV_DEBUG='0', CARGO_INCREMENTAL='0')
        environment.setdefault('CARGO_TARGET_DIR', str(ROOT / 'target'))
        packaged = subprocess.run(
            ['cargo', 'package', '--offline', '--allow-dirty', '--no-verify', '-p', PACKAGE],
            cwd=ROOT, env=environment, text=True, capture_output=True)
        self.assertEqual(packaged.returncode, 0, packaged.stdout + packaged.stderr)
        archive = Path(environment['CARGO_TARGET_DIR']) / 'package' / f'{PACKAGE}-{VERSION}.crate'
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-spec-package-') as directory:
            with tarfile.open(archive) as source:
                source.extractall(directory, filter='data')
            extracted = Path(directory) / f'{PACKAGE}-{VERSION}'
            for group in ('css', 'languages', 'cldr'):
                for source in (ROOT / 'spec' / group).rglob('*'):
                    if source.is_file():
                        relative = source.relative_to(ROOT / 'spec')
                        self.assertEqual((extracted / relative).read_bytes(), source.read_bytes(),
                                         f'archive omitted or changed pinned input {relative}')
            checked = subprocess.run(
                ['cargo', 'check', '--offline', '-j', '2', '--manifest-path', str(extracted / 'Cargo.toml')],
                cwd=directory, env=environment, text=True, capture_output=True)
            self.assertEqual(checked.returncode, 0, checked.stdout + checked.stderr)


if __name__ == '__main__':
    unittest.main(verbosity=2)
