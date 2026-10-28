"""Build and render as an external dependency without consumer Cargo patches."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]


CONSUMER_SOURCE = r'''
use ferri_pdf::{render_html, RenderOptions};
fn main() {
    let root = std::path::PathBuf::from(std::env::args_os().nth(1).unwrap());
    let profile = std::fs::read_to_string(root.join("tests/oracle/fixtures/profile.css")).unwrap();
    let options = RenderOptions {
        base: root.join("tests/oracle/fixtures").into(),
        include_layout: true,
        ..Default::default()
    };
    let html = format!("<style>{profile}section:has(h1){{color:rebeccapurple}}</style>\
        <section><h1>Consumer</h1><a href='mailto:reader@example.org'>Link</a>\
        <img src='../assets/image-pattern.png'>\
        <svg xmlns='http://www.w3.org/2000/svg' width='40' height='20'>\
        <rect width='40' height='20' fill='blue'/></svg></section>");
    let output = render_html(&html, &options).expect("external consumer must render");
    assert!(output.pdf.starts_with(b"%PDF-"));
    assert!(output.diagnostics.is_empty(), "{:?}", output.diagnostics);
    assert!(output.layout.unwrap().pages.iter().flatten().any(|line| line.text == "Consumer"));
}
'''


class PackageConsumer(unittest.TestCase):
    def test_external_consumer_renders_with_and_without_default_features(self):
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-consumer-') as directory:
            project = Path(directory)
            (project / 'src').mkdir()
            (project / 'Cargo.toml').write_text(
                '[package]\nname="ferri-pdf-consumer-check"\nversion="0.0.0"\nedition="2024"\n'
                '[features]\ndefault=["http"]\nhttp=["ferri-pdf/http"]\n'
                '[dependencies]\nferri-pdf={path=' + json.dumps(str(ROOT)) + ',default-features=false}\n')
            (project / 'src/main.rs').write_text(CONSUMER_SOURCE)
            environment = dict(os.environ, CARGO_PROFILE_DEV_DEBUG='0', CARGO_INCREMENTAL='0')
            environment.setdefault('CARGO_TARGET_DIR', str(ROOT / 'target'))
            for features in ([], ['--no-default-features']):
                with self.subTest(features=features):
                    result = subprocess.run(
                        ['cargo', 'run', '--offline', '-j', '2', '--manifest-path',
                         str(project / 'Cargo.toml'), *features, '--', str(ROOT)],
                        cwd=project, env=environment, text=True, capture_output=True)
                    self.assertEqual(result.returncode, 0, result.stdout + result.stderr)


if __name__ == '__main__':
    unittest.main(verbosity=2)
