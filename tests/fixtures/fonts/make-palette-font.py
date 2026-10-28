"""Derive a COLR/CPAL palette fixture from the bundled Liberation Sans."""
import hashlib
import json
from pathlib import Path

import fontTools
from fontTools import subset
from fontTools.colorLib.builder import buildCOLR, buildCPAL
from fontTools.ttLib import TTFont

ROOT = Path(__file__).resolve().parents[3]
FIXTURES = Path(__file__).resolve().parent
SOURCE = ROOT / "assets/fonts/LiberationSans-Regular.ttf"
TARGET = FIXTURES / "PaletteTest.ttf"
FAMILY = "Palette Test"
FOREGROUND = 0xFFFF
# Each colour glyph is its own outline in one palette entry, so painted colours identify entries.
LAYERS = {"A": [("A", 0)], "H": [("H", 1)], "I": [("I", FOREGROUND)]}
CHANNEL_MAX = 255
# RGBA bytes. Palette 1 is marked for light and palette 2 for dark backgrounds.
PALETTES = [
    [(200, 0, 0, 255), (0, 120, 0, 255)],
    [(0, 0, 200, 255), (200, 120, 0, 255)],
    [(220, 220, 0, 255), (0, 200, 200, 255)],
]
LIGHT_BACKGROUND, DARK_BACKGROUND = 1, 2
PALETTE_TYPES = [0, LIGHT_BACKGROUND, DARK_BACKGROUND]
IDENTITY_NAMES = {1: FAMILY, 2: "Regular", 3: "PaletteTest-Regular", 4: FAMILY + " Regular",
                  6: "PaletteTest-Regular", 16: FAMILY, 17: "Regular"}

with TTFont(SOURCE, recalcTimestamp=False) as font:
    options = subset.Options()
    options.drop_tables += ["FFTM"]
    subsetter = subset.Subsetter(options=options)
    subsetter.populate(text="AHI ")
    subsetter.subset(font)
    font["COLR"] = buildCOLR(LAYERS, version=0)
    font["CPAL"] = buildCPAL(
        [[tuple(channel / CHANNEL_MAX for channel in color) for color in palette] for palette in PALETTES],
        paletteTypes=PALETTE_TYPES,
    )
    for record in font["name"].names:
        if record.nameID in IDENTITY_NAMES:
            record.string = IDENTITY_NAMES[record.nameID].encode(record.getEncoding())
    font.save(TARGET)

with TTFont(TARGET) as font:
    cpal = font["CPAL"]
    palettes = [[[color.red, color.green, color.blue, color.alpha] for color in palette]
                for palette in cpal.palettes]
    assert palettes == [[list(color) for color in palette] for palette in PALETTES], palettes
    record = {
        "derived_from": str(SOURCE.relative_to(ROOT)),
        "source_sha256": hashlib.sha256(SOURCE.read_bytes()).hexdigest(),
        "license": "../../../assets/fonts/LIBERATION-LICENSE.txt",
        "generator": Path(__file__).name,
        "tool": "FontTools " + fontTools.__version__,
        "changes": "Subset to AHI and space, renamed family; COLRv0 paints A with entry 0, H with entry 1 and I with the foreground colour; CPAL version 1 has three palettes typed default, light and dark.",
        "sha256": hashlib.sha256(TARGET.read_bytes()).hexdigest(),
        "bytes": TARGET.stat().st_size,
        "units_per_em": font["head"].unitsPerEm,
        "palette_types": list(cpal.paletteTypes),
        "palettes": palettes,
        "layers": {glyph: [[layer, index] for layer, index in layers] for glyph, layers in LAYERS.items()},
    }
(FIXTURES / "PaletteTest-source.json").write_text(json.dumps(record, indent=2) + "\n")
