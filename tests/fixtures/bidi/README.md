# Bidirectional text references

`character-reference.json` selects eight records from Unicode 17.0.0's
[BidiCharacterTest.txt](https://www.unicode.org/Public/17.0.0/ucd/BidiCharacterTest.txt).
Each entry retains its source line, exact record, paragraph direction, logical
characters and reference visual order. Balanced explicit controls are translated
into CSS scopes using the table in
[CSS Writing Modes 3 §2.4.2](https://www.w3.org/TR/2019/REC-css-writing-modes-3-20191210/#bidi-control-codes).
The cases exercise nested overrides, embeddings, directional isolates and
first-strong isolates under different paragraph directions. The four punctuation
mirror mappings come from independently pinned
[BidiMirroring.txt](https://www.unicode.org/Public/17.0.0/ucd/BidiMirroring.txt).
Both files' SHA-256 hashes and the retained Unicode licence are recorded in JSON.

Regenerate with downloaded copies of those exact files:

```sh
PYTHONDONTWRITEBYTECODE=1 python3 tests/fixtures/bidi/make-character-reference.py BidiCharacterTest.txt BidiMirroring.txt
```

The generator rejects different source hashes or unbalanced selected controls.
It derives logical and visual sequences directly from Unicode's records without
importing the renderer or a candidate PDF. Reference visual strings retain the
original characters; the native PDF test applies the independent mirror mapping
when the official embedding level is odd.

`tests/text_bidi.rs` checks the public rendering pipeline against those records.
`tests/bidi_pdf.py` checks actual painted CIDs, embedded glyph outlines and advances,
text matrices, visual ordering and exact logical `ActualText`. It compares against
the unchanged bundled DejaVu Sans font, whose hash it checks. Additional native
cases cover forced breaks, generated and slotted content, and opacity groups.
The native checks also distinguish ISO-8859-8 visual ordering from its logical
counterpart and verify author overrides and origin rollback against the same
original font.
Run after building the default CLI:

```sh
PYTHONDONTWRITEBYTECODE=1 python3 tests/bidi_pdf.py
```

ASCII override cases also pass exact `pdftotext -raw` extraction. Poppler applies
another bidi pass when exporting complex mixed-script `ActualText`, so that
export can differ from the encoded source. The native verifier therefore decodes
the PDF's replacement strings directly for all cases while separately checking
the complete painted glyph sequence. It does not adjust the logical text to
compensate for an exporter's reordering.
