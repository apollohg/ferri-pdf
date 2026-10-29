"""Subset the pinned CJK source while retaining its vertical substitutions and metrics."""
import hashlib
import json
from pathlib import Path
import subprocess
import sys

import fontTools
from fontTools import subset
from fontTools.ttLib import TTFont

FIXTURES = Path(__file__).resolve().parent
PROVENANCE = json.loads((FIXTURES / "CJKSpacingTest-source.json").read_text())
TARGET = FIXTURES / "CJKWritingTest.otf"
REFERENCE = FIXTURES / "CJKWritingTest-source.json"
TEXT = "漢字水あア「」（）〔〕、。ＡA0123456789"
FEATURES = ["vert", "vrt2", "vkrn", "hwid", "twid", "qwid"]
SHAPER_OPTIONS = ["--output-format=json", "--no-glyph-names"]

source = Path(sys.argv[1])
if hashlib.sha256(source.read_bytes()).hexdigest() != PROVENANCE["source_sha256"]:
    sys.exit("source does not match the pinned Noto CJK release")
license_file = FIXTURES / PROVENANCE["license"]
if hashlib.sha256(license_file.read_bytes()).hexdigest() != PROVENANCE["license_sha256"]:
    sys.exit("licence does not match the pinned Noto CJK release")
with TTFont(source, recalcTimestamp=False) as font:
    options = subset.Options()
    options.layout_features = FEATURES
    options.name_IDs = [0, 1, 2, 3, 4, 5, 6, 13, 14]
    subsetter = subset.Subsetter(options=options)
    subsetter.populate(text=TEXT)
    subsetter.subset(font)
    names = {1: "CJK Writing Test", 3: "CJKWritingTest-Regular",
             4: "CJK Writing Test", 6: "CJKWritingTest-Regular"}
    for record in font["name"].names:
        if record.nameID in names:
            record.string = names[record.nameID].encode(record.getEncoding())
    font.save(TARGET)
samples = {}
for direction in ["ltr", "ttb"]:
    samples[direction] = {
        ch: json.loads(subprocess.run(["hb-shape", str(TARGET), ch, *SHAPER_OPTIONS,
                                      f"--direction={direction}"], check=True,
                                     capture_output=True, text=True).stdout)
        for ch in TEXT
    }
samples["widths"] = {
    feature: json.loads(subprocess.run(["hb-shape", str(TARGET), "A", *SHAPER_OPTIONS,
                                        "--direction=ltr", f"--features={feature}"], check=True,
                                       capture_output=True, text=True).stdout)
    for feature in ["hwid", "twid", "qwid"]
}
record = {key: PROVENANCE[key] for key in ["repository", "commit", "source", "source_sha256",
                                          "license_source", "license", "license_sha256"]}
record.update({"generator": Path(__file__).name, "tool": "FontTools " + fontTools.__version__,
               "text": TEXT, "features": FEATURES, "bytes": TARGET.stat().st_size,
               "sha256": hashlib.sha256(TARGET.read_bytes()).hexdigest(),
               "shaper": subprocess.run(["hb-shape", "--version"], check=True,
                                        capture_output=True, text=True).stdout.splitlines()[0],
               "samples": samples})
REFERENCE.write_text(json.dumps(record, indent=2, ensure_ascii=False) + "\n")
