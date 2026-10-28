"""Measure unmodified bundled fonts with fontTools, independently of the renderer."""
import hashlib
import json
from pathlib import Path

import fontTools
from fontTools.pens.boundsPen import BoundsPen
from fontTools.ttLib import TTFont

ROOT = Path(__file__).resolve().parents[3]
FAMILIES = {"Liberation Sans": "LiberationSans-Regular", "DejaVu Sans": "DejaVuSans", "DejaVu Serif": "DejaVuSerif", "DejaVu Sans Mono": "DejaVuSansMono"}
result = {"tool": "fontTools " + fontTools.__version__, "faces": {}}
for family, filename in FAMILIES.items():
    source = Path("crates/fonts/assets/fonts") / (filename + ".ttf")
    with TTFont(ROOT / source) as font:
        glyphs, cmap = font.getGlyphSet(), font.getBestCmap()
        def top(character):
            pen = BoundsPen(glyphs)
            glyphs[cmap[ord(character)]].draw(pen)
            return pen.bounds[3]
        os2 = font["OS/2"]
        typo = bool(os2.fsSelection & (1 << 7))
        result["faces"][family] = {
            "file": source.as_posix(), "sha256": hashlib.sha256((ROOT / source).read_bytes()).hexdigest(),
            "units_per_em": font["head"].unitsPerEm,
            "x_height": getattr(os2, "sxHeight", 0) or top("x"),
            "cap_height": getattr(os2, "sCapHeight", 0) or top("H"),
            "zero_advance": font["hmtx"][cmap[ord("0")]][0],
            "ascender": os2.sTypoAscender if typo else font["hhea"].ascent,
            "descender": os2.sTypoDescender if typo else font["hhea"].descent,
            "line_gap": os2.sTypoLineGap if typo else font["hhea"].lineGap,
        }
Path(__file__).with_name("font-preferences-reference.json").write_text(json.dumps(result, indent=2) + "\n")
