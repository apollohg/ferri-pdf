"""Package renamed subsets of the bundled Liberation Sans as WOFF, WOFF2 and collections."""
import hashlib
import io
import json
import struct
from pathlib import Path

import brotli
import fontTools
from fontTools import subset
from fontTools.ttLib import TTCollection, TTFont, woff2
from fontTools.ttLib.sfnt import WOFFFlavorData

ROOT = Path(__file__).resolve().parents[3]
FIXTURES = Path(__file__).resolve().parent
FAMILY = "WOFF Test"
FACES = {
    "Regular": ROOT / "assets/fonts/LiberationSans-Regular.ttf",
    "Bold": ROOT / "assets/fonts/LiberationSans-Bold.ttf",
}
TEXT = "".join(chr(code) for code in range(0x20, 0x7F)) + "Éé"
# One simple glyph carries the OVERLAP_SIMPLE flag, which WOFF2 moves to a bitmap (§5.1).
OVERLAP_GLYPH = "O"
OVERLAP_SIMPLE = 0x40
METADATA = (b'<?xml version="1.0" encoding="UTF-8"?>\n<metadata version="1.0">'
            b'<uniqueid id="ferri-pdf.woff-test"/></metadata>\n')
PRIVATE = b"ferri-pdf WOFF fixture private data"
TRANSFORMED = {"glyf", "loca", "hmtx"}
COLLECTION_VERSION = 0x00010000
WOFF2_HEADER = ">4sLLHHLLHHLLLLL"
KNOWN_TAG_MASK = 0x3F
TRANSFORM_SHIFT = 6


def face(style, source):
    with TTFont(source, recalcTimestamp=False) as font:
        options = subset.Options()
        options.drop_tables += ["FFTM"]
        subsetter = subset.Subsetter(options=options)
        subsetter.populate(text=TEXT)
        subsetter.subset(font)
        postscript = f"WoffTest-{style}"
        names = {1: FAMILY, 2: style, 3: postscript, 4: f"{FAMILY} {style}", 6: postscript,
                 16: FAMILY, 17: style}
        for record in font["name"].names:
            if record.nameID in names:
                record.string = names[record.nameID].encode(record.getEncoding())
        if style == "Regular":
            glyph = font["glyf"][OVERLAP_GLYPH]
            glyph.expand(font["glyf"])
            glyph.flags[0] |= OVERLAP_SIMPLE
        target = FIXTURES / f"{postscript}.ttf"
        font.save(target)
    return target


def packaged(source, target, flavor, flavor_data):
    with TTFont(source, recalcTimestamp=False) as font:
        font.flavor = flavor
        font.flavorData = flavor_data
        font.save(target)
    return target


def woff2_entries(path):
    """The directory entries of a single-font WOFF2 file with their stored stream bytes."""
    reader = woff2.WOFF2Reader(path.open("rb"))
    return reader.sfntVersion, [(tag, entry, entry.loadData(reader.transformBuffer))
                                for tag, entry in reader.tables.items()]


def directory_entry(tag, entry):
    index = woff2.getKnownTagIndex(tag)
    data = bytes([index | entry.transformVersion << TRANSFORM_SHIFT])
    if index == KNOWN_TAG_MASK:
        data += tag.encode("latin-1")
    data += woff2.packBase128(entry.origLength)
    if entry.transformed:
        data += woff2.packBase128(entry.length)
    return data


def collection(fonts, target):
    """A WOFF2 collection (§4.2) of single-font WOFF2 files, sharing identical tables, each
    loca immediately after its glyf."""
    entries, stream, faces = [], b"", []
    for flavor, tables in fonts:
        tables.sort(key=lambda table: (table[0] == "loca", table[0]))
        glyf = next(index for index, table in enumerate(tables) if table[0] == "glyf")
        tables.insert(glyf + 1, tables.pop())
        indices = []
        for tag, entry, data in tables:
            shared = next((index for index, (other, other_entry, other_data) in enumerate(entries)
                           if not entry.transformed and other == tag and other_data == data), None)
            if shared is None:
                shared = len(entries)
                entries.append((tag, entry, data))
                stream += data
            indices.append(shared)
        faces.append((flavor, indices))
    directory = b"".join(directory_entry(tag, entry) for tag, entry, _ in entries)
    directory += struct.pack(">L", COLLECTION_VERSION) + woff2.pack255UShort(len(faces))
    for flavor, indices in faces:
        directory += woff2.pack255UShort(len(indices)) + flavor.encode("latin-1")
        directory += b"".join(woff2.pack255UShort(index) for index in indices)
    compressed = brotli.compress(stream, mode=brotli.MODE_FONT)
    header_size = struct.calcsize(WOFF2_HEADER)
    length = header_size + len(directory) + len(compressed)
    total_sfnt_size = sum(len(data) for _, _, data in entries)
    header = struct.pack(WOFF2_HEADER, b"wOF2", int.from_bytes(b"ttcf", "big"), length,
                         len(entries), 0, total_sfnt_size, len(compressed), 1, 0, 0, 0, 0, 0, 0)
    target.write_bytes(header + directory + compressed)
    return target


