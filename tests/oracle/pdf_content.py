"""Minimal PDF reader for the oracle: objects, content streams, painted glyphs, navigation and text-free copies."""
import re
import hashlib
import xml.etree.ElementTree as ET
import zlib
from dataclasses import dataclass
from pathlib import Path

from fontTools import agl

WHITESPACE = b"\x00\t\n\x0c\r "
DELIMITERS = b"()<>[]{}/%"
NUMBER = re.compile(rb"[+-]?(?:\d+\.?\d*|\.\d+)")
INDIRECT = re.compile(rb"(\d+)\s+(\d+)\s+R(?=[\x00\t\n\x0c\r ()<>\[\]{}/%]|$)")
XREF_ENTRY = re.compile(rb"(\d{10})\s(\d{5})\s([nf])")
OBJECT_HEADER = re.compile(rb"(\d+)\s+(\d+)\s+obj")
TEXT_SHOWING = {"Tj", "TJ", "'", '"'}
CLIPPING_RENDER_MODES = {4, 5, 6, 7}
STROKING_RENDER_MODES = {1, 2, 5, 6}
FILLING_RENDER_MODES = {0, 2, 4, 6}
IDENTITY = (1.0, 0.0, 0.0, 1.0, 0.0, 0.0)
GLYPH_SPACE_UNITS = 1000.0
SINGLE_BYTE_SPACE = 32
PNG_PREDICTOR_MIN = 10
SIMPLE_ENCODINGS = {"WinAnsiEncoding": "cp1252", "MacRomanEncoding": "mac_roman"}


class PdfError(Exception):
    pass


class Name(str):
    pass


class Operator(str):
    pass


@dataclass(frozen=True)
class Ref:
    number: int
    generation: int


@dataclass
class Stream:
    entries: dict
    raw: bytes


def regular_end(data, position):
    while position < len(data) and data[position] not in WHITESPACE and data[position] not in DELIMITERS:
        position += 1
    return position


def skip_space(data, position):
    while position < len(data):
        if data[position] in WHITESPACE:
            position += 1
        elif data[position] == ord("%"):
            while position < len(data) and data[position] not in b"\r\n":
                position += 1
        else:
            break
    return position


def literal_string(data, position):
    escapes = {ord("n"): b"\n", ord("r"): b"\r", ord("t"): b"\t", ord("b"): b"\b", ord("f"): b"\f"}
    output, depth, position = bytearray(), 1, position + 1
    while True:
        byte = data[position]
        if byte == ord("\\"):
            position += 1
            byte = data[position]
            if byte in escapes:
                output += escapes[byte]
            elif byte in b"01234567":
                digits = re.match(rb"[0-7]{1,3}", data[position:position + 3]).group()
                output.append(int(digits, 8) & 0xFF)
                position += len(digits) - 1
            elif byte == ord("\r"):
                position += data[position + 1:position + 2] == b"\n"
            elif byte != ord("\n"):
                output.append(byte)
        elif byte == ord("("):
            depth += 1
            output.append(byte)
        elif byte == ord(")"):
            depth -= 1
            if not depth:
                return bytes(output), position + 1
            output.append(byte)
        else:
            output.append(byte)
        position += 1


def parse(data, position):
    """Parse one object or content-stream operator at `position`; returns (value, end)."""
    position = skip_space(data, position)
    if position >= len(data):
        raise PdfError("unexpected end of data")
    byte = data[position]
    if data.startswith(b"<<", position):
        entries, position = {}, position + 2
        while True:
            position = skip_space(data, position)
            if data.startswith(b">>", position):
                return entries, position + 2
            key, position = parse(data, position)
            value, position = parse(data, position)
            entries[key] = value
    if byte == ord("<"):
        end = data.index(b">", position)
        digits = re.sub(rb"\s", b"", data[position + 1:end])
        return bytes.fromhex((digits + b"0" * (len(digits) % 2)).decode()), end + 1
    if byte == ord("("):
        return literal_string(data, position)
    if byte == ord("["):
        items, position = [], position + 1
        while True:
            position = skip_space(data, position)
            if data[position] == ord("]"):
                return items, position + 1
            item, position = parse(data, position)
            items.append(item)
    if byte == ord("/"):
        end = regular_end(data, position + 1)
        name = re.sub(rb"#([0-9A-Fa-f]{2})", lambda m: bytes([int(m.group(1), 16)]), data[position + 1:end])
        return Name(name.decode("latin-1")), end
    reference = INDIRECT.match(data, position)
    if reference:
        return Ref(int(reference.group(1)), int(reference.group(2))), reference.end()
    number = NUMBER.match(data, position)
    if number and regular_end(data, position) == number.end():
        text = number.group()
        return (float(text) if b"." in text else int(text)), number.end()
    if byte in b"{}":
        return Operator(chr(byte)), position + 1
    end = regular_end(data, position)
    if end == position:
        raise PdfError(f"unexpected delimiter {chr(byte)!r} at {position}")
    word = data[position:end].decode("latin-1")
    return {"true": True, "false": False, "null": None}.get(word, Operator(word)), end


