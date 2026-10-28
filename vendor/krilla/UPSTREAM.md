# Local Krilla source patch

This directory replaces the registry dependency through the root Cargo patch. The engine's ten crates remain separate workspace members; this dependency is excluded from that workspace. Only this copy is compiled.

Source: Krilla 0.8.2, copied from the resolved crates.io package, associated with upstream commit `3ffdf0588cf98050aad6edba51ca70162e1fb5b5`. `UPSTREAM.json` records the package checksum and original file hashes. MIT, Apache and upstream notices are preserved; the MPL text accompanies the adapted resvg function acknowledged in `NOTICE.md`.

The local patch changes these source files:

- `graphics/image.rs`: GIF decoding composites the first frame onto the logical screen at its frame offset, filled with the background colour unless the frame is transparent. Upstream embeds the bare frame, so offset or partial first frames lose their canvas size and position.

- `surface.rs`: scoped `ActualText` replacement spans work without a tagged-PDF structure tree. A guard closes the span on normal returns, errors and unwinding.

- `text/glyph/mod.rs` and `text/group.rs`: an optional, default-false Glyph hook
  retains Unicode mappings for every glyph in a shared source cluster. The SVG
  adapter uses it when batching multi-glyph clusters; ActualText retains the
  logical source once. Other Glyph implementations retain upstream behaviour.

- `serialize.rs` and `graphics/image.rs`: optional validated JPEG quality is
  applied in the existing deferred image serialization, after source deduplication.
  Default JPEG streams pass through unchanged. Re-encoding retains source
  dimensions, colour model and ICC metadata, including inverted Adobe CMYK and
  YCCK planes. Ordinary CMYK does not subsample ink channels. Non-JPEG sampled
  representations retain their existing lossless encoding, alpha and bit depth.
  The manifest adds jpeg-encoder 0.7.1 with only its std feature; its original
  `(MIT OR Apache-2.0) AND IJG` terms apply. No assembly, SIMD feature or native
  build dependency is enabled.

- `content.rs`, `stream.rs`, `page.rs`, `graphics/{image_usage,mask,xobject}.rs`
  `text/type3.rs` and `serialize.rs`: optional DPI tracks composed raster-use transforms across
  shared streams and pages, then sizes deferred payloads for their largest use.
  Derived usage metadata does not affect stream identity. Traversal is iterative
  and bounded to one million nodes. Images used inside glyph streams retain
  source resolution even when shared with smaller document images.
- `graphics/image/resize.rs`: bounded, alpha-associated Catmull-Rom resizing uses
  the existing image 0.25.10 dependency without default features. Eight- and
  sixteen-bit samples retain their depth, colour model and ICC metadata.
  `error.rs` reports the traversal resource limit. Resampling rejects working
  storage above 512 MiB per image.
- JPEG re-encoding averages chroma samples and applies the Annex K luminance
  table to all ordinary CMYK ink components, using the requested quality scale.

Canvas allocation uses checked arithmetic. Every other recorded upstream file matches the registry package.