faces = {style: face(style, source) for style, source in FACES.items()}
regular = faces["Regular"]
woff_data = WOFFFlavorData()
woff_data.metaData, woff_data.privData = METADATA, PRIVATE
woff2_data = woff2.WOFF2FlavorData(transformedTables=TRANSFORMED)
woff2_data.metaData, woff2_data.privData = METADATA, PRIVATE
outputs = [
    packaged(regular, FIXTURES / "WoffTest-Regular.woff", "woff", woff_data),
    packaged(regular, FIXTURES / "WoffTest-Regular.woff2", "woff2", woff2_data),
    packaged(regular, FIXTURES / "WoffTest-Regular-NullGlyf.woff2", "woff2",
             woff2.WOFF2FlavorData(transformedTables={"hmtx"})),
]
singles = []
for style, path in faces.items():
    single = packaged(path, FIXTURES / f"target-{style}.woff2", "woff2",
                      woff2.WOFF2FlavorData(transformedTables=TRANSFORMED))
    singles.append(woff2_entries(single))
    single.unlink()
outputs.append(collection(singles, FIXTURES / "WoffTest-Collection.woff2"))
ttc = TTCollection()
ttc.fonts = [TTFont(path, recalcTimestamp=False) for path in faces.values()]
ttc.save(FIXTURES / "WoffTest.ttc", shareTables=True)
outputs.append(FIXTURES / "WoffTest.ttc")

# Independent read-back: every package decodes to the outlines and metrics of its face.
def summary(font):
    glyphs = font.getGlyphSet()
    return {name: (glyphs[name].width, font["glyf"][name].getCoordinates(font["glyf"])[0].array.tolist()
                   if font["glyf"][name].numberOfContours else [])
            for name in font.getGlyphOrder()}


expected = {style: summary(TTFont(path)) for style, path in faces.items()}
for path in outputs[:3]:
    assert summary(TTFont(path)) == expected["Regular"], path
for index, style in enumerate(FACES):
    assert summary(TTFont(FIXTURES / "WoffTest.ttc", fontNumber=index)) == expected[style]
transforms = {tag: entry.transformVersion for tag, entry in
              woff2.WOFF2Reader((FIXTURES / "WoffTest-Regular.woff2").open("rb")).tables.items()}
assert transforms["glyf"] == 0 and transforms["hmtx"] == 1, transforms
with TTFont(regular) as font:
    font["glyf"][OVERLAP_GLYPH].expand(font["glyf"])
    assert font["glyf"][OVERLAP_GLYPH].flags[0] & OVERLAP_SIMPLE
    composites = [name for name in font.getGlyphOrder() if font["glyf"][name].isComposite()]
    assert composites, "the subset keeps composite glyphs"
record = {
    "derived_from": {style: str(path.relative_to(ROOT)) for style, path in FACES.items()},
    "source_sha256": {style: hashlib.sha256(path.read_bytes()).hexdigest()
                      for style, path in FACES.items()},
    "license": "../../../assets/fonts/LIBERATION-LICENSE.txt",
    "generator": Path(__file__).name,
    "tool": f"FontTools {fontTools.__version__}, Brotli {brotli.__version__}",
    "changes": "Subset to U+0020-U+007E and Éé with hinting kept, renamed family; the regular "
               "face's O sets OVERLAP_SIMPLE. Packaged as WOFF with zlib tables and metadata and "
               "private blocks; WOFF2 with transformed glyf, loca and hmtx and the same blocks; "
               "WOFF2 with only hmtx transformed; a TrueType collection sharing identical "
               "tables; and a WOFF2 collection of the two transformed faces.",
    "composite_glyphs": composites,
    "files": {path.name: {"sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
                          "bytes": path.stat().st_size}
              for path in [*faces.values(), *outputs]},
}
(FIXTURES / "WoffTest-source.json").write_text(json.dumps(record, indent=2) + "\n")
