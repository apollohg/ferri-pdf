# Local changes to krilla-svg 0.8.1

`SOURCES.json` records the original registry checksum, upstream revision and file
hashes. The MIT and Apache 2.0 licences are retained unchanged.

Text rendering consumes native source-cluster identities and em advances from
the patched usvg. Compatible glyphs sharing a source cluster are painted in one
text operation, retaining each glyph's positioned origin and advance. This lets
krilla emit one logical ActualText cluster while embedding every painted glyph
with a Unicode mapping. The optional Glyph mapping hook defaults to the upstream
behaviour for other callers. Incompatible per-glyph transforms retain individual
painting and an enclosing ActualText span. Fill/stroke order, invisible painting
and paint transforms use the same helper for both paths.

`tests/font_geometry.py` independently checks fallback ligatures and a two-glyph
combining cluster, including source outlines, relative HarfBuzz positions and
Poppler extraction. These changes do not establish complete SVG text conformance.

Glyph painting keeps positioned origins in the enclosing SVG coordinate system.
The equivalent affine transform and inverse paint transform preserve visible
geometry while allowing enclosing replacement-text spans to retain their
selection coordinates across separate glyph operations.

The module is compiled inside `ferri-pdf`. Font bytes come from the in-memory face
source that `ferri-pdf` registers, so the unsafe memory-mapping accessor and the
copy-on-write font database clone are not used. The unused SVG glyph renderer is
removed.

Groups blending by `plus-darker` or `plus-lighter`, and translucent paint composited in linear
light, have no PDF blend mode. Their stacking context's backdrop is rendered with resvg up to
the node and up to and including it; the difference is drawn over the vector backdrop as the
colour that source-over compositing turns into the composite. resvg composites those operators
and linear light per pixel. Rasters resolve at the caller's image resolution.

Groups with CSS mask layers or a mask border (CSS Masking 1 §7, §8), which only SVG content
the document styles has, mask through the settings' `LayerMasks`: the document draws the
layers' values as it does for its own boxes, in the group's fill, stroke and view boxes. The
resvg calls behind compositing and filters pass it on, and resvg applies the same values in
pixels.

Groups with a 3D `matrix` draw as the document draws projected layers: text clusters through
the projection's tangent within the resampling tolerance, groups holding such text or blending
with what lies below them in place, and other content rendered by resvg in the plane's bands
and resampled. Scenes draw the depth-sorted pieces of their planes. resvg renders both in
pixels the same way.

Glyphs draw from a font instance with the COLR palette the face's `font-palette` selects, and
the copy synthetic bold adds draws after each cluster.
