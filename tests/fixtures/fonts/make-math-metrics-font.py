"""Subset an immutable STIX release without changing its MATH glyph metrics."""
import hashlib
import io
import json
from pathlib import Path
from urllib.request import urlopen
import fontTools
from fontTools import subset
from fontTools.ttLib import TTFont

ROOT = Path(__file__).resolve().parent
RELEASE = "https://raw.githubusercontent.com/stipub/stixfonts/v2.13b171/"
SOURCE = RELEASE + "fonts/static_otf/STIXTwoMath-Regular.otf"
SOURCE_SHA256 = "3a5f3f26f40d5698b3c62dd085d48d6663696a3f80825aab8b553d5097518e8c"
TARGET = ROOT / "MathMetrics.otf"
LICENSE = "MathMetrics-LICENSE.txt"
with urlopen(SOURCE) as response:
    source = response.read()
assert hashlib.sha256(source).hexdigest() == SOURCE_SHA256
with TTFont(io.BytesIO(source), recalcTimestamp=False) as font:
    options = subset.Options()
    options.recalc_timestamp = False
    sub = subset.Subsetter(options=options)
    sub.populate(unicodes=set(range(0x20, 0x7f)) | {0x1d453, 0x1d465, 0x222b})
    sub.subset(font)
    names = {1: "Math Metrics", 2: "Regular", 3: "MathMetrics-Regular",
             4: "Math Metrics Regular", 6: "MathMetrics-Regular",
             16: "Math Metrics", 17: "Regular"}
    for record in font["name"].names:
        if record.nameID in names:
            record.string = names[record.nameID].encode(record.getEncoding())
    cff = font["CFF "].cff
    cff.fontNames = ["MathMetrics-Regular"]
    cff.topDictIndex[0].FamilyName = "Math Metrics"
    cff.topDictIndex[0].FullName = "Math Metrics Regular"
    font.save(TARGET)
with urlopen(RELEASE + "OFL.txt") as response:
    (ROOT / LICENSE).write_bytes(response.read())
(ROOT / "MathMetrics-source.json").write_text(json.dumps({
    "derived_from": SOURCE,
    "source_sha256": SOURCE_SHA256,
    "license": LICENSE,
    "tool": "fontTools " + fontTools.__version__,
    "generator": Path(__file__).name,
    "changes": "Subset Unicode coverage and rename family. Original outlines and MATH metrics are retained.",
    "sha256": hashlib.sha256(TARGET.read_bytes()).hexdigest(),
    "bytes": TARGET.stat().st_size,
}, indent=2) + "\n")
