"""Independent small manifest inputs shared by oracle tooling tests."""
import copy
import hashlib
import json
from pathlib import Path

TEST_SOURCE_ID = "css-sizing-3"
TEST_REQUIREMENT_ID = "sizing.width"
TEST_CLASSES = ("invoices", "reports", "books", "multilingual", "tables", "images")
TEST_SCALES = ("small", "medium", "large")


def write_json(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2) + "\n")


def valid_bundle(root: Path) -> dict:
    source = {
        "id": TEST_SOURCE_ID,
        "url": "https://www.w3.org/TR/2021/CRD-css-sizing-3-20211217/",
        "sha256": "a" * 64,
        "licence": "W3C document licence",
        "maturity": "CRD",
        "dependencies": [],
    }
    sections = [{"id": "width", "title": "Width", "disposition": {
        "kind": "requirements", "requirement_ids": [TEST_REQUIREMENT_ID],
    }}]
    requirement = {
        "id": TEST_REQUIREMENT_ID, "source_id": TEST_SOURCE_ID,
        "sections": ["width"], "behaviour": "Explicit widths determine the content box.",
        "applicability": {"status": "applicable", "reason": "Static box geometry."},
        "dependencies": [], "implementation": {"status": "unknown", "evidence": []},
        "case_ids": [], "priority": "foundations",
    }
    cases = []
    classes = []
    for family in TEST_CLASSES:
        ids = []
        for scale in TEST_SCALES:
            case_id = f"{family}-{scale}"
            ids.append(case_id)
            cases.append({
                "id": case_id, "input": "probe.html", "scale": scale,
                "requirement_ids": [TEST_REQUIREMENT_ID], "timeout_s": 120,
                "assertions": [],
                "provenance": {"kind": "independent", "licence": "CC0"},
                "warnings": [],
            })
        classes.append({"id": family, "case_ids": ids})
    suite = {"classes": classes, "protocol": {"warmup_runs": 3, "timed_runs": 30}}
    from comparison import default_settings
    defaults = {"tolerances": default_settings()}
    index = {"schema_version": 1, "source_id": TEST_SOURCE_ID, "sections": sections}
    index_path = root / "sections" / f"{TEST_SOURCE_ID}.json"
    write_json(index_path, index)
    source["index_sha256"] = hashlib.sha256(index_path.read_bytes()).hexdigest()
    write_json(root / "sources.json", {"schema_version": 1, "sources": [source]})
    write_json(root / "requirements" / "sizing.json", {"schema_version": 1, "requirements": [requirement]})
    write_json(root / "cases.json", {"schema_version": 1, "cases": cases})
    write_json(root / "benchmarks.json", {"schema_version": 1, **suite})
    write_json(root / "defaults.json", {"schema_version": 1, **defaults})
    (root / "probe.html").write_text("<!doctype html><style>p{width:100pt}</style><p>Probe</p>\n")
    return copy.deepcopy({
        "sources": {TEST_SOURCE_ID: source}, "sections": {TEST_SOURCE_ID: sections},
        "requirements": {TEST_REQUIREMENT_ID: requirement},
        "cases": {case["id"]: case for case in cases}, "benchmarks": suite,
        "defaults": defaults, "scope": "full", "root": root,
    })


def write_probe_pdf(path: Path, variant: str = 'base', *, content_stream: bytes | None = None) -> Path:
    from pdf_content import Name, Ref, serialize
    page_count = 1 if variant == 'missing-page' else 2
    objects = [b'', b'', serialize({'Type': Name('Font'), 'Subtype': Name('Type1'), 'BaseFont': Name('Helvetica'), 'Encoding': Name('WinAnsiEncoding'), 'FirstChar': 0, 'LastChar': 255, 'Widths': [600] * 256})]
    pages = []
    for index in range(page_count):
        page_id = len(objects) + 1
        content_id = page_id + 1
        pages.append(Ref(page_id, 0))
        x = 38 if variant == 'shifted' else 36.3 if variant == 'subpoint-shift' else 36
        word = 'Prob' if variant == 'missing-glyph' else 'Probe'
        colour = '1 0 0 rg' if variant == 'colour' else '0 0 0 rg'
        content = f'{colour}\nBT /F1 12 Tf 1 0 0 1 {x} 36 Tm ({word}) Tj ET\n'.encode()
        if content_stream is not None:
            content = content_stream
        if variant == 'clipped':
            content = b'q 0 0 10 10 re W n\n' + content + b'Q\n'
        if variant == 'occluded':
            content += b'1 1 1 rg 30 30 70 30 re f\n'
        objects.append(serialize({'Type': Name('Page'), 'Parent': Ref(2, 0), 'MediaBox': [0, 0, 150, 100], 'Resources': {'Font': {'F1': Ref(3, 0)}}, 'Contents': Ref(content_id, 0)}))
        objects.append(serialize({'Length': len(content)}) + b'\nstream\n' + content + b'endstream')
    objects[0] = serialize({'Type': Name('Catalog'), 'Pages': Ref(2, 0)})
    objects[1] = serialize({'Type': Name('Pages'), 'Count': page_count, 'Kids': pages})
    return write_pdf_objects(path, objects)


def write_pdf_objects(path: Path, objects: list[bytes]) -> Path:
    """Write an object-numbered PDF with object 1 as its catalog."""
    from pdf_content import Ref, serialize
    data = bytearray(b'%PDF-1.4\n')
    offsets = [0]
    for number, body in enumerate(objects, start=1):
        offsets.append(len(data))
        data += f'{number} 0 obj\n'.encode() + body + b'\nendobj\n'
    xref = len(data)
    data += f'xref\n0 {len(offsets)}\n0000000000 65535 f \n'.encode()
    for offset in offsets[1:]:
        data += f'{offset:010d} 00000 n \n'.encode()
    data += b'trailer\n' + serialize({'Size': len(offsets), 'Root': Ref(1, 0)})
    data += f'\nstartxref\n{xref}\n%%EOF\n'.encode()
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(data)
    return path
