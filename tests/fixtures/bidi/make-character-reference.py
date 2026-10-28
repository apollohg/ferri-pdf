"""Regenerate CSS bidi references from the two pinned Unicode source files."""
import argparse
import hashlib
import html
import json
from pathlib import Path

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("character_test", type=Path)
parser.add_argument("mirroring", type=Path)
arguments = parser.parse_args()
reference_path = Path(__file__).with_name("character-reference.json")
reference = json.loads(reference_path.read_text())
sources = []
for path, digest in [(arguments.character_test, reference["sha256"]),
                     (arguments.mirroring, reference["mirroring_sha256"])]:
    data = path.read_bytes()
    if hashlib.sha256(data).hexdigest() != digest:
        parser.error(f"{path.name} does not match the pinned reference SHA-256")
    sources.append(data.decode().splitlines())

# CSS Writing Modes 3, section 2.4.2: opening control -> CSS scope and closer.
SCOPES = {
    0x202A: ("embed", "ltr", 0x202C),
    0x202B: ("embed", "rtl", 0x202C),
    0x202D: ("bidi-override", "ltr", 0x202C),
    0x202E: ("bidi-override", "rtl", 0x202C),
    0x2066: ("isolate", "ltr", 0x2069),
    0x2067: ("isolate", "rtl", 0x2069),
    0x2068: ("plaintext", "ltr", 0x2069),
}
CLOSERS = {closer for _, _, closer in SCOPES.values()}
FORMATTING = SCOPES.keys() | CLOSERS
relevant = set()
for case in reference["cases"]:
    record = sources[0][case["line"] - 1].split("#")[0].strip()
    if record != case["record"]:
        parser.error(f"selected record at line {case['line']} changed")
    codepoints, direction, _, _, order = record.split(";")
    points = [int(value, 16) for value in codepoints.split()]
    body, stack = [], []
    for point in points:
        if point in SCOPES:
            mode, direction_name, closer = SCOPES[point]
            body.append(f'<span style="unicode-bidi:{mode};direction:{direction_name}">')
            stack.append(closer)
        elif point in CLOSERS:
            if not stack or stack.pop() != point:
                parser.error(f"selected record at line {case['line']} has unbalanced controls")
            body.append("</span>")
        else:
            body.append(html.escape(chr(point), quote=False))
    if stack:
        parser.error(f"selected record at line {case['line']} has unclosed controls")
    case["direction"] = int(direction)
    case["body"] = "".join(body)
    case["logical"] = "".join(chr(point) for point in points if point not in FORMATTING)
    case["visual"] = "".join(chr(points[int(index)]) for index in order.split()
                             if points[int(index)] not in FORMATTING)
    relevant.update(points)

mirrors = {}
for line in sources[1]:
    fields = line.split("#")[0].strip().split(";")
    if len(fields) == 2 and int(fields[0], 16) in relevant:
        mirrors[fields[0].strip()] = fields[1].strip()
reference["mirrors"] = dict(sorted(mirrors.items()))
reference_path.write_text(json.dumps(reference, ensure_ascii=False, indent=2) + "\n")
