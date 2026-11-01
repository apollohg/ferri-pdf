"""Make a renamed four-face subset family of the bundled Liberation Sans, as separate fonts and
as a collection, for installed-font tests."""
import hashlib
import json
from pathlib import Path

import fontTools
from fontTools import subset
from fontTools.ttLib import TTCollection, TTFont

ROOT = Path(__file__).resolve().parents[3]
FIXTURES = Path(__file__).resolve().parent
FAMILY = "System Test"
POSTSCRIPT_FAMILY = "SystemTest"
FACES = {
    "Regular": ROOT / "assets/fonts/LiberationSans-Regular.ttf",
    "Bold": ROOT / "assets/fonts/LiberationSans-Bold.ttf",
    "Italic": ROOT / "assets/fonts/LiberationSans-Italic.ttf",
    "BoldItalic": ROOT / "assets/fonts/LiberationSans-BoldItalic.ttf",
}
STYLE_NAMES = {"Regular": "Regular", "Bold": "Bold", "Italic": "Italic", "BoldItalic": "Bold Italic"}
TEXT = "".join(chr(code) for code in range(0x20, 0x7F))
# The collection lists the faces in reverse, so every face but the bold italic one, the regular
# face included, sits at a non-zero index.
COLLECTION_ORDER = ["BoldItalic", "Italic", "Bold", "Regular"]
COLLECTION = FIXTURES / "SystemTest.ttc"
NAME_IDS = {"family": 1, "subfamily": 2, "unique": 3, "full": 4, "postscript": 6,
            "typographic_family": 16, "typographic_subfamily": 17}


def face(style, source):
    with TTFont(source, recalcTimestamp=False) as font:
        options = subset.Options()
        options.drop_tables += ["FFTM"]
        options.hinting = False
        subsetter = subset.Subsetter(options=options)
        subsetter.populate(text=TEXT)
        subsetter.subset(font)
        postscript = f"{POSTSCRIPT_FAMILY}-{style}"
        style_name = STYLE_NAMES[style]
        names = {NAME_IDS["family"]: FAMILY, NAME_IDS["subfamily"]: style_name,
                 NAME_IDS["unique"]: postscript, NAME_IDS["full"]: f"{FAMILY} {style_name}",
                 NAME_IDS["postscript"]: postscript}
        table = font["name"]
        table.names = [record for record in table.names
                       if record.nameID not in (NAME_IDS["typographic_family"],
                                                NAME_IDS["typographic_subfamily"])]
        for record in table.names:
            if record.nameID in names:
                record.string = names[record.nameID].encode(record.getEncoding())
        target = FIXTURES / f"{postscript}.ttf"
        font.save(target)
    return target


faces = {style: face(style, source) for style, source in FACES.items()}
collection = TTCollection()
collection.fonts = [TTFont(faces[style], recalcTimestamp=False) for style in COLLECTION_ORDER]
collection.save(COLLECTION, shareTables=True)

# Independent read-back: each face keeps its renamed names and its own advances.
for index, style in enumerate(COLLECTION_ORDER):
    single, member = TTFont(faces[style]), TTFont(COLLECTION, fontNumber=index)
    assert member["name"].getDebugName(NAME_IDS["postscript"]) == f"{POSTSCRIPT_FAMILY}-{style}"
    assert member["hmtx"].metrics == single["hmtx"].metrics, style
assert TTFont(faces["Bold"])["OS/2"].usWeightClass == 700
assert TTFont(faces["Italic"])["post"].italicAngle < 0
record = {
    "derived_from": {style: str(path.relative_to(ROOT)) for style, path in FACES.items()},
    "source_sha256": {style: hashlib.sha256(path.read_bytes()).hexdigest()
                      for style, path in FACES.items()},
    "license": "../../../assets/fonts/LIBERATION-LICENSE.txt",
    "generator": Path(__file__).name,
    "tool": f"FontTools {fontTools.__version__}",
    "changes": "Subset to U+0020-U+007E without hinting and renamed to the System Test family "
               "with PostScript names SystemTest-STYLE; typographic family names removed. The "
               "collection holds the same four faces in the order "
               f"{', '.join(COLLECTION_ORDER)}, sharing identical tables.",
    "files": {path.name: {"sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
                          "bytes": path.stat().st_size}
              for path in [*faces.values(), COLLECTION]},
}
(FIXTURES / "SystemTest-source.json").write_text(json.dumps(record, indent=2) + "\n")
