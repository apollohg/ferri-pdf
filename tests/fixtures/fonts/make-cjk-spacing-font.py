"""Derive the CJK spacing fixtures from the pinned upstream Noto Sans JP release.

Usage: python3 tests/fixtures/fonts/make-cjk-spacing-font.py /path/to/NotoSansJP-Regular.otf
"""
import hashlib
import json
from pathlib import Path
import subprocess
import sys

import fontTools
from fontTools import subset
from fontTools.ttLib import TTFont

FIXTURES = Path(__file__).resolve().parent
COMMIT = "523d033d6cb47f4a80c58a35753646f5c3608a78"
REPOSITORY = "https://github.com/notofonts/noto-cjk"
SOURCE_URL = f"https://raw.githubusercontent.com/notofonts/noto-cjk/{COMMIT}/Sans/SubsetOTF/JP/NotoSansJP-Regular.otf"
LICENSE_URL = f"https://raw.githubusercontent.com/notofonts/noto-cjk/{COMMIT}/LICENSE"
SOURCE_SHA256 = "dff723ba59d57d136764a04b9b2d03205544f7cd785a711442d6d2d085ac5073"
LICENSE = FIXTURES / "CJKSpacingTest-OFL.txt"
TEXT = ("漢字水卜一あア" "「」（）〔〕、。・：；，．　‘’“”"
        "abcxyzAB0123456789 ()１Ａ♥;:!?«» ")
# The trimming feature is retained in one face and removed from the other, whose punctuation
# must then be trimmed from its outlines.
FACES = {
    "CJKSpacingTest.otf": ("CJK Spacing Test", "CJKSpacingTest-Regular", ["halt"]),
    "CJKSpacingTest-NoHalt.otf": ("CJK Spacing Test NoHalt", "CJKSpacingTestNoHalt-Regular", []),
}
SHAPER_OPTIONS = ["--output-format=json", "--no-glyph-names"]


def shape(font, text, features):
    shaped = subprocess.run(["hb-shape", str(font), text, *SHAPER_OPTIONS, "--features=" + features],
                            check=True, capture_output=True, text=True).stdout
    return [{"advance": glyph["ax"], "offset": glyph["dx"]} for glyph in json.loads(shaped)]


source = Path(sys.argv[1])
if hashlib.sha256(source.read_bytes()).hexdigest() != SOURCE_SHA256:
    sys.exit(f"{source} does not match the pinned upstream release")
faces = {}
for name, (family, postscript, features) in FACES.items():
    target = FIXTURES / name
    with TTFont(source, recalcTimestamp=False) as font:
        options = subset.Options()
        options.layout_features = features
        options.name_IDs = [0, 1, 2, 3, 4, 5, 6, 13, 14]
        subsetter = subset.Subsetter(options=options)
        subsetter.populate(text=TEXT)
        subsetter.subset(font)
        identities = {1: family, 3: postscript, 4: family, 6: postscript}
        for record in font["name"].names:
            if record.nameID in identities:
                record.string = identities[record.nameID].encode(record.getEncoding())
        font.save(target)
    with TTFont(target) as font:
        units_per_em = font["head"].unitsPerEm
        tags = sorted({record.FeatureTag for record in font["GPOS"].table.FeatureList.FeatureRecord}) \
            if "GPOS" in font else []
    faces[name] = {
        "family": family,
        "sha256": hashlib.sha256(target.read_bytes()).hexdigest(),
        "bytes": target.stat().st_size,
        "units_per_em": units_per_em,
        "gpos_features": tags,
        "samples": {ch: {"default": shape(target, ch, ""), "halt": shape(target, ch, "halt")}
                    for ch in TEXT},
    }
record = {
    "repository": REPOSITORY,
    "commit": COMMIT,
    "source": SOURCE_URL,
    "source_sha256": SOURCE_SHA256,
    "license_source": LICENSE_URL,
    "license": LICENSE.name,
    "license_sha256": hashlib.sha256(LICENSE.read_bytes()).hexdigest(),
    "generator": Path(__file__).name,
    "tool": "FontTools " + fontTools.__version__,
    "shaper": subprocess.run(["hb-shape", "--version"], check=True, capture_output=True,
                             text=True).stdout.splitlines()[0],
    "shaper_options": SHAPER_OPTIONS,
    "text": TEXT,
    "changes": "Subset to the recorded text with original outlines and metrics; renamed family; "
               "GPOS keeps only halt in CJKSpacingTest.otf and is removed from CJKSpacingTest-NoHalt.otf.",
    "faces": faces,
}
(FIXTURES / "CJKSpacingTest-source.json").write_text(json.dumps(record, indent=2, ensure_ascii=False) + "\n")
