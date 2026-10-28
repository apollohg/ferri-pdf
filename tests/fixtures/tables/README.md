# Independent table references

`weasy70-widths.json` contains 15 single-page reference cases measured with
WeasyPrint 70.0 and the unchanged bundled DejaVu Sans font. It records the font's
SHA-256 and the hashes of the release's `layout/preferred.py` and `layout/table.py`.
The cases distinguish absolute cell limits, conflicting limits, percentage and
zero-percentage limits, content-box and border-box widths, fixed layout, collapsed
borders, narrow specified cells and percentage-only columns below 100%.

Regenerate with a Python environment containing **WeasyPrint 70.0**:

```sh
PYTHONDONTWRITEBYTECODE=1 python3 tests/fixtures/tables/make-weasy70-widths.py
```

On macOS the environment may also require `DYLD_FALLBACK_LIBRARY_PATH` pointing to
the installed Pango/GLib libraries. The generator checks engine version, engine
source hashes and font hash, then measures the reference engine's actual border
boxes in PDF points. It imports no candidate renderer or layout trace. Reference
generation was byte-identical on the characterized environment.

`tests/tables.rs` checks native layout against these independently generated
widths. Its 0.03 pt tolerance accounts for text-shaping quantization between the
engines; the original failures were unsupported rendering, a 113 pt excessive
cell width, and percentage columns stopping at 90% of the target width.
`tests/page_paint.py` separately checks actual PDF ink on either side of the
reference column boundary. No frozen oracle fixture or tolerance is changed.

`weasy70-rows.json` uses the same pinned engine sources and font. Its cases
record exact text baselines and page membership for ordinary rows, row/cell
avoidance, repeated groups, insufficient line space, widows/orphans and empty
fixed-height content. Regenerate with:

```sh
PYTHONDONTWRITEBYTECODE=1 python3 tests/fixtures/tables/make-weasy70-widths.py tests/fixtures/tables/weasy70-rows.json
```

The cases also cover table/group avoidance, multiple rows in one kept group,
separate kept groups, forced-break precedence, page versus column avoidance,
the initial frontier of a rowspan band and an oversized avoided group.
There are 16 exact pagination cases.

The `parallel_flows` observation retains WeasyPrint's actual rowspan output,
including its different placement of the following row. It is deliberately not
counted as exact WeasyPrint parity. CSS Fragmentation 3 permits independent cell
flows and optional balancing of unforced breaks. Separate layout and native PDF
tests verify ferri-pdf's independent row/column progression, complete content and
line geometry.

`weasy70-cell-edges.json` records 28 exact edge-fragmentation cases using the
same font and engine release, with additional hashes for `block.py`, `column.py`
and `inline.py`. Cases cover sliced padding and borders, initial and final line
capacity, nested content, columns, table ancestors, separate row extents and
percentage padding at different destination widths. Regenerate it with the same
generator, passing this file as the argument.

Ten additional `observations` retain actual reference output without claiming
identical optional break placement. They cover nested closing padding,
column packing, cloned edges and zero-gap table/ancestor boundaries. Separate
source-derived tests require complete content, correct cloned/final geometry
and native content-box ink. Existing width and row reference files remain
unchanged.
