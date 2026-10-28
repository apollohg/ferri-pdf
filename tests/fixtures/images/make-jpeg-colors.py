"""Generate independent RGB-component and YCCK JPEGs using libjpeg, never renderer code."""
import hashlib
import json
from pathlib import Path
import shlex
import subprocess
import tempfile

import PIL
from PIL import Image

HERE = Path(__file__).resolve().parent
source = HERE / "make-jpeg-colors.c"
flags = shlex.split(subprocess.check_output(["pkg-config", "--cflags", "--libs", "libjpeg"], text=True))
result = {
    "tool": subprocess.check_output(["cjpeg", "-version"], stderr=subprocess.STDOUT, text=True).strip(),
    "inspection_tool": "Pillow " + PIL.__version__,
    "generator_sha256": hashlib.sha256(source.read_bytes()).hexdigest(),
    "quality": 97,
    "files": {},
}
with tempfile.TemporaryDirectory(prefix="ferri-pdf-jpeg-reference-") as directory:
    executable = Path(directory) / "make-jpeg-colors"
    subprocess.run(["cc", "-O2", str(source), *flags, "-o", str(executable)], check=True)
    for mode in ["rgb", "ycck"]:
        target = HERE / (mode + ".jpg")
        subprocess.run([str(executable), str(target), mode], check=True)
        with Image.open(target) as image:
            result["files"][target.name] = {
                "sha256": hashlib.sha256(target.read_bytes()).hexdigest(),
                "size": image.size, "mode": image.mode,
                "adobe_transform": image.info.get("adobe_transform"),
            }
(HERE / "jpeg-colors.json").write_text(json.dumps(result, indent=2) + "\n")
