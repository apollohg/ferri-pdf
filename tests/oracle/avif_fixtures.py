#!/usr/bin/env python3
"""Build a separate alpha grid from the independently encoded AV1 tiles."""
import struct
from pathlib import Path

ASSETS = Path(__file__).resolve().parent / "assets"
BOX_HEADER = 8
FULL_BOX_HEADER = 4
ALPHA_GRID_ID = 6
COLOR_GRID_ID = 1
COLOR_TILES = (2, 4)
ALPHA_TILES = (3, 5)
ISPE_PROPERTY = 1
ALPHA_PROPERTY = 8
ESSENTIAL = 0x80


def box(kind, payload):
    return struct.pack(">I4s", BOX_HEADER + len(payload), kind) + payload


def boxes(data):
    offset = 0
    while offset < len(data):
        size, kind = struct.unpack_from(">I4s", data, offset)
        if size < BOX_HEADER or offset + size > len(data):
            raise ValueError("invalid fixture box bounds")
        yield kind, data[offset + BOX_HEADER:offset + size]
        offset += size


def full_box(version=0, flags=0):
    return bytes([version]) + flags.to_bytes(3, "big")


def alpha_grid():
    source = (ASSETS / "image-grid.avif").read_bytes()
    top = dict(boxes(source))
    meta = dict(boxes(top[b"meta"][FULL_BOX_HEADER:]))
    original = meta[b"iloc"]
    # Source libheif uses v1, 32-bit offsets/lengths/base offsets, one extent/item.
    if original[:6] != full_box(1) + b"\x44\x40":
        raise ValueError("unexpected source iloc field sizes")
    count = int.from_bytes(original[6:8], "big")
    records = []
    offset = 8
    item_data = {}
    for _ in range(count):
        item, method, data_ref, base, extents, start, length = struct.unpack_from(">HHHIHII", original, offset)
        offset += struct.calcsize(">HHHIHII")
        if data_ref or extents != 1:
            raise ValueError("unexpected source extent layout")
        backing = meta[b"idat"] if method == 1 else source
        item_data[item] = backing[base + start:base + start + length]
    item_data[ALPHA_GRID_ID] = item_data[COLOR_GRID_ID]
    data = bytearray()
    for item, payload in sorted(item_data.items()):
        records.append(struct.pack(">HHHHII", item, 1, 0, 1, len(data), len(payload)))
        data.extend(payload)
    meta[b"idat"] = bytes(data)
    meta[b"iloc"] = full_box(1) + b"\x44\x00" + struct.pack(">H", len(records)) + b"".join(records)
    iinf = meta[b"iinf"]
    entries = list(boxes(iinf[FULL_BOX_HEADER + 2:]))
    entries = [(kind, full_box(2, int.from_bytes(payload[4:6], "big") != COLOR_GRID_ID) + payload[FULL_BOX_HEADER:]) for kind, payload in entries]
    entries.append((b"infe", full_box(2, 1) + struct.pack(">HH4s", ALPHA_GRID_ID, 0, b"grid") + b"\0"))
    meta[b"iinf"] = full_box() + struct.pack(">H", len(entries)) + b"".join(box(k, p) for k, p in entries)
    iprp = dict(boxes(meta[b"iprp"]))
    ipma = iprp[b"ipma"]
    entry_count = int.from_bytes(ipma[FULL_BOX_HEADER:BOX_HEADER], "big")
    iprp[b"ipma"] = ipma[:FULL_BOX_HEADER] + struct.pack(">I", entry_count + 1) + ipma[BOX_HEADER:] + struct.pack(">HB", ALPHA_GRID_ID, 2) + bytes([ISPE_PROPERTY, ESSENTIAL | ALPHA_PROPERTY])
    meta[b"iprp"] = b"".join(box(k, p) for k, p in iprp.items())
    def reference(kind, origin, targets):
        return box(kind, struct.pack(">HH", origin, len(targets)) + b"".join(struct.pack(">H", target) for target in targets))
    meta[b"iref"] = full_box() + reference(b"dimg", COLOR_GRID_ID, COLOR_TILES) + reference(b"dimg", ALPHA_GRID_ID, ALPHA_TILES) + reference(b"auxl", ALPHA_GRID_ID, (COLOR_GRID_ID,))
    output = box(b"ftyp", top[b"ftyp"]) + box(b"meta", full_box() + b"".join(box(k, p) for k, p in meta.items()))
    (ASSETS / "image-alpha-grid.avif").write_bytes(output)


if __name__ == "__main__":
    alpha_grid()
