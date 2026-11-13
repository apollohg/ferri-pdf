"""The repository as the folder a render may read local files in.

A document reads local files only inside its own folder unless `--file-root` names another.
The suites write their documents to temporary folders and name the repository's fonts and
images from them, so those folders are made inside the repository and it is the file root.
"""
from pathlib import Path
import tempfile

ROOT = Path(__file__).resolve().parents[1]
SCRATCH = ROOT / "target/pdf-suites"
FILE_ROOT = ("--file-root", str(ROOT))

SCRATCH.mkdir(parents=True, exist_ok=True)
tempfile.tempdir = str(SCRATCH)
