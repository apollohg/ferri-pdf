"""Derive an emoji presentation fixture from the bundled DejaVu Sans."""
import hashlib
import json
from pathlib import Path
import subprocess

import fontTools
from fontTools import subset
from fontTools.ttLib import TTFont
from fontTools.ttLib.tables._c_m_a_p import CmapSubtable

ROOT = Path(__file__).resolve().parents[3]
FIXTURES = Path(__file__).resolve().parent
SOURCE = ROOT / "assets/fonts/DejaVuSans.ttf"
TARGET = FIXTURES / "PresentationTest.ttf"
FAMILY = "Presentation Test"
TEXT_SELECTOR, EMOJI_SELECTOR = 0xFE0E, 0xFE0F
HEART, BEVERAGE = 0x2764, 0x2615
# Distinct glyphs expose which presentation selector, explicit or implied, reached the face.
SEQUENCES = {TEXT_SELECTOR: [(HEART, "A"), (BEVERAGE, "C")],
             EMOJI_SELECTOR: [(HEART, "B"), (BEVERAGE, "B")]}
SHAPER_OPTIONS = ["--output-format=json", "--no-glyph-names", "--remove-default-ignorables"]
IDENTITY_NAMES = {1: FAMILY, 2: "Book", 3: "PresentationTest-Book", 4: FAMILY,
                  6: "PresentationTest", 16: FAMILY, 17: "Book"}

with TTFont(SOURCE, recalcTimestamp=False) as font:
    options = subset.Options()
    options.retain_gids = True
    options.drop_tables += ["FFTM"]
    subsetter = subset.Subsetter(options=options)
    subsetter.populate(text="ABC ❤☕")
    subsetter.subset(font)
    table = CmapSubtable.newSubtable(14)
    table.platformID, table.platEncID, table.language = 0, 5, 0
    table.cmap, table.uvsDict = {}, SEQUENCES
    font["cmap"].tables.append(table)
    for record in font["name"].names:
        if record.nameID in IDENTITY_NAMES:
            record.string = IDENTITY_NAMES[record.nameID].encode(record.getEncoding())
    font.save(TARGET)

samples = []
with TTFont(TARGET) as font:
    order = font.getGlyphOrder()
    units_per_em = font["head"].unitsPerEm
for text in ["❤", "❤︎", "❤️", "☕", "☕︎", "☕️"]:
    shaped = json.loads(subprocess.run(
        ["hb-shape", str(TARGET), text, *SHAPER_OPTIONS],
        check=True, capture_output=True, text=True).stdout)
    assert len(shaped) == 1, (text, shaped)
    samples.append({"text": text, "glyph_id": shaped[0]["g"], "glyph": order[shaped[0]["g"]],
                    "advance": shaped[0]["ax"]})
record = {
    "derived_from": str(SOURCE.relative_to(ROOT)),
    "source_sha256": hashlib.sha256(SOURCE.read_bytes()).hexdigest(),
    "license": "../../../assets/fonts/FONT-LICENSE.txt",
    "generator": Path(__file__).name,
    "tool": "FontTools " + fontTools.__version__,
    "shaper": subprocess.run(["hb-shape", "--version"], check=True, capture_output=True, text=True).stdout.splitlines()[0],
    "shaper_options": SHAPER_OPTIONS,
    "changes": "Subset to ABC, space, U+2764 and U+2615 with original glyph IDs, outlines and metrics; renamed family; format-14 text and emoji presentation sequences select the A, B and C glyphs.",
    "sha256": hashlib.sha256(TARGET.read_bytes()).hexdigest(),
    "bytes": TARGET.stat().st_size,
    "units_per_em": units_per_em,
    "samples": samples,
}
(FIXTURES / "PresentationTest-source.json").write_text(json.dumps(record, indent=2) + "\n")
