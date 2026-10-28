"""Derive the metric fallback fixture from the pinned, OFL-licensed style font."""
import hashlib
import json
from pathlib import Path

import fontTools
from fontTools.ttLib import TTFont
from fontTools.ttLib.tables._c_m_a_p import CmapSubtable

ROOT = Path(__file__).resolve().parent
SOURCE = ROOT / "FontStyleTest-Ital.ttf"
TARGET = ROOT / "MetricGlyphTest.ttf"
REMOVED_CHARACTERS = "xH"
NOTDEF_CHARACTER = "0"
IDENTITY_NAMES = {
    1: "Metric Glyph Test",
    2: "Regular",
    3: "MetricGlyphTest-Regular",
    4: "Metric Glyph Test Regular",
    6: "MetricGlyphTest-Regular",
    16: "Metric Glyph Test",
    17: "Regular",
    25: "MetricGlyphTest",
}

with TTFont(SOURCE, recalcTimestamp=False) as font:
    for table in font["cmap"].tables:
        if table.isUnicode():
            for character in REMOVED_CHARACTERS:
                table.cmap.pop(ord(character), None)
            table.cmap[ord(NOTDEF_CHARACTER)] = ".notdef"
    extended = CmapSubtable.newSubtable(12)
    extended.platformID = 0
    extended.platEncID = 4
    extended.language = 0
    extended.cmap = next(table.cmap for table in font["cmap"].tables if table.isUnicode()).copy()
    font["cmap"].tables.append(extended)
    font["OS/2"].sxHeight = 0
    font["OS/2"].sCapHeight = 0
    for record in font["name"].names:
        if record.nameID in IDENTITY_NAMES:
            record.string = IDENTITY_NAMES[record.nameID].encode(record.getEncoding())
    font.save(TARGET)

with TTFont(TARGET) as font:
    assert all(ord(character) not in font.getBestCmap() for character in REMOVED_CHARACTERS)
    metrics = {
        "units_per_em": font["head"].unitsPerEm,
        "ascender": font["hhea"].ascent,
        "descender": font["hhea"].descent,
        "line_gap": font["hhea"].lineGap,
        "x_height": font["OS/2"].sxHeight,
        "cap_height": font["OS/2"].sCapHeight,
    }
record = {
    "derived_from": SOURCE.name,
    "source_sha256": hashlib.sha256(SOURCE.read_bytes()).hexdigest(),
    "upstream_record": "FontStyleTest-source.json",
    "license": "FontStyleTest-OFL.txt",
    "tool": "fontTools " + fontTools.__version__,
    "generator": Path(__file__).name,
    "removed_cmap_characters": REMOVED_CHARACTERS,
    "notdef_character": NOTDEF_CHARACTER,
    "changes": "Zero unspecified x/cap metrics; explicit format-12 .notdef mapping for 0; renamed family identities; all original outlines, axes and other metrics retained.",
    "sha256": hashlib.sha256(TARGET.read_bytes()).hexdigest(),
    "bytes": TARGET.stat().st_size,
    "metrics": metrics,
}
(ROOT / "MetricGlyphTest-source.json").write_text(json.dumps(record, indent=2) + "\n")
