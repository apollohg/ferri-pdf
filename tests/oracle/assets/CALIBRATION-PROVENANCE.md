# Independent PDF calibration input

`calibration.pdf` is a project-authored PDF fixture, independent of any renderer output. It has two 150 × 100 pt pages, each painting `Probe` in a 12 pt Helvetica font at (36, 36), with explicitly declared character advances. It contains no third-party font bytes.

The source constructor is `write_probe_pdf` in `tests/oracle/test_support.py`. Its variants deliberately remove a page or glyph, move text, change colour, clip text or cover it with later paint. The comparison tests inspect and rasterise those actual PDFs. The normal calibration routine copies the committed reference into the run directory and applies the existing geometry and ink checks there.

This file records project ownership and provenance; it adds no redistribution grant or new repository licence.
