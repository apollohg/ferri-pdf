"""Measure Roboto Flex optical-size instances independently of the renderer."""
import hashlib
import json
from pathlib import Path

import fontTools
from fontTools.pens.boundsPen import BoundsPen
from fontTools.ttLib import TTFont

FIXTURES = Path(__file__).resolve().parent
SOURCE = FIXTURES / "RobotoFlex.ttf"
CHARACTER = "H"
# Optical sizes the CSS tests request: font sizes in points, clamped to the axis, and settings
# that replace the optical size or another axis after it.
LOCATIONS = [
    {"opsz": 8},
    {"opsz": 14},
    {"opsz": 20},
    {"opsz": 40},
    {"opsz": 144},
    {"opsz": 20, "wght": 700},
]

with TTFont(SOURCE) as font:
    glyph = font.getBestCmap()[ord(CHARACTER)]
    axes = {axis.axisTag: [axis.minValue, axis.defaultValue, axis.maxValue]
            for axis in font["fvar"].axes}
    samples = []
    for location in LOCATIONS:
        glyphs = font.getGlyphSet(location=location)
        pen = BoundsPen(glyphs)
        glyphs[glyph].draw(pen)
        samples.append({"location": location, "advance": glyphs[glyph].width,
                        "bounds": list(pen.bounds)})
    record = {
        "source": SOURCE.name,
        "sha256": hashlib.sha256(SOURCE.read_bytes()).hexdigest(),
        "generator": Path(__file__).name,
        "tool": "fontTools " + fontTools.__version__,
        "method": "TTFont.getGlyphSet(location) advance and BoundsPen bounds of the cmap glyph for H; unspecified axes retain file defaults",
        "units_per_em": font["head"].unitsPerEm,
        "optical_axis": axes["opsz"],
        "samples": samples,
    }
(FIXTURES / "RobotoFlex-optical.json").write_text(json.dumps(record, indent=2) + "\n")
