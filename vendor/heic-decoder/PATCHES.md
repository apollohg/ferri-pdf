# Local decoder changes

Source: `heic-decoder` 0.1.1 from crates.io, repository
https://github.com/Multipad-cyber/heic-decoder-rust (MIT OR Apache-2.0).
The upstream licence files are retained. Published crate SHA-256:
`61c012c388f9a8e5f2e611d67cfca513d4735fd298b648a9c5a61859e34d9bcf`.

The container parser now shares a 64 MiB expansion budget across the primary
image, alpha image, repeated item extents, repeated tile configurations, extent
records and NAL records. Checks precede their allocations. The sequence path
shares the same budget for configuration records and sample bytes. Regression tests cover repeated extents/items and zero-width extent/NAL records.
Sample NAL splitting rejects missing headers and independently bounds cumulative
NAL payload and record allocations to 64 MiB before storing them.
This bounds container expansion, not total process memory or HEVC working storage.

`decode_parsed` allows the renderer to validate coded pixel counts after parsing
and then decode that same container, avoiding a second parse and copy.

The standalone Cargo workspace retains upstream unit tests. Its unused Criterion
dev dependency was removed because the published package includes no benchmarks.

Validation: `cargo test --manifest-path vendor/heic-decoder/Cargo.toml --lib`.
