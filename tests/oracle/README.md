# HTML/CSS-to-PDF oracle

The oracle renders independently authored cases with the Rust CLI and checks the resulting PDFs against independent, source-backed output assertions. The public profile contains 15 synthetic cases and 18 locked benchmark workloads. Fixtures, fonts, assets, source locks and expected output assertions are included in the repository. External application data is not required.

The runtime remains Rust. Inspection requires Python, Pillow, fontTools, a local GNU FriBidi shared library and local Poppler commands (`pdftoppm`, `pdftotext`, `pdffonts`). WeasyPrint is optional and its absence is reported separately. Install FriBidi through the system package manager (for example, `brew install fribidi` or the distribution package providing `libfribidi.so.0`); `ORACLE_FRIBIDI_LIBRARY` can select an explicit shared-library path. The run records its path and native version. Missing FriBidi blocks RTL replacement-text analysis with an explicit error; it never skips the check.

## Running checks

From the repository root:

```sh
python3 tests/oracle/run.py --validate-manifests
python3 tests/oracle/run.py --inventory
python3 -m unittest discover -s tests/oracle -p 'test_*.py' -v
python3 tests/oracle/run.py --case paint-overlap --output-dir tmp/paint-check
python3 tests/oracle/run.py --require-conformance --output-dir tmp/full-profile
python3 tests/oracle/run.py --foundations --require-conformance --output-dir tmp/foundations
python3 tests/oracle/run.py --benchmark-suite --preflight-only --output-dir tmp/workload-preflight
python3 tests/oracle/run.py --benchmark-suite --output-dir tmp/workload-timings
```

`--binary PATH` uses a prebuilt CLI and records its build profile, lock state and features as unknown. A native executable is identified by its SHA-256, build receipt and source provenance.

`--weasyprint PATH` additionally renders each selected case with a local WeasyPrint executable and records a complete PDF comparison against the Rust output. The comparison is diagnostic; it never affects a gate.

Unfiltered `--require-conformance` includes every public case and locked workload. Filtered runs cannot claim full-profile acceptance. Gate exits are `1` for infrastructure, `3` for conformance and `130` for interruption. Diagnostic runs can exit successfully while recording renderer failures.

## Evidence and standards

`report.json`, `report.md` and `requirements.md` share execution data and link assertions to artifacts. Interrupted runs retain selected `not_run` cases. Execution states distinguish rendered, unsupported, rendering failures, timeouts and infrastructure failures. Assertion states distinguish pass, fail, unverified, blocked and not applicable. Exclusions never add passes. Implementation presence is separate from verification.

A case passes only when it rendered and every mandatory assertion passes. Each assertion cites a pinned normative source section or an independent reference with its hash, licence and origin. Evaluation checks every cited source hash and section against the loaded profile and rejects ambiguous bases. Unknown clauses, unresolved imports, unreviewed publication overlaps and missing assertions remain blockers. The source/clause inventory is not complete standards verification. Foundations reports its six groups and 139 behaviors separately; a scoped pass does not imply global conformance.

`sources.json` locks normative publications by immutable URL or demonstrated upstream revision and content SHA-256. Compact section indexes retain titles and text hashes; full bodies stay in the external cache. Webref and MDN metadata assist discovery and grammar generation but do not establish conformance to the selected publication. Normal tests never fetch source material.

Word inventories retain ordinary extracted word boundaries. If a reader returns a multiword replacement-text phrase as one box, its visual words are counted only when the painted glyphs corroborate its characters and word boundaries.

Logical `ActualText` is checked against its exact painted-glyph interval using independent FriBidi Unicode bidi ordering. PDF does not retain the paragraph base direction, so either resolved base direction may corroborate it. Whitespace and compatibility normalization follow the text comparison. Only a complete visual line with corroborated replacement text and matching logical or visual extracted text is normalized to painted order. Partial lines and multi-line spans retain the original extracted text. False replacement text fails text comparison even when Poppler exports the same text from both PDFs.

Tolerances and calibration evidence live in `manifests/defaults.json` and `assets/`. Every rendering run calibrates the tolerances against the committed calibration PDF and synthetic rasters; a changed or failed calibration blocks the gates. Never bless candidate output as expected output or widen tolerances to obtain a pass. Independent table references and their reproducible engine/font provenance are documented in [the table fixture guide](../fixtures/tables/README.md).

## Benchmark protocol

`static-documents-v2` fixes six classes: invoices, reports, books, multilingual text, tables and images. Each has three scales. All 18 workloads remain mandatory, including unsupported features. Complete word/value inventories, image identities and placements, SVG probes and font selection determine eligibility before timing.

Each eligible document runs three warmups and thirty timed runs, each a new CLI process with warmed filesystem caches. Timing includes startup, resource loading, parsing, layout, serialization and file writing. Inspection, tracing and a separate RSS measurement occur outside timing. Every PDF is verified. No failures are retried or trimmed; changed inputs, resources or executable hashes invalidate measurements. The report gives each document's median, minimum and maximum duration on the recorded machine; any missing or ineligible document blocks the suite summary.

## Public data boundary

New regression inputs must be independently authored minimal documents with generic content, bundled assets and portable paths. Do not copy client templates, business data, source locations or render artifacts into tests or documentation. Private acceptance archives preserve previous records locally without changing their bytes. They are excluded from the public profile and package, and cannot be treated as publicly reproducible evidence.

Removing files from the current tree does not remove them from Git history. A public source release must use a clean export or reviewed history cleanup; publishing the existing historical repository would retain older private material.

The PDF reader resolves named destinations through catalog `Dests` dictionaries (name objects) and `Names/Dests` trees (byte strings), including dictionary-wrapped explicit destinations. This follows ISO 32000-1 §12.3.2.3; the dictionary form is also clarified in the [PDF specification errata](https://pdf-issues.pdfa.org/32000-2-2020/clause12.html#12324-named-destinations). Missing destinations, malformed trees and reference cycles fail analysis instead of dropping navigation from comparison.
