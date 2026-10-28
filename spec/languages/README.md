# Language matching data

`aliases.rs` is generated from the [IANA Language Subtag Registry](https://www.iana.org/assignments/language-subtag-registry/language-subtag-registry), File-Date 2026-09-17, SHA-256 `755fad43283be7b41ebe3c89ad054b6eaf928f404f9c0edb74799e0eab74beb1`.

Regenerate using `python3 spec/languages/generate.py tmp/language-subtag-registry-2026-09-17.txt`. The generator rejects any other input hash. The tables contain registry facts: preferred values and extlang prefixes. Rendering performs no language-data download or host-locale lookup.

Selectors use RFC 5646 §4.5 canonicalization and extlang form before RFC 4647 §3.3.2 extended filtering. This registry snapshot supplies the data that those selected normative algorithms require; it does not replace the frozen specification publications or benchmark corpus.
