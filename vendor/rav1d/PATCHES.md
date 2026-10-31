# Local changes to rav1d 1.1.0

Upstream: https://github.com/memorysafety/rav1d, revision
`782dab2135ea64a057c097088a13eb8ed3cc3320` (BSD-2-Clause; COPYING retained).
Source: the published crates.io archive, SHA-256
`1932f060d5e7bd49dc9f8b272c1dc5e9ce0ffe141c28be900265d3989b36c9ed`.
Modified file: src/decode.rs.

- `rav1d_submit_frame`'s `on_error` no longer unwraps the frame header. When
  `rav1d_decode_frame` fails with a single frame context, `rav1d_decode_frame_exit`
  has already cleared the header (and released the output CDF), so the unwrap
  panicked inside `extern "C" fn dav1d_send_data` and aborted the process on
  corrupt AV1 tile data. The decode error is now returned. Upstream made the same
  change in commit `6c6f463785a546dea18fd9ca0f14634c2ee0557c`, not yet released.
- Default features no longer enable assembly. The published 1.1.0 archive omits
  the aarch64 assembly headers (upstream issue #1449), so its default build fails
  on aarch64, and x86 assembly needs NASM. The renderer disables assembly anyway;
  the `asm*` features remain available.
- The package excludes upstream's CI configuration, coverage configuration,
  retranspile script and nightly `rust-toolchain.toml`; none affect the build.

The crate keeps upstream's own `unsafe` code and lints; the renderer's
`unsafe_code = "forbid"` applies only to its own crate.
