"""Subset the bundled math font and make its unused outer connectors Core-valid."""
import hashlib
import json
from pathlib import Path
import shutil
import fontTools
from fontTools import subset
from fontTools.ttLib import TTFont

ROOT = Path(__file__).resolve().parent
SOURCE = ROOT.parents[2] / "assets/fonts/DejaVuSerif.ttf"
TARGET = ROOT / "MathTest.ttf"
LICENSE = "MathTest-LICENSE.txt"
with TTFont(SOURCE, recalcTimestamp=False) as font:
    options = subset.Options()
    options.recalc_timestamp = False
    sub = subset.Subsetter(options=options)
    sub.populate(unicodes=set(range(0x20, 0x7f)) | set(range(0x370, 0x400)) |
                 set(range(0x2100, 0x2400)) | set(range(0x1d400, 0x1d800)))
    sub.subset(font)
    variants = font["MATH"].table.MathVariants
    overlap = variants.MinConnectorOverlap
    for construction in variants.VertGlyphConstruction + variants.HorizGlyphConstruction:
        if construction.GlyphAssembly:
            for part in construction.GlyphAssembly.PartRecords:
                part.StartConnectorLength = max(part.StartConnectorLength, overlap)
                part.EndConnectorLength = max(part.EndConnectorLength, overlap)
    names = {1: "Math Test", 2: "Regular", 3: "MathTest-Regular", 4: "Math Test Regular",
             6: "MathTest-Regular", 16: "Math Test", 17: "Regular"}
    for record in font["name"].names:
        if record.nameID in names:
            record.string = names[record.nameID].encode(record.getEncoding())
    font.save(TARGET)
shutil.copyfile(SOURCE.parent / "FONT-LICENSE.txt", ROOT / LICENSE)
(ROOT / "MathTest-source.json").write_text(json.dumps({
    "derived_from": "assets/fonts/DejaVuSerif.ttf",
    "source_sha256": hashlib.sha256(SOURCE.read_bytes()).hexdigest(),
    "license": LICENSE,
    "tool": "fontTools " + fontTools.__version__,
    "generator": "make-math-font.py",
    "changes": "Subset Unicode coverage; rename family; raise unused outer connector lengths to MinConnectorOverlap to meet pinned MathML Core 5.3.1 validity. Original outlines, MATH constants, part advances and usable connectors are retained.",
    "sha256": hashlib.sha256(TARGET.read_bytes()).hexdigest(),
    "bytes": TARGET.stat().st_size,
}, indent=2) + "\n")