def png_unpredict(data, columns):
    rows, previous = [], bytearray(columns)
    for start in range(0, len(data), columns + 1):
        kind, row = data[start], bytearray(data[start + 1:start + 1 + columns])
        for index in range(len(row)):
            left = row[index - 1] if index else 0
            up, up_left = previous[index], previous[index - 1] if index else 0
            if kind == 1:
                row[index] = (row[index] + left) & 0xFF
            elif kind == 2:
                row[index] = (row[index] + up) & 0xFF
            elif kind == 3:
                row[index] = (row[index] + (left + up) // 2) & 0xFF
            elif kind == 4:
                estimate = left + up - up_left
                distances = (abs(estimate - left), abs(estimate - up), abs(estimate - up_left))
                row[index] = (row[index] + (left, up, up_left)[distances.index(min(distances))]) & 0xFF
            elif kind:
                raise PdfError(f"unsupported PNG predictor row type {kind}")
        rows.append(bytes(row))
        previous = row
    return b"".join(rows)


class Document:
    def __init__(self, path):
        self.path = Path(path)
        self.data = self.path.read_bytes()
        self.offsets = {}
        self.compressed = {}
        self.cache = {}
        self.object_streams = {}
        self.destination_names = None
        self.trailer = {}
        self.startxref = int(re.findall(rb"startxref\s+(\d+)", self.data)[-1])
        self.read_xref(self.startxref)

    def read_xref(self, position):
        seen = set()
        while position is not None and position not in seen:
            seen.add(position)
            position = skip_space(self.data, position)
            if self.data.startswith(b"xref", position):
                entries, position = self.read_xref_table(position + len(b"xref"))
            else:
                entries, position = self.read_xref_stream(position)
            for key, value in entries.items():
                self.trailer.setdefault(key, value)
            if "XRefStm" in entries:
                self.read_xref_stream(entries["XRefStm"])
            position = entries.get("Prev")

    def record(self, number, generation, offset=None, container=None):
        if number in self.offsets or number in self.compressed:
            return
        if container is None:
            self.offsets[number] = (offset, generation)
        else:
            self.compressed[number] = container

    def read_xref_table(self, position):
        while True:
            position = skip_space(self.data, position)
            if self.data.startswith(b"trailer", position):
                return parse(self.data, position + len(b"trailer"))[0], None
            start, position = parse(self.data, position)
            count, position = parse(self.data, position)
            for index in range(count):
                entry = XREF_ENTRY.match(self.data, skip_space(self.data, position))
                if entry.group(3) == b"n":
                    self.record(start + index, int(entry.group(2)), int(entry.group(1)))
                position = entry.end()

    def read_xref_stream(self, position):
        stream = self.parse_indirect(position)[1]
        widths = stream.entries["W"]
        data = self.decode(stream)
        index = stream.entries.get("Index", [0, stream.entries["Size"]])
        row_length, cursor = sum(widths), 0
        for start, count in zip(index[::2], index[1::2]):
            for number in range(start, start + count):
                row = data[cursor:cursor + row_length]
                cursor += row_length
                fields, field_start = [], 0
                for width in widths:
                    fields.append(int.from_bytes(row[field_start:field_start + width], "big") if width else None)
                    field_start += width
                kind = 1 if fields[0] is None else fields[0]
                if kind == 1:
                    self.record(number, fields[2] or 0, fields[1])
                elif kind == 2:
                    self.record(number, 0, container=(fields[1], fields[2]))
        return stream.entries, None

    def parse_indirect(self, position):
        header = OBJECT_HEADER.match(self.data, skip_space(self.data, position))
        if not header:
            raise PdfError(f"no object header at {position}")
        value, position = parse(self.data, header.end())
        position = skip_space(self.data, position)
        if self.data.startswith(b"stream", position):
            position += len(b"stream")
            position += 2 if self.data.startswith(b"\r\n", position) else 1
            length = self.resolve(value["Length"])
            value = Stream(value, self.data[position:position + length])
        return int(header.group(1)), value

    def get(self, number):
        if number not in self.cache:
            if number in self.offsets:
                self.cache[number] = self.parse_indirect(self.offsets[number][0])[1]
            elif number in self.compressed:
                container, index = self.compressed[number]
                self.cache[number] = self.object_stream(container)[index]
            else:
                self.cache[number] = None
        return self.cache[number]

    def object_stream(self, number):
        if number not in self.object_streams:
            stream = self.get(number)
            data = self.decode(stream)
            first, count = stream.entries["First"], stream.entries["N"]
            header, position = [], 0
            for _ in range(2 * count):
                value, position = parse(data, position)
                header.append(value)
            self.object_streams[number] = [parse(data, first + offset)[0] for offset in header[1::2]]
        return self.object_streams[number]

    def resolve(self, value):
        seen = set()
        while isinstance(value, Ref):
            if value in seen:
                raise PdfError(f"cyclic object reference {value}")
            seen.add(value)
            value = self.get(value.number)
        return value

    def named_destination(self, name):
        root = self.resolve(self.trailer["Root"])
        if isinstance(name, Name):
            destinations = self.resolve(root.get("Dests", {}))
            if not isinstance(destinations, dict):
                raise PdfError("catalog Dests is not a dictionary")
        else:
            if self.destination_names is None:
                names = self.resolve(root.get("Names", {}))
                if not isinstance(names, dict):
                    raise PdfError("catalog Names is not a dictionary")
                self.destination_names = self.name_tree(names.get("Dests"))
            destinations = self.destination_names
        if name not in destinations:
            raise PdfError(f"missing named destination {name!r}")
        value = self.resolve(destinations[name])
        if isinstance(value, dict):
            if "D" not in value:
                raise PdfError(f"named destination {name!r} has no D entry")
            value = self.resolve(value["D"])
        return value

    def name_tree(self, root):
        values, seen = {}, set()
        pending = [] if root is None else [root]
        while pending:
            node = self.resolve(pending.pop())
            if not isinstance(node, dict):
                raise PdfError("name tree node is not a dictionary")
            identity = id(node)
            if identity in seen:
                raise PdfError("cyclic or shared name tree node")
            seen.add(identity)
            if ("Kids" in node) == ("Names" in node):
                raise PdfError("name tree node must contain Kids or Names")
            if "Kids" in node:
                children = self.resolve(node["Kids"])
                if not isinstance(children, list):
                    raise PdfError("name tree Kids is not an array")
                pending.extend(reversed(children))
            else:
                pairs = self.resolve(node["Names"])
                if not isinstance(pairs, list) or len(pairs) % 2:
                    raise PdfError("name tree Names is not a key/value array")
                for key, value in zip(pairs[::2], pairs[1::2]):
                    key = self.resolve(key)
                    if not isinstance(key, bytes):
                        raise PdfError("name tree key is not a byte string")
                    if key in values:
                        raise PdfError(f"duplicate name tree key {key!r}")
                    values[key] = value
        return values

    def decode(self, stream):
        filters = self.resolve(stream.entries.get("Filter", []))
        filters = [filters] if isinstance(filters, Name) else filters
        parameters = self.resolve(stream.entries.get("DecodeParms", [None] * len(filters)))
        parameters = [parameters] if isinstance(parameters, dict) or parameters is None else parameters
        data = stream.raw
        for name, parameter in zip(filters, parameters):
            if name != "FlateDecode":
                raise PdfError(f"unsupported stream filter {name}")
            data = zlib.decompress(data)
            parameter = self.resolve(parameter) or {}
            if parameter.get("Predictor", 1) >= PNG_PREDICTOR_MIN:
                data = png_unpredict(data, parameter.get("Columns", 1))
        return data

    def pages(self):
        output = []

        def walk(node, inherited):
            node = self.resolve(node)
            inherited = {**inherited, **{key: node[key] for key in ("Resources", "MediaBox") if key in node}}
            if "Kids" in node:
                for kid in self.resolve(node["Kids"]):
                    walk(kid, inherited)
            else:
                output.append({**node, **inherited})

        walk(self.resolve(self.trailer["Root"])["Pages"], {})
        return output

    def contents(self, page):
        contents = page.get("Contents", [])
        if isinstance(contents, Ref) and isinstance(self.resolve(contents), list):
            contents = self.resolve(contents)
        return contents if isinstance(contents, list) else [contents]


def multiply(m, n):
    a, b, c, d, e, f = m
    p, q, r, s, t, u = n
    return (a * p + b * r, a * q + b * s, c * p + d * r, c * q + d * s, e * p + f * r + t, e * q + f * s + u)


def canonical_font_name(name):
    name = re.sub(r"^[A-Z]{6}\+", "", name)
    return re.sub(r"-Identity-[HV]$", "", name)


def unicode_cmap(document, stream):
    mapping = {}
    if stream is None:
        return mapping
    data, position, operands = document.decode(document.resolve(stream)), 0, []
    while True:
        position = skip_space(data, position)
        if position >= len(data):
            return mapping
        value, position = parse(data, position)
        if not isinstance(value, Operator):
            operands.append(value)
            continue
        if value == "endbfchar":
            for source, target in zip(operands[::2], operands[1::2]):
                mapping[source] = target.decode("utf-16-be")
        elif value == "endbfrange":
            for low, high, target in zip(operands[::3], operands[1::3], operands[2::3]):
                first, last = int.from_bytes(low, "big"), int.from_bytes(high, "big")
                for offset, code in enumerate(range(first, last + 1)):
                    key = code.to_bytes(len(low), "big")
                    if isinstance(target, list):
                        mapping[key] = target[offset].decode("utf-16-be")
                    else:
                        base = bytearray(target)
                        base[-1] += offset
                        mapping[key] = bytes(base).decode("utf-16-be")
        operands = []


@dataclass
class Font:
    name: str
    code_length: int
    widths: dict
    default_width: float
    unicode: dict
    width_scale: float
    fallback_codec: str | None
    differences: dict

    def codes(self, data):
        return [data[i:i + self.code_length] for i in range(0, len(data) - self.code_length + 1, self.code_length)]

    def text(self, code):
        if code in self.unicode:
            return self.unicode[code]
        value = int.from_bytes(code, "big")
        if value in self.differences and agl.toUnicode(self.differences[value]):
            return agl.toUnicode(self.differences[value])
        if self.fallback_codec:
            return code.decode(self.fallback_codec)
        raise PdfError(f"no Unicode mapping for code {code.hex()} in {self.name}")

    def advance(self, code):
        width = self.widths.get(int.from_bytes(code, "big"), self.default_width)
        return width * self.width_scale


def load_font(document, reference):
    font = document.resolve(reference)
    subtype = font["Subtype"]
    unicode = unicode_cmap(document, font.get("ToUnicode"))
    if subtype == "Type0":
        if font["Encoding"] not in ("Identity-H", "Identity-V"):
            raise PdfError(f"unsupported CID encoding {font['Encoding']}")
        descendant = document.resolve(document.resolve(font["DescendantFonts"])[0])
        widths, entries, index = {}, document.resolve(descendant.get("W", [])), 0
        while index < len(entries):
            first, following = entries[index], document.resolve(entries[index + 1])
            if isinstance(following, list):
                widths.update({first + offset: width for offset, width in enumerate(following)})
                index += 2
            else:
                widths.update({code: entries[index + 2] for code in range(first, following + 1)})
                index += 3
        default = document.resolve(descendant.get("DW", GLYPH_SPACE_UNITS))
        return Font(canonical_font_name(font["BaseFont"]), 2, widths, default, unicode, 1 / GLYPH_SPACE_UNITS, None, {})
    first = font.get("FirstChar", 0)
    widths = {first + offset: width for offset, width in enumerate(document.resolve(font.get("Widths", [])))}
    descriptor = document.resolve(font.get("FontDescriptor")) or {}
    encoding = document.resolve(font.get("Encoding"))
    differences, codec = {}, "latin-1"
    if isinstance(encoding, dict):
        code = 0
        for item in encoding.get("Differences", []):
            if isinstance(item, int):
                code = item
            else:
                differences[code] = item
                code += 1
        encoding = encoding.get("BaseEncoding")
    if encoding in SIMPLE_ENCODINGS:
        codec = SIMPLE_ENCODINGS[encoding]
    if subtype == "Type3":
        name = descriptor.get("FontName") or font.get("Name") or f"Type3-{reference.number}"
        scale = float(document.resolve(font["FontMatrix"])[0])
        return Font(f"Type3:{name}", 1, widths, 0, unicode, scale, codec, differences)
    return Font(canonical_font_name(font["BaseFont"]), 1, widths, descriptor.get("MissingWidth", 0), unicode, 1 / GLYPH_SPACE_UNITS, codec, differences)


@dataclass
class Glyph:
    page: int
    text: str
    font: str
    render_mode: int
    fill: tuple
    stroke: tuple | None
    matrix: tuple

    @property
    def origin(self):
        return self.matrix[4], self.matrix[5]


@dataclass
class ReplacementText:
    page: int
    text: str
    glyph_start: int
    glyph_end: int


def colour(operands):
    values = [float(value) for value in operands if isinstance(value, (int, float))]
    patterns = [value for value in operands if isinstance(value, Name)]
    if patterns:
        return (f"pattern:{patterns[0]}",) + tuple(values)
    if len(values) == 1:
        return (values[0],) * 3
    if len(values) == 3:
        return tuple(values)
    if len(values) == 4:
        c, m, y, k = values
        return tuple((1 - min(1, channel + k)) for channel in (c, m, y))
    raise PdfError(f"unsupported colour operands {operands}")


class Interpreter:
    """Executes content streams, recording every painted glyph."""

    def __init__(self, document):
        self.document = document
        self.fonts = {}
        self.glyphs = []
        self.images = []
        self.replacement_text = []

    def font(self, resources, name):
        reference = self.document.resolve(self.document.resolve(resources.get("Font", {})))[name]
        key = reference if isinstance(reference, Ref) else id(reference)
        if key not in self.fonts:
            self.fonts[key] = load_font(self.document, reference)
        return self.fonts[key]

    def run_page(self, index, page):
        data = b"\n".join(self.document.decode(self.document.resolve(reference)) for reference in self.document.contents(page))
        graphics = {"ctm": IDENTITY, "fill": (0.0,) * 3, "stroke": (0.0,) * 3, "fill_alpha": 1.0, "stroke_alpha": 1.0,
                    "group_fill_alpha": 1.0, "group_stroke_alpha": 1.0,
                    "font": None, "size": 0.0, "char": 0.0, "word": 0.0, "scale": 1.0, "leading": 0.0, "rise": 0.0, "mode": 0}
        self.run(index, data, self.document.resolve(page.get("Resources", {})), graphics, set())

    def run(self, page, data, resources, graphics, active, replacement=None):
        """Forms start from the graphics state at `Do` (PDF 32000 §8.10.1); alphas inside a
        transparency group are tracked as their product with the group's."""
        document = self.document
        stack, operands, position = [], [], 0
        marked = [replacement]
        text_matrix = line_matrix = IDENTITY
        while True:
            position = skip_space(data, position)
            if position >= len(data):
                if len(marked) != 1:
                    raise PdfError("unclosed marked-content sequence")
                return
            value, position = parse(data, position)
            if not isinstance(value, Operator):
                operands.append(value)
                continue
            op = value
            if op in ("BDC", "BMC"):
                expected = 2 if op == "BDC" else 1
                if len(operands) != expected or not isinstance(operands[0], Name):
                    raise PdfError(f"invalid {op} operands")
                current = marked[-1]
                if op == "BDC":
                    properties = operands[1]
                    if isinstance(properties, Name):
                        named = document.resolve(resources.get("Properties", {}))
                        if properties not in named:
                            raise PdfError(f"missing marked-content properties {properties}")
                        properties = named[properties]
                    properties = document.resolve(properties)
                    if not isinstance(properties, dict):
                        raise PdfError("marked-content properties are not a dictionary")
                    if "ActualText" in properties:
                        actual = document.resolve(properties["ActualText"])
                        if not isinstance(actual, bytes):
                            raise PdfError("ActualText is not a text string")
                        text = text_string(actual)
                        if current is None:
                            current = len(self.replacement_text)
                            self.replacement_text.append(ReplacementText(page, text, len(self.glyphs), len(self.glyphs)))
                marked.append(current)
            elif op == "EMC":
                if operands or len(marked) == 1:
                    raise PdfError("unmatched marked-content end")
                current = marked.pop()
                if current is not None and current != marked[-1]:
                    self.replacement_text[current].glyph_end = len(self.glyphs)
            elif op == "BI":
                self.images.append({"page":page,"inline":True,"matrix":graphics["ctm"]})
                position = data.index(b"EI", data.index(b"ID", position)) + len(b"EI")
            elif op == "q":
                stack.append(dict(graphics))
            elif op == "Q":
                graphics = stack.pop()
            elif op == "cm":
                graphics["ctm"] = multiply(tuple(map(float, operands)), graphics["ctm"])
            elif op in ("g", "rg", "k", "sc", "scn"):
                graphics["fill"] = colour(operands)
            elif op in ("G", "RG", "K", "SC", "SCN"):
                graphics["stroke"] = colour(operands)
            elif op in ("cs", "CS"):
                graphics["fill" if op == "cs" else "stroke"] = (0.0,) * 3
            elif op == "gs":
                state = document.resolve(document.resolve(resources.get("ExtGState", {}))[operands[0]])
                if "ca" in state:
                    graphics["fill_alpha"] = graphics["group_fill_alpha"] * float(state["ca"])
                if "CA" in state:
                    graphics["stroke_alpha"] = graphics["group_stroke_alpha"] * float(state["CA"])
            elif op == "BT":
                text_matrix = line_matrix = IDENTITY
            elif op == "Tf":
                graphics["font"], graphics["size"] = self.font(resources, operands[0]), float(operands[1])
            elif op in ("Tc", "Tw", "Tz", "TL", "Ts", "Tr"):
                key = {"Tc": "char", "Tw": "word", "Tz": "scale", "TL": "leading", "Ts": "rise", "Tr": "mode"}[op]
                graphics[key] = float(operands[0]) / 100 if op == "Tz" else (int(operands[0]) if op == "Tr" else float(operands[0]))
            elif op in ("Td", "TD", "T*", "'", '"', "Tm"):
                if op == "Tm":
                    line_matrix = tuple(map(float, operands))
                else:
                    if op == "TD":
                        graphics["leading"] = -float(operands[1])
                    tx, ty = (float(operands[0]), float(operands[1])) if op in ("Td", "TD") else (0.0, -graphics["leading"])
                    line_matrix = multiply((1, 0, 0, 1, tx, ty), line_matrix)
                text_matrix = line_matrix
            if op == '"':
                graphics["word"], graphics["char"] = float(operands[0]), float(operands[1])
            if op in TEXT_SHOWING:
                strings = operands[-1] if op == "TJ" else [operands[-1]]
                text_matrix = self.show(page, graphics, text_matrix, strings)
            elif op == "Do":
                target = document.resolve(document.resolve(resources.get("XObject", {}))[operands[0]])
                if target.entries.get("Subtype") == "Image":
                    entries = target.entries
                    space = document.resolve(entries.get('ColorSpace'))
                    rgb = space=='DeviceRGB'
                    if isinstance(space,list) and space[0]=='ICCBased':
                        rgb = document.resolve(space[1]).entries.get('N')==3
                    filters=document.resolve(entries.get('Filter',[]))
                    filters=filters if isinstance(filters,list) else [filters]
                    decodable=all(name=='FlateDecode' for name in filters)
                    pixels = document.decode(target) if decodable and rgb and entries.get('BitsPerComponent')==8 else None
                    expected_bytes = entries['Width']*entries['Height']*3
                    self.images.append({'page':page,'inline':False,'matrix':graphics['ctm'],
                        'width_px':entries['Width'],'height_px':entries['Height'],
                        'pixels_sha256':hashlib.sha256(pixels).hexdigest() if pixels is not None and len(pixels)==expected_bytes else None})
                elif target.entries.get("Subtype") == "Form" and id(target) not in active:
                    matrix = tuple(map(float, target.entries.get("Matrix", IDENTITY)))
                    form_resources = document.resolve(target.entries.get("Resources", resources))
                    inner = dict(graphics, ctm=multiply(matrix, graphics["ctm"]))
                    if "Group" in target.entries:
                        inner["group_fill_alpha"], inner["group_stroke_alpha"] = graphics["fill_alpha"], graphics["stroke_alpha"]
                    self.run(page, document.decode(target), form_resources, inner, active | {id(target)}, marked[-1])
            operands = []

    def show(self, page, graphics, text_matrix, strings):
        font, size, scale = graphics["font"], graphics["size"], graphics["scale"]
        if graphics["mode"] in CLIPPING_RENDER_MODES:
            raise PdfError("clipping text render modes are not supported by the oracle")
        for item in strings:
            if isinstance(item, (int, float)):
                text_matrix = multiply((1, 0, 0, 1, -item / GLYPH_SPACE_UNITS * size * scale, 0), text_matrix)
                continue
            for code in font.codes(item):
                rendering = multiply(multiply((size * scale, 0, 0, size, 0, graphics["rise"]), text_matrix), graphics["ctm"])
                if graphics["mode"] != 3:
                    self.glyphs.append(Glyph(
                        page, font.text(code), font.name, graphics["mode"],
                        graphics["fill"] + (graphics["fill_alpha"],) if graphics["mode"] in FILLING_RENDER_MODES else None,
                        graphics["stroke"] + (graphics["stroke_alpha"],) if graphics["mode"] in STROKING_RENDER_MODES else None,
                        rendering))
                spacing = graphics["word"] if font.code_length == 1 and code[0] == SINGLE_BYTE_SPACE else 0.0
                advance = (font.advance(code) * size + graphics["char"] + spacing) * scale
                text_matrix = multiply((1, 0, 0, 1, advance, 0), text_matrix)
        return text_matrix


def inspect(path):
    document = Document(path)
    interpreter = Interpreter(document)
    for index, page in enumerate(document.pages(), start=1):
        interpreter.run_page(index, page)
    return {'glyphs':interpreter.glyphs,'images':interpreter.images,'replacement_text':interpreter.replacement_text}


def glyphs(path):
    return inspect(path)['glyphs']


# Text strings are UTF-16BE after this byte order mark, otherwise PDFDocEncoding (ISO 32000-1 §7.9.2.2).
UTF16_BOM = b"\xfe\xff"
# PDFDocEncoding agrees with ASCII over the printable range; other bytes need its own table.
PRINTABLE_ASCII = range(0x20, 0x7F)
# XMP properties that ISO 32000-1 §14.3.2 Table 317 relates to information dictionary entries.
XMP_NAMESPACES = {
    "rdf": "http://www.w3.org/1999/02/22-rdf-syntax-ns#",
    "dc": "http://purl.org/dc/elements/1.1/",
    "pdf": "http://ns.adobe.com/pdf/1.3/",
    "xmp": "http://ns.adobe.com/xap/1.0/",
}
XMP_INFO_PROPERTIES = ("dc:title", "dc:creator", "dc:description", "pdf:Keywords", "xmp:CreatorTool")


def text_string(value):
    if value.startswith(UTF16_BOM):
        return value[len(UTF16_BOM):].decode("utf-16-be")
    if any(byte not in PRINTABLE_ASCII for byte in value):
        raise PdfError(f"PDFDocEncoding text beyond printable ASCII is not decoded: {value!r}")
    return value.decode("ascii")


def page_refs(document):
    """Page object numbers in page order."""
    order = []

    def walk(reference):
        node = document.resolve(reference)
        if "Kids" in node:
            for kid in document.resolve(node["Kids"]):
                walk(kid)
        else:
            order.append(reference.number)

    walk(document.resolve(document.trailer["Root"])["Pages"])
    return order


def destination(document, pages, value):
    """Resolve ISO 32000-1 §12.3.2 destinations to a 1-based page and top."""
    value = document.resolve(value)
    if isinstance(value, (Name, bytes)):
        value = document.named_destination(value)
    if not isinstance(value, list) or len(value) < 2:
        raise PdfError(f"destination is not an explicit array: {value!r}")
    page, kind, *arguments = value
    if kind != "XYZ":
        raise PdfError(f"destination type {kind} is not read")
    if len(arguments) != 3 or not isinstance(page, Ref) or page.number not in pages:
        raise PdfError(f"invalid XYZ destination {value!r}")
    return {"page": pages.index(page.number) + 1, "top": document.resolve(arguments[1])}


def link_target(document, pages, annotation):
    if "Dest" in annotation:
        return {"destination": destination(document, pages, annotation["Dest"])}
    action = document.resolve(annotation["A"])
    if action["S"] == "URI":
        return {"uri": document.resolve(action["URI"]).decode("ascii")}
    if action["S"] == "GoTo":
        return {"destination": destination(document, pages, action["D"])}
    raise PdfError(f"link action {action['S']} is not read")


def outline_items(document, pages, reference):
    items = []
    while reference is not None:
        item = document.resolve(reference)
        children = outline_items(document, pages, item.get("First"))
        items.append({
            "title": text_string(document.resolve(item["Title"])),
            # A negative count closes the item (ISO 32000-1 §12.3.3).
            "open": document.resolve(item.get("Count", 0)) >= 0,
            "destination": destination(document, pages, item["Dest"]),
            "children": children,
        })
        reference = item.get("Next")
    return items


def xmp_properties(document, root):
    if "Metadata" not in root:
        return None
    tree = ET.fromstring(document.decode(document.resolve(root["Metadata"])))
    properties = {}
    for name in XMP_INFO_PROPERTIES:
        element = tree.find(f".//{name}", XMP_NAMESPACES)
        if element is not None:
            values = [item.text or "" for item in element.iter(f"{{{XMP_NAMESPACES['rdf']}}}li")]
            properties[name] = values if values else [element.text or ""]
    return properties


def navigation(path):
    """Link annotations, outline, information dictionary, catalog language and XMP of a PDF."""
    document = Document(path)
    pages = page_refs(document)
    root = document.resolve(document.trailer["Root"])
    links = []
    for number, page in enumerate(document.pages(), start=1):
        for reference in document.resolve(page.get("Annots", [])) or []:
            annotation = document.resolve(reference)
            if annotation.get("Subtype") != "Link":
                continue
            quads = annotation.get("QuadPoints")
            links.append({
                "page": number,
                "rect": [float(value) for value in document.resolve(annotation["Rect"])],
                "quads": [float(value) for value in document.resolve(quads)] if quads is not None else None,
                **link_target(document, pages, annotation),
            })
    outlines = document.resolve(root.get("Outlines")) or {}
    info = document.resolve(document.trailer.get("Info")) or {}
    return {
        "links": links,
        "outline": outline_items(document, pages, outlines.get("First")),
        "info": {key: text_string(document.resolve(value)) for key, value in info.items()},
        "lang": text_string(document.resolve(root["Lang"])) if "Lang" in root else None,
        "xmp": xmp_properties(document, root),
    }


def strip_text(data):
    """Remove text-showing operators and their operands; every other operator is kept byte for byte."""
    kept, position, operand_start, cut = [], 0, 0, 0
    while True:
        position = skip_space(data, position)
        if position >= len(data):
            kept.append(data[cut:])
            return b"".join(kept)
        value, position = parse(data, position)
        if not isinstance(value, Operator):
            continue
        if value == "BI":
            position = data.index(b"EI", data.index(b"ID", position)) + len(b"EI")
        elif value in TEXT_SHOWING:
            kept.append(data[cut:operand_start])
            kept.append(b" ")
            cut = position
        operand_start = position


def serialize(value):
    if isinstance(value, bool):
        return b"true" if value else b"false"
    if value is None:
        return b"null"
    if isinstance(value, Name):
        return b"/" + re.sub(rb"[^!-~]|[#()<>\[\]{}/%]", lambda m: b"#%02X" % m.group()[0], value.encode("latin-1"))
    if isinstance(value, Ref):
        return b"%d %d R" % (value.number, value.generation)
    if isinstance(value, int):
        return b"%d" % value
    if isinstance(value, float):
        return (f"{value:.10f}".rstrip("0").rstrip(".") or "0").encode()
    if isinstance(value, bytes):
        return b"<" + value.hex().encode() + b">"
    if isinstance(value, list):
        return b"[" + b" ".join(map(serialize, value)) + b"]"
    if isinstance(value, dict):
        return b"<<" + b"".join(serialize(Name(key)) + b" " + serialize(item) for key, item in value.items()) + b">>"
    raise PdfError(f"cannot serialize {value!r}")


def content_streams(document):
    """Every page, annotation appearance, Form XObject and tiling-pattern content stream reference."""
    found, pending = [], []

    def add_resources(resources):
        resources = document.resolve(resources) or {}
        for category in ("XObject", "Pattern"):
            for reference in (document.resolve(resources.get(category)) or {}).values():
                pending.append(reference)

    for page in document.pages():
        found.extend(reference for reference in document.contents(page) if isinstance(reference, Ref))
        add_resources(page.get("Resources"))
        for annotation in document.resolve(page.get("Annots", [])) or []:
            appearances = document.resolve(document.resolve(annotation).get("AP", {})) or {}
            for appearance in appearances.values():
                appearance_value = document.resolve(appearance)
                if isinstance(appearance_value, Stream):
                    pending.append(appearance)
                else:
                    pending.extend(appearance_value.values())
    while pending:
        reference = pending.pop()
        stream = document.resolve(reference)
        if not isinstance(reference, Ref) or reference in found or not isinstance(stream, Stream):
            continue
        if stream.entries.get("Subtype") == "Form" or stream.entries.get("PatternType") == 1:
            found.append(reference)
            add_resources(stream.entries.get("Resources"))
    return found


def write_text_free(source, target):
    """Write `source` plus an incremental update replacing each content stream with a text-free copy."""
    document = Document(source)
    output = bytearray(document.data)
    if not output.endswith(b"\n"):
        output += b"\n"
    offsets = {}
    for reference in content_streams(document):
        stream = document.resolve(reference)
        data = strip_text(document.decode(stream))
        entries = {key: value for key, value in stream.entries.items() if key not in ("Filter", "DecodeParms", "Length", "DL")}
        entries["Length"] = len(data)
        offsets[reference] = len(output)
        output += b"%d %d obj\n" % (reference.number, reference.generation) + serialize(entries) + b"\nstream\n" + data + b"\nendstream\nendobj\n"
    xref = len(output)
    output += b"xref\n"
    for reference, offset in sorted(offsets.items(), key=lambda item: item[0].number):
        output += b"%d 1\n%010d %05d n \n" % (reference.number, offset, reference.generation)
    size = max([document.trailer.get("Size", 0)] + [reference.number + 1 for reference in offsets])
    trailer = {"Size": size, "Root": document.trailer["Root"], "Prev": document.startxref}
    for key in ("Info", "ID"):
        if key in document.trailer:
            trailer[key] = document.trailer[key]
    output += b"trailer\n" + serialize(trailer) + b"\nstartxref\n%d\n%%%%EOF\n" % xref
    Path(target).write_bytes(output)
