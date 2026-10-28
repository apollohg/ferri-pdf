# Local changes to zenavif-parse 0.6.2

Upstream: https://github.com/imazen/zenavif-parse (MPL-2.0; LICENSE retained).
Modified file: src/lib.rs.

- Expose per-tile alpha associations and separate alpha-grid tiles, preserving
  dimg reference order and premultiplication flags.
- Expose colour and alpha transformations in their individual association order.
  Correct the mirror-direction documentation (0: top/bottom; 1: left/right).
- Index auxiliary image properties and references once, avoiding repeated nested
  scans when resolving colour/alpha tiles and depth items.
- Prefer an associated ICC profile over NCLX regardless of property order.
- Accept AVIF compatibility brands as well as AVIF major brands.
- Bound item-data expansion, iloc extent records, copied tile extent records,
  and associated ICC/auxiliary property copies. Check concatenated extent sizes
  before allocating the output buffer. Fixed local ceilings are 64 MiB of item
  data/property expansion and 64 MiB worth of extent records.

The renderer reads the actual grid descriptor rather than relying on inferred
zero-valued grid dimensions, and separately bounds aggregate coded pixels and
frame counts. These bounds are not a process-wide memory accounting mechanism.
