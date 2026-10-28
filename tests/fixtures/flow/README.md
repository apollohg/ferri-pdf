# Independent flow-margin references

`weasy70-margins.json` contains 26 exact cases measured with WeasyPrint 70.0,
using the public bundled DejaVu Sans font. It pins the font and seven engine
source hashes, including page layout and the UA stylesheet. Cases cover initial
body and heading defaults, nested and mixed-sign collapse, root margins,
borders and padding, percentage margins, independent formatting contexts,
forced and automatic breaks, named-page transitions and destination widths.
All documents are synthetic.

Regenerate using the existing reference generator in a WeasyPrint 70.0 Python
environment:

```sh
PYTHONDONTWRITEBYTECODE=1 python3 tests/fixtures/tables/make-weasy70-widths.py tests/fixtures/flow/weasy70-margins.json
```

The generator verifies version, source and font hashes before measuring actual
reference-engine text boxes in PDF points. It imports no candidate renderer.
On macOS, the environment may require `DYLD_FALLBACK_LIBRARY_PATH` pointing to
the installed Pango/GLib libraries.

`tests/flow_margins.rs` checks exact page membership and baselines at the existing
0.03 pt shaping tolerance. `tests/named_pages_pdf.py` checks PDF text operators,
page membership and independently calculated background-ink boundaries.

One `observations` case preserves actual WeasyPrint output for automatic column
breaks: it retains the 12 pt top margin at every column. The source-derived
candidate tests instead preserve that margin only at the initial flow start,
then truncate margins adjoining automatic breaks to zero, as required by
[CSS Fragmentation 3 §5.2](https://www.w3.org/TR/2018/CR-css-break-3-20181204/#break-margins).
The observation is not counted as exact parity and is never overwritten with
candidate output.

## List break defaults

`weasy70-list-breaks.json` records 13 independently measured list-pagination
cases. The same pinned release and public font are used, with additional engine
hashes for outside-marker placeholders and box traversal. The line extractor
includes positioned placeholders, so the ordered-list case contains every
marker as well as its item text. All four earlier reference files regenerate
byte-identically with this extraction.

```sh
PYTHONDONTWRITEBYTECODE=1 python3 tests/fixtures/tables/make-weasy70-widths.py tests/fixtures/flow/weasy70-list-breaks.json
```

Cases cover both list types, author overrides and rollback, forced-break
precedence, wrapper propagation, a preceding block's final fragment, long
paragraphs, empty and nested lists, and default ordered markers. Layout and
native PDF checks reuse the existing exact-baseline helpers and tolerance.
The list rule is a chosen print UA default, rather than a normative CSS
initial value.

## Oversized avoidance

`weasy70-avoid.json` records 21 exact cases with the same pinned engine/font
and two retained observations. Regenerate with:

```sh
PYTHONDONTWRITEBYTECODE=1 python3 tests/fixtures/tables/make-weasy70-widths.py tests/fixtures/flow/weasy70-avoid.json
```

Cases distinguish avoided and ordinary blocks, legacy aliases, nested and
paragraph content, initial and positioned-only starts, empty spacers, forced
breaks, page-height changes, sliced/cloned decorations, columns and captions.
Native tests check the same exact baselines and complete painted glyph streams.
The positioned-only control follows CSS painting order when checking PDF
operators, while retaining the reference's box-tree order in the JSON.

The forced-descendant and bottom-caption observations retain actual WeasyPrint
output. Separate source-derived tests preserve a fitting forced-break prefix
and keep the caption's final line with its following paragraph. Neither
observation is counted as exact reference parity. Caption ink checks require
full continuing fragments and the final destination width/height.
