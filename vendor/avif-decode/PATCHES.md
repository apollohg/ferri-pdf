# Local changes to avif-decode 3.0.0

Upstream: https://github.com/kornelski/avif-decode (BSD-3-Clause; LICENSE retained).

- Replace the container-owning decoder and CLI with a reusable raw AV1 item API.
  The renderer's separately bounded container parser supplies colour/alpha items.
- Set explicit thread and frame-pixel limits. Disable rav1d assembly on every target.
- Decode with the `ferri-pdf-rav1d` fork, which returns AV1 frame decode errors
  instead of aborting the process.
- Bound drained pictures to AV1's four spatial layers; retain pictures through a
  cleanup guard so malformed input cannot leak a previously decoded layer.
- Expose the decoded sequence's colour primaries/transfer description.
- Declare Rust 1.97 after compiling and exercising this adaptation on 1.97.1.
  The original 1.98 manifest is retained as Cargo.toml.orig. No ignored-version
  flag is needed by the workspace.
- Remove the obsolete CLI and container-based tests/dependencies. Renderer tests
  cover actual AVIF files, independent pixel references and malformed inputs.

Pixel conversion remains upstream, including its nearest-neighbour chroma
upsampling. The renderer handles alpha planes, colour profiles and transforms.
