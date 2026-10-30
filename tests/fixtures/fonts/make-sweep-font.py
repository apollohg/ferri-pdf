"""Derive a COLRv1 sweep gradient fixture from the bundled Liberation Sans."""
import hashlib
import json
from pathlib import Path

import fontTools
from fontTools import subset
from fontTools.colorLib.builder import buildCOLR, buildCPAL
from fontTools.pens.ttGlyphPen import TTGlyphPen
from fontTools.ttLib import TTFont
from fontTools.ttLib.tables import otTables

ROOT = Path(__file__).resolve().parents[3]
FIXTURES = Path(__file__).resolve().parent
SOURCE = ROOT / "assets/fonts/LiberationSans-Regular.ttf"
TARGET = FIXTURES / "SweepTest.ttf"
FAMILY = "Sweep Test"
# A square from the baseline, one em wide and tall, whose centre the sweeps turn about.
EM = 2048
CENTER = EM // 2
CHANNEL_MAX = 255
PALETTE = [(255, 0, 0, 255), (0, 0, 255, 255)]
# Each glyph sweeps from palette entry 0 to entry 1, padded, over its angles in degrees,
# counter-clockwise from the positive x axis; the second runs its colour line backwards.
SWEEPS = {"forward": ("■", 0, 90), "backward": ("□", 90, 0)}
IDENTITY_NAMES = {1: FAMILY, 2: "Regular", 3: "SweepTest-Regular", 4: FAMILY + " Regular",
                  6: "SweepTest-Regular", 16: FAMILY, 17: "Regular"}


def square():
    pen = TTGlyphPen(None)
    pen.moveTo((0, 0))
    pen.lineTo((0, EM))
    pen.lineTo((EM, EM))
    pen.lineTo((EM, 0))
    pen.closePath()
    return pen.glyph()


def sweep(glyph, start, end):
    return {
        "Format": otTables.PaintFormat.PaintGlyph,
        "Glyph": glyph,
        "Paint": {
            "Format": otTables.PaintFormat.PaintSweepGradient,
            "ColorLine": {"Extend": "pad", "ColorStop": [
                {"StopOffset": 0.0, "PaletteIndex": 0, "Alpha": 1.0},
                {"StopOffset": 1.0, "PaletteIndex": 1, "Alpha": 1.0},
            ]},
            "centerX": CENTER, "centerY": CENTER, "startAngle": start, "endAngle": end,
        },
    }


with TTFont(SOURCE, recalcTimestamp=False) as font:
    options = subset.Options()
    options.drop_tables += ["FFTM"]
    subsetter = subset.Subsetter(options=options)
    subsetter.populate(text=" ")
    subsetter.subset(font)
    order = font.getGlyphOrder() + list(SWEEPS)
    font.setGlyphOrder(order)
    for name, (character, _, _) in SWEEPS.items():
        font["glyf"][name] = square()
        font["hmtx"][name] = (EM, 0)
        for table in font["cmap"].tables:
            if table.isUnicode():
                table.cmap[ord(character)] = name
    font["maxp"].numGlyphs = len(order)
    font["COLR"] = buildCOLR({name: sweep(name, start, end)
                              for name, (_, start, end) in SWEEPS.items()}, version=1)
    font["CPAL"] = buildCPAL([[tuple(channel / CHANNEL_MAX for channel in color)
                               for color in PALETTE]])
    for record in font["name"].names:
        if record.nameID in IDENTITY_NAMES:
            record.string = IDENTITY_NAMES[record.nameID].encode(record.getEncoding())
    font.save(TARGET)

with TTFont(TARGET) as font:
    paints = {record.BaseGlyph: record.Paint.Paint for record in
              font["COLR"].table.BaseGlyphList.BaseGlyphPaintRecord}
    cmap = font.getBestCmap()
    angles = {character: [paints[cmap[ord(character)]].startAngle,
                          paints[cmap[ord(character)]].endAngle]
              for character, _, _ in SWEEPS.values()}
    assert angles == {character: [start, end] for character, start, end in SWEEPS.values()}, angles
    record = {
        "derived_from": str(SOURCE.relative_to(ROOT)),
        "source_sha256": hashlib.sha256(SOURCE.read_bytes()).hexdigest(),
        "license": "../../../assets/fonts/LIBERATION-LICENSE.txt",
        "generator": Path(__file__).name,
        "tool": "FontTools " + fontTools.__version__,
        "changes": "Subset to space, renamed family; adds one-em squares for U+25A0 and U+25A1 that COLRv1 fills with padded sweep gradients from palette entry 0 to entry 1 about the square's centre, over 0 to 90 and 90 to 0 degrees.",
        "sha256": hashlib.sha256(TARGET.read_bytes()).hexdigest(),
        "bytes": TARGET.stat().st_size,
        "units_per_em": font["head"].unitsPerEm,
        "center": [CENTER, CENTER],
        "palette": [list(color) for color in PALETTE],
        "sweeps": {character: [start, end] for character, start, end in SWEEPS.values()},
    }
(FIXTURES / "SweepTest-source.json").write_text(json.dumps(record, indent=2) + "\n")
