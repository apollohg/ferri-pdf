"""Derive an OpenType SVG glyph fixture, whose glyph images name a file and a data URL."""
import base64
import hashlib
import json
from pathlib import Path

import fontTools
from fontTools import subset
from fontTools.ttLib import TTFont, newTable
from fontTools.ttLib.tables.S_V_G_ import SVGDocument

ROOT = Path(__file__).resolve().parents[3]
FIXTURES = Path(__file__).resolve().parent
SOURCE = ROOT / "assets/fonts/LiberationSans-Regular.ttf"
TARGET = FIXTURES / "SvgGlyphTest.ttf"
FAMILY = "Svg Glyph Test"
# Relative to the package root, the working directory of `cargo test`.
IMAGE = Path("tests/oracle/assets/image-pattern.png")
DATA_URL = "data:image/png;base64," + base64.b64encode((ROOT / IMAGE).read_bytes()).decode()
# Each glyph's document draws one em-square image above the baseline.
IMAGES = {"F": IMAGE.as_posix(), "D": DATA_URL}
DOCUMENT = ('<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink">'
            '<image id="glyph{glyph}" xlink:href="{href}" x="0" y="-{em}" width="{em}" height="{em}"/></svg>')
IDENTITY_NAMES = {1: FAMILY, 2: "Regular", 3: "SvgGlyphTest-Regular", 4: FAMILY + " Regular",
                  6: "SvgGlyphTest-Regular", 16: FAMILY, 17: "Regular"}

with TTFont(SOURCE, recalcTimestamp=False) as font:
    options = subset.Options()
    options.drop_tables += ["FFTM"]
    subsetter = subset.Subsetter(options=options)
    subsetter.populate(text="".join(IMAGES) + " ")
    subsetter.subset(font)
    em = font["head"].unitsPerEm
    cmap = font.getBestCmap()
    table = newTable("SVG ")
    # Document records are sorted by glyph ID.
    table.docList = [
        SVGDocument(DOCUMENT.format(glyph=glyph, href=href, em=em), glyph, glyph, False)
        for glyph, href in sorted((font.getGlyphID(cmap[ord(char)]), href) for char, href in IMAGES.items())
    ]
    font["SVG "] = table
    for record in font["name"].names:
        if record.nameID in IDENTITY_NAMES:
            record.string = IDENTITY_NAMES[record.nameID].encode(record.getEncoding())
    font.save(TARGET)

with TTFont(TARGET) as font:
    cmap = font.getBestCmap()
    documents = {document.startGlyphID: document.data for document in font["SVG "].docList}
    glyphs = {}
    for char, href in IMAGES.items():
        glyphs[char] = font.getGlyphID(cmap[ord(char)])
        assert f'xlink:href="{href}"' in documents[glyphs[char]], (char, documents[glyphs[char]])
    record = {
        "derived_from": str(SOURCE.relative_to(ROOT)),
        "source_sha256": hashlib.sha256(SOURCE.read_bytes()).hexdigest(),
        "license": "../../../assets/fonts/LIBERATION-LICENSE.txt",
        "generator": Path(__file__).name,
        "tool": "FontTools " + fontTools.__version__,
        "changes": "Subset to F, D and space, renamed family; the SVG table draws F as an image file named relative to the package root and D as the same image in a data URL.",
        "image": IMAGE.as_posix(),
        "image_sha256": hashlib.sha256((ROOT / IMAGE).read_bytes()).hexdigest(),
        "sha256": hashlib.sha256(TARGET.read_bytes()).hexdigest(),
        "bytes": TARGET.stat().st_size,
        "units_per_em": font["head"].unitsPerEm,
        "glyphs": glyphs,
    }
(FIXTURES / "SvgGlyphTest-source.json").write_text(json.dumps(record, indent=2) + "\n")
