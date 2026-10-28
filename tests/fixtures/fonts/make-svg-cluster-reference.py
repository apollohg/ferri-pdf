"""Measure a fallback combining cluster independently with HarfBuzz and FontTools."""
import hashlib
import json
from pathlib import Path
import subprocess

import fontTools
from fontTools.ttLib import TTFont

ROOT = Path(__file__).resolve().parents[3]
SOURCE = Path("assets/fonts/DejaVuSans.ttf")
TEXT = "a\u0346"
command = ["hb-shape", str(ROOT / SOURCE), TEXT, "--output-format=json",
           "--no-glyph-names", "--direction=ltr", "--language=en"]
with TTFont(ROOT / SOURCE) as font:
    result = {
        "tools": [subprocess.check_output(["hb-shape", "--version"], text=True).splitlines()[0],
                  "fontTools " + fontTools.__version__],
        "source": SOURCE.as_posix(),
        "sha256": hashlib.sha256((ROOT / SOURCE).read_bytes()).hexdigest(),
        "units_per_em": font["head"].unitsPerEm,
        "text": TEXT,
        "options": command[3:],
        "glyphs": json.loads(subprocess.check_output(command, text=True)),
    }
Path(__file__).with_name("svg-cluster-reference.json").write_text(json.dumps(result, indent=2) + "\n")
