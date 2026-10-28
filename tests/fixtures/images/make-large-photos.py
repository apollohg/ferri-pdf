"""Generate synthetic phone-size HEIC/AVIF references with independent libheif tools."""
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile

from PIL import Image

HERE = Path(__file__).resolve().parent
SIZE = (8000, 6000)
COLOR = (19, 93, 201)
CODECS = {
    "heic": ["-p", "preset=ultrafast", "-p", "x265:pools=1", "-p", "x265:frame-threads=1"],
    "avif": ["-A", "-p", "speed=9", "-p", "threads=1"],
}


def generate():
    metadata = {
        "tool": subprocess.check_output(["heif-enc", "--version"], stderr=subprocess.STDOUT, text=True).strip(),
        "encoders": subprocess.check_output(["heif-enc", "--list-encoders"], stderr=subprocess.STDOUT, text=True).strip(),
        "generator_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
        "size": SIZE,
        "source_color": COLOR,
        "files": {},
    }
    with tempfile.TemporaryDirectory(prefix="ferri-pdf-large-photo-reference-") as directory:
        source = Path(directory) / "source.png"
        with Image.new("RGB", SIZE, COLOR) as photo:
            photo.save(source)
        for codec, arguments in CODECS.items():
            target = HERE / f"photo-48mp.{codec}"
            subprocess.run(["heif-enc", *arguments, "-o", str(target), str(source)], check=True)
            reference = Path(directory) / f"{codec}.png"
            subprocess.run(["heif-convert", str(target), str(reference)], check=True, capture_output=True)
            with Image.open(reference) as decoded:
                if decoded.size != SIZE or decoded.mode != "RGB":
                    raise ValueError(f"unexpected independent decode: {decoded.size}, {decoded.mode}")
                extrema = decoded.getextrema()
            metadata["files"][target.name] = {
                "sha256": hashlib.sha256(target.read_bytes()).hexdigest(),
                "arguments": arguments,
                "decoded_extrema": extrema,
            }
    (HERE / "large-photos.json").write_text(json.dumps(metadata, indent=2) + "\n")


if __name__ == "__main__":
    generate()
