"""Derive a variation-sequence fixture from the existing bundled OFL font."""
import hashlib
import json
from pathlib import Path
import subprocess

import fontTools
from fontTools import subset
from fontTools.feaLib.builder import addOpenTypeFeaturesFromString
from fontTools.pens.recordingPen import DecomposingRecordingPen
from fontTools.ttLib import TTFont
from fontTools.ttLib.tables._c_m_a_p import CmapSubtable

ROOT = Path(__file__).resolve().parents[3]
FIXTURES = Path(__file__).resolve().parent
SOURCE = ROOT / "crates/fonts/assets/fonts/LiberationSerif-Regular.ttf"
TARGET = FIXTURES / "VariationSequenceTest.ttf"
FAMILY = "Variation Sequence Test"
SHAPER_OPTIONS = ["--output-format=json", "--no-glyph-names", "--remove-default-ignorables"]
SEQUENCES = {0xFE00: [(ord("A"), "B"), (ord("H"), None)],
             0xE0100: [(ord("A"), "C")]}
IDENTITY_NAMES = {1: FAMILY, 2: "Regular", 3: "VariationSequenceTest-Regular",
                  4: FAMILY + " Regular", 6: "VariationSequenceTest-Regular",
                  16: FAMILY, 17: "Regular"}

with TTFont(SOURCE, recalcTimestamp=False) as font:
    options = subset.Options()
    options.retain_gids = True
    options.drop_tables += ["FFTM"]
    subsetter = subset.Subsetter(options=options)
    subsetter.populate(text="ABCH ")
    subsetter.subset(font)
    for cmap in font["cmap"].tables:
        if cmap.isUnicode():
            cmap.cmap.update({0x1820: "A", 0x180B: "space"})
    addOpenTypeFeaturesFromString(font, """
        languagesystem mong dflt;
        feature ccmp { sub A space by B; } ccmp;
    """)
    table = CmapSubtable.newSubtable(14)
    table.platformID, table.platEncID, table.language = 0, 5, 0
    table.cmap, table.uvsDict = {}, SEQUENCES
    font["cmap"].tables.append(table)
    for record in font["name"].names:
        if record.nameID in IDENTITY_NAMES:
            record.string = IDENTITY_NAMES[record.nameID].encode(record.getEncoding())
    font.save(TARGET)

samples = []
for text in ["A\ufe00", "A\U000e0100", "H\ufe00", "A\ufe01", "A\ufe00\ufe01", "\u1820\u180b"]:
    shaped = json.loads(subprocess.run(
        ["hb-shape", str(TARGET), text, *SHAPER_OPTIONS],
        check=True, capture_output=True, text=True).stdout)
    assert len(shaped) == 1, (text, shaped)
    glyph = shaped[0]
    with TTFont(TARGET) as font:
        units_per_em = font["head"].unitsPerEm
        glyphs = font.getGlyphSet()
        pen = DecomposingRecordingPen(glyphs)
        glyphs[font.getGlyphOrder()[glyph["g"]]].draw(pen)
        samples.append({"text": text, "glyph_id": glyph["g"], "advance": glyph["ax"],
                        "outline": pen.value})
record = {
    "derived_from": str(SOURCE.relative_to(ROOT)),
    "source_sha256": hashlib.sha256(SOURCE.read_bytes()).hexdigest(),
    "license": "../../../crates/fonts/assets/fonts/LIBERATION-LICENSE.txt",
    "generator": Path(__file__).name,
    "tool": "FontTools " + fontTools.__version__,
    "shaper": subprocess.run(["hb-shape", "--version"], check=True, capture_output=True, text=True).stdout.splitlines()[0],
    "shaper_options": SHAPER_OPTIONS,
    "changes": "Subset to ABCH and space, retain original glyph IDs/outlines/metrics, rename family, add format-14 default and non-default BMP/supplementary variation sequences and a Mongolian GSUB selector control.",
    "sha256": hashlib.sha256(TARGET.read_bytes()).hexdigest(),
    "bytes": TARGET.stat().st_size,
    "units_per_em": units_per_em,
    "samples": samples,
}
(FIXTURES / "VariationSequenceTest-source.json").write_text(json.dumps(record, indent=2) + "\n")
