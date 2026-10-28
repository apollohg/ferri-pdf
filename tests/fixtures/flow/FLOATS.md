# Float geometry reference

`weasy70-floats.json` contains independently authored public HTML/CSS and measurements from WeasyPrint 70.0. It pins the font and relevant engine source hashes. It supplies independent geometry acceptance for ordinary floats. `src/tests/floats.rs` also checks source-required behaviour where the reference engine differs, fragmentation across changing page widths, clearance and source preservation. Native `page_paint.py` and `navigation_pdf.py` checks cover actual PDF paint, annotations and outlines.

Regenerate using the existing reference tool in an environment with the pinned engine and native libraries:

```sh
python tests/fixtures/tables/make-weasy70-widths.py tests/fixtures/flow/weasy70-floats.json
```

`line-geometry` records text-box x coordinates, widths and baselines in points. Cases use unique labels per page, allowing the shared Rust reference checker to require each label exactly once. The tolerance remains the existing 0.03pt; no frozen prior reference or tolerance was changed.

The `cases` group covers side bands, opposing floats, same-side wrapping, negative clearance, shrink-to-fit sizing, enclosure, shared and nested formatting contexts, relative painting offsets, float clearance, oversized widths, unbreakable text, fragmented floats and columns.

## Source review and observations

The selected CSS 2.1 [float rules](https://www.w3.org/TR/2011/REC-CSS2-20110607/visuren.html#floats) state that nonpositive outer-height floats do not shorten lines. The engine's zero-height and negative-outer-height samples shorten the first line. Those outputs are retained under `observations`, not imposed on this implementation. Explicit synthetic assertions must use the source rule.

The same source permits a new block formatting context to move below a float or fit alongside it. `bfc-beside-float` records WeasyPrint's below-float choice. It is an observation because the implementation design permits adjacent placement when the box fits; it does not establish a conformance requirement to match that engine's choice.

[CSS Fragmentation 3 parallel flows](https://www.w3.org/TR/css-break-3/#parallel-flows) requires independent fragmentation of float contents and surrounding content. WeasyPrint's `parallel-forced-break` observation omits the `FloatNext` text entirely. Do not adopt that output as acceptance. The implementation must retain the float continuation on page 2 while both surrounding body lines remain on page 1.

Reference output is checked against source requirements before promotion from observations. Matching WeasyPrint alone does not establish standards conformance, nor does this collection cover the complete float contract.
