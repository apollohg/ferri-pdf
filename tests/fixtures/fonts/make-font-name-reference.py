"""Regenerate the name-comparison reference from its pinned Unicode source file."""
import argparse
import hashlib
import json
from pathlib import Path

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("case_folding", type=Path)
arguments = parser.parse_args()
reference_path = Path(__file__).with_name("font-name-matching.json")
reference = json.loads(reference_path.read_text())
data = arguments.case_folding.read_bytes()
if hashlib.sha256(data).hexdigest() != reference["sha256"]:
    parser.error("CaseFolding.txt does not match the pinned reference SHA-256")
mappings = {}
for line in data.decode().splitlines():
    fields = line.split("#")[0].strip().split(";")
    if len(fields) >= 3 and fields[1].strip() in {"C", "F"}:
        mappings[int(fields[0], 16)] = "".join(chr(int(value, 16)) for value in fields[2].split())


def fold(text):
    return "".join(mappings.get(ord(character), character) for character in text)


relevant = set()
for case in reference["cases"]:
    declared, requested = case["declared"], case["requested"]
    case["matches"] = fold(declared) == fold(requested)
    relevant.update(ord(character) for character in declared + requested if ord(character) in mappings)
reference["mappings"] = {f"{point:04X}": mappings[point] for point in sorted(relevant)}
reference_path.write_text(json.dumps(reference, ensure_ascii=False, indent=2) + "\n")
