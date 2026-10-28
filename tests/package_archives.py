"""Resolve and render using actual Cargo archives, with no checkout dependencies."""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tarfile
import tempfile
import tomllib
import unittest

from package_consumer import CONSUMER_SOURCE, ROOT


class PackageArchives(unittest.TestCase):
    def run_cargo(self, arguments, cwd):
        result = subprocess.run(['cargo', *arguments], cwd=cwd, env=self.environment,
                                text=True, capture_output=True)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        return result.stdout

    def test_registry_consumer_uses_only_packaged_local_sources(self):
        self.environment = dict(os.environ, CARGO_PROFILE_DEV_DEBUG='0', CARGO_INCREMENTAL='0')
        self.environment.setdefault('CARGO_TARGET_DIR', str(ROOT / 'target'))
        metadata = json.loads(self.run_cargo(['metadata', '--offline', '--locked',
                                             '--format-version', '1'], ROOT))
        packages = [package for package in metadata['packages'] if package['source'] is None]
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-archives-') as directory:
            temporary = Path(directory)
            sources = temporary / 'sources'
            self.run_cargo(['vendor', '--offline', '--locked', '--versioned-dirs', str(sources)], ROOT)
            for package in packages:
                with self.subTest(package=package['name']):
                    self.run_cargo(['package', '--offline', '--allow-dirty', '--no-verify',
                                    '--exclude-lockfile', '--manifest-path', package['manifest_path']], ROOT)
                    name = f"{package['name']}-{package['version']}"
                    archive = Path(self.environment['CARGO_TARGET_DIR']) / 'package' / f'{name}.crate'
                    with tarfile.open(archive) as contents:
                        contents.extractall(sources, filter='data')
                    unpacked = sources / name
                    manifest = tomllib.loads((unpacked / 'Cargo.toml').read_text())
                    def check_dependencies(value):
                        for key, child in value.items():
                            if key in ('dependencies', 'dev-dependencies', 'build-dependencies'):
                                for dependency in child.values():
                                    self.assertNotIn('path', dependency,
                                                     f'{name} retained a checkout dependency')
                            elif isinstance(child, dict):
                                check_dependencies(child)
                    check_dependencies(manifest)
                    files = {str(path.relative_to(unpacked)): hashlib.sha256(path.read_bytes()).hexdigest()
                             for path in unpacked.rglob('*') if path.is_file()}
                    (unpacked / '.cargo-checksum.json').write_text(json.dumps({
                        'files': files, 'package': hashlib.sha256(archive.read_bytes()).hexdigest()}))
            project = temporary / 'consumer'
            (project / 'src').mkdir(parents=True)
            (project / '.cargo').mkdir()
            (project / '.cargo/config.toml').write_text(
                '[source.crates-io]\nreplace-with="packaged"\n[source.packaged]\ndirectory='
                + json.dumps(str(sources)) + '\n')
            version = next(package['version'] for package in packages if package['name'] == 'ferri-pdf')
            (project / 'Cargo.toml').write_text(
                '[package]\nname="ferri-pdf-archive-consumer"\nversion="0.0.0"\nedition="2024"\n'
                '[features]\ndefault=["http"]\nhttp=["ferri-pdf/http"]\n[dependencies]\n'
                'ferri-pdf={version=' + json.dumps('=' + version) + ',default-features=false}\n')
            (project / 'src/main.rs').write_text(CONSUMER_SOURCE)
            for features in ([], ['--no-default-features']):
                self.run_cargo(['run', '--offline', '-j', '2', *features, '--', str(ROOT)], project)
            library_manifest = sources / f'ferri-pdf-{version}' / 'Cargo.toml'
            self.run_cargo(['check', '--offline', '-j', '2', '--bin', 'ferri-pdf',
                            '--manifest-path', str(library_manifest)], project)
            resolved = json.loads(self.run_cargo(['metadata', '--offline', '--format-version', '1'], project))
            local_names = {package['name'] for package in packages}
            print(f'Verified {len(packages)} local archives, both library feature configurations, and CLI', flush=True)
            for package in resolved['packages']:
                if package['name'] in local_names:
                    self.assertTrue(Path(package['manifest_path']).is_relative_to(sources),
                                    f"{package['name']} resolved outside extracted archives")


if __name__ == '__main__':
    unittest.main(verbosity=2)
