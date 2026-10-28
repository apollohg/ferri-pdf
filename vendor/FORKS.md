# Maintained dependency forks

These packages have distinct `ferri-pdf-` Cargo identities. Rust dependency aliases
retain upstream crate names. Explicit path/version/package edges carry the forks
into downstream builds; consumers do not need root-level Cargo patches.

Original source licences and copyright notices remain in each directory. These
packages are not upstream releases despite retaining their upstream base versions.
`UPSTREAM_CARGO.toml`, where present, retains the upstream manifest.
`UPSTREAM_VCS.json` retains upstream revision metadata. Cargo reserves the original
`Cargo.toml.orig` and `.cargo_vcs_info.json` names for its generated archive files;
the provenance files are renamed byte-for-byte rather than discarded.
The root workspace excludes the fork directories except the existing XML test
member; it does not include `.` explicitly, which would pull path dependencies
back into automatic membership. The root package remains an implicit member.

| Directory | Cargo package | Base version | Licence |
|---|---|---|---|
| avif-decode | ferri-pdf-avif-decode | 3.0.0 | BSD-3-Clause |
| cssparser | ferri-pdf-cssparser | 0.37.0 | MPL-2.0 |
| cssparser-color | ferri-pdf-cssparser-color | 0.5.0 | MPL-2.0 |
| heic-decoder | ferri-pdf-heic-decoder | 0.1.1 | MIT OR Apache-2.0 |
| html5ever | ferri-pdf-html5ever | 0.39.0 | MIT OR Apache-2.0 |
| icu_casemap | ferri-pdf-icu-casemap | 2.3.0 | Unicode-3.0 |
| krilla | ferri-pdf-krilla | 0.8.2 | MIT OR Apache-2.0 |
| krilla-svg | ferri-pdf-krilla-svg | 0.8.1 | MIT OR Apache-2.0 |
| rav1d | ferri-pdf-rav1d | 1.1.0 | BSD-2-Clause |
| resvg | ferri-pdf-resvg | 0.47.0 | Apache-2.0 OR MIT |
| roxmltree | ferri-pdf-roxmltree | 0.21.1 | MIT OR Apache-2.0 |
| scraper | ferri-pdf-scraper | 0.27.0 | ISC |
| selectors | ferri-pdf-selectors | 0.38.0 | MPL-2.0 |
| usvg | ferri-pdf-usvg | 0.47.0 | Apache-2.0 OR MIT |
| zenavif-parse | ferri-pdf-zenavif-parse | 0.6.2 | MPL-2.0 |

Scraper, cssparser-color and resvg are newly copied from their published crate
archives solely to route shared parser/SVG types through the maintained forks.
Their implementation files are unchanged. Original registry archive checksums:

- `scraper 0.27.0`: `bdd0be4d296f048bfb06dd01bbc80ef789ddd2e55583e8d2e6b804942abfabc2`
- `cssparser-color 0.5.0`: `bbaa233e1dcd9c13a5d3e3a8a2c0f5a727bac380398345dbcb31db4597edc86b`
- `resvg 0.47.0`: `9be183ad6a216aa96f33e4c8033b0988b8b3ea6fd2359d19af5bac4643fd8e81`

The public consumer regression is `python3 tests/package_consumer.py`. It builds
an independent temporary project, adds no patches, and renders synthetic HTML,
selectors, text, raster and SVG content with default features enabled and disabled.
It uses the existing Cargo cache offline and reuses the root target directory.
Registry publication, archive verification and licence/provenance review remain
release gates; changing package identity alone does not establish those gates.
