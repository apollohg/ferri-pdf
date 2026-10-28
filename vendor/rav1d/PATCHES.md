# Local changes to rav1d 1.1.0

Upstream: https://github.com/memorysafety/rav1d (BSD-2-Clause; COPYING retained).

The manifest makes `cc` and `nasm-rs` optional build dependencies activated only by
`asm`. The renderer enables only `bitdepth_8` and `bitdepth_16`. Decoder sources and
the build script are unchanged; no assembly is built. The dependency audit checks
the exact build-script digest and enabled features, and still rejects active
native build tools. Upstream sources for other feature configurations are retained.
