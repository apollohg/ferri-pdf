"""Regenerate an independent layout reference with WeasyPrint 70.0."""
import argparse
import hashlib
import json
from pathlib import Path

import weasyprint
from weasyprint import HTML, __version__

ROOT = Path(__file__).resolve().parents[3]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("reference", type=Path, nargs="?",
                    default=Path(__file__).with_name("weasy70-widths.json"))
REFERENCE = parser.parse_args().reference
POINTS_PER_CSS_PIXEL = 72 / 96

reference = json.loads(REFERENCE.read_text())
if __version__ != reference["version"]:
    raise SystemExit(f"expected WeasyPrint {reference['version']}, got {__version__}")
for name, digest in reference["sources"].items():
    source = Path(weasyprint.__file__).parent / "layout" / name
    if hashlib.sha256(source.read_bytes()).hexdigest() != digest:
        raise SystemExit(f"reference engine {name} SHA-256 mismatch")
font = ROOT / reference["font_path"]
if hashlib.sha256(font.read_bytes()).hexdigest() != reference["font_sha256"]:
    raise SystemExit("reference font SHA-256 mismatch")
for case in (reference["cases"] + reference.get("parallel_flows", [])
             + reference.get("observations", [])):
    source = ("<!doctype html><style>@font-face{font-family:Probe;src:url('"
              + font.as_uri() + "')}" + reference["base_css"] + case["css"]
              + "</style>" + case["body"])
    document = HTML(string=source).render()
    measurement = reference.get("measurement", "widths")
    if measurement in ("lines", "line-geometry"):
        case["pages"] = [[{"text": box.text,
                           "baseline": (box.position_y + box.baseline) * POINTS_PER_CSS_PIXEL,
                           **({"x": box.position_x * POINTS_PER_CSS_PIXEL,
                               "width": box.width * POINTS_PER_CSS_PIXEL}
                              if measurement == "line-geometry" else {})}
                          for box in page._page_box.descendants(placeholders=True)
                          if type(box).__name__ == "TextBox"]
                         for page in document.pages]
    elif measurement == "widths":
        if len(document.pages) != 1:
            raise SystemExit(f"{case['name']} must produce one reference page")
        boxes = list(document.pages[0]._page_box.descendants())
        case["widths"] = {box.element.get("id"): box.border_width() * POINTS_PER_CSS_PIXEL
                          for box in boxes if type(box).__name__ == "TableCellBox"}
        case["table_width"] = next(box.border_width() * POINTS_PER_CSS_PIXEL
                                   for box in boxes if type(box).__name__ == "TableBox")
    else:
        raise SystemExit(f"unknown reference measurement {measurement}")
REFERENCE.write_text(json.dumps(reference, indent=2) + "\n")
