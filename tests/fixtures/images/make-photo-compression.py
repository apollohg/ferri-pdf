"""Generate small synthetic opaque 8/10-bit photos and independent PNG decodes."""
import hashlib
import json
from pathlib import Path
import struct
import subprocess
import tempfile
import zlib

from PIL import Image

HERE = Path(__file__).resolve().parent
SIZE = (128, 96)
CODECS = {
    "heic": ["-p", "preset=ultrafast", "-p", "x265:pools=1", "-p", "x265:frame-threads=1"],
    "avif": ["-A", "-p", "speed=9", "-p", "threads=1"],
}


def chunk(kind, data):
    return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))


def generate():
    pixels = [((x * 2) % 256, (y * 2) % 256, (x + y) % 256)
              for y in range(SIZE[1]) for x in range(SIZE[0])]
    records = {}
    with tempfile.TemporaryDirectory(prefix="ferri-pdf-photo-compression-") as directory:
        source = Path(directory) / "source.png"
        for variant in ("opaque", "10bit"):
            if variant == "opaque":
                with Image.new("RGB", SIZE) as image:
                    image.putdata(pixels)
                    image.save(source)
            else:
                rows = b"".join(b"\0" + b"".join(struct.pack(">3H", *(v * 257 for v in pixel))
                                               for pixel in pixels[y * SIZE[0]:(y + 1) * SIZE[0]])
                                for y in range(SIZE[1]))
                source.write_bytes(b"\x89PNG\r\n\x1a\n"
                                   + chunk(b"IHDR", struct.pack(">IIBBBBB", *SIZE, 16, 2, 0, 0, 0))
                                   + chunk(b"IDAT", zlib.compress(rows)) + chunk(b"IEND", b""))
            for codec, options in CODECS.items():
                target = HERE / f"photo-{variant}.{codec}"
                arguments = [*options, "-q", "95"]
                if variant == "10bit":
                    arguments += ["-b", "10"]
                subprocess.run(["heif-enc", *arguments, "-o", str(target), str(source)],
                               check=True, capture_output=True)
                reference = HERE / f"photo-{variant}-{codec}.png"
                subprocess.run(["heif-convert", str(target), str(reference)], check=True, capture_output=True)
                records[target.name] = {
                    "sha256": hashlib.sha256(target.read_bytes()).hexdigest(),
                    "reference": reference.name,
                    "reference_sha256": hashlib.sha256(reference.read_bytes()).hexdigest(),
                    "arguments": arguments,
                }
    (HERE / "photo-compression.json").write_text(json.dumps({
        "tool": subprocess.check_output(["heif-enc", "--version"], stderr=subprocess.STDOUT, text=True).strip(),
        "size": SIZE, "files": records,
    }, indent=2) + "\n")


if __name__ == "__main__":
    generate()
