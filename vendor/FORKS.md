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
The root workspace excludes the fork directories; it does not include `.`
explicitly, which would pull path dependencies back into automatic membership.
The root package remains an implicit member.

| Directory | Cargo package | Base version | Licence |
|---|---|---|---|
| avif-decode | ferri-pdf-avif-decode | 3.0.0 | BSD-3-Clause |
| heic-decoder | ferri-pdf-heic-decoder | 0.1.1 | MIT OR Apache-2.0 |
| icu_casemap | ferri-pdf-icu-casemap | 2.3.0 | Unicode-3.0 |
| krilla | ferri-pdf-krilla | 0.8.2 | MIT OR Apache-2.0 |
| selectors | ferri-pdf-selectors | 0.38.0 | MPL-2.0 |
| zenavif-parse | ferri-pdf-zenavif-parse | 0.6.2 | MPL-2.0 |

## Inlined modules

These upstream libraries are compiled as private modules of `ferri-pdf` rather than
as separate packages, because they share its DOM and font types. Each module directory
keeps the upstream licences and provenance files. Binaries, examples, disabled Cargo
features and unused entry points (such as the usvg SVG writer) are removed. Each
module's `PATCHES.md`, where present, lists its local changes.

| Module | Upstream crate | Base version | Licence |
|---|---|---|---|
| src/svg/krilla_svg | krilla-svg | 0.8.1 | MIT OR Apache-2.0 |
| src/svg/resvg | resvg | 0.47.0 | Apache-2.0 OR MIT |
| src/svg/usvg | usvg | 0.47.0 | Apache-2.0 OR MIT |

Resvg is copied from its published crate archive solely to route shared SVG types
through the maintained usvg module. Beyond inlining (module paths, visibility, the
always-enabled `raster-images` feature), its IIR blur views pixels through
`bytemuck` instead of the deprecated `rgb::ComponentSlice`. Original registry
archive checksum:

- `resvg 0.47.0`: `9be183ad6a216aa96f33e4c8033b0988b8b3ea6fd2359d19af5bac4643fd8e81`

The public consumer regression is `python3 tests/package_consumer.py`. It builds
an independent temporary project, adds no patches, and renders synthetic HTML,
selectors, text, raster and SVG content with default features enabled and disabled.
It uses the existing Cargo cache offline and reuses the root target directory.
Registry publication, archive verification and licence/provenance review remain
release gates; changing package identity alone does not establish those gates.
