# Local changes to usvg 0.47.0

Upstream: [linebender/resvg](https://github.com/linebender/resvg/tree/3a0fdba53ccf2d346b54cc53ba7adf0ee60d0707/crates/usvg), revision `3a0fdba53ccf2d346b54cc53ba7adf0ee60d0707`. The crates.io archive SHA-256 is `d46cf96c5f498d36b7a9693bc6a7075c0bb9303189d61b2249b0dc3d309c07de`. Both upstream licences, headers and source metadata are retained. `Cargo.toml.orig` records the upstream manifest; the active manifest additionally uses cssparser and the common DOM, with the local XML provider shared by both packages.

The selected [CSS Display 3 Appendix B](https://www.w3.org/TR/2026/CRD-css-display-3-20260605/#unbox-svg) removes principal formatting for renderable SVG containers, text-content child elements and `use`, including its instantiated content. Other SVG elements, including a document SVG root and a text root, compute `contents` to `none`.

Conversion now classifies this display before producing geometry or effects. Boxless groups use transparent internal grouping, retain original native source/selector ancestry and computed attributes, and omit their own transforms, opacity, filters, clips and masks. Nested SVG elements do not establish a viewport. `use` retains instantiation and referenced viewports while ignoring its own positioning and size overrides. Conditional processing of `switch` remains before child conversion.

Text children omit their own position, rotation, length/path layout, baseline shift and decoration. Real ancestor decoration and inherited presentation properties remain. Native text collection preserves outer text-path flow, can hoist a path through a boxless span, and skips hidden subtrees. Explicit child inheritance still reads the original computed attributes. The existing inheritance path now includes `transform`.

Declaration and rule-list tokenization uses cssparser, preserving escaped identifiers, comments, case-insensitive property names, `!important` and malformed-declaration recovery. Display keyword tokens are decoded before interpretation. Selector parsing and matching of original XML source nodes now use the same native DOM engine as HTML; SimpleCss and its separate matcher are removed. A source-qualified XML view preserves original ancestry, namespaces, name/attribute case, namespace declarations, text, comments and processing instructions. It uses XML document semantics even for foreign HTML-namespace elements. The view and its match caches are built only when stylesheet selector rules exist. Namespace bindings stay local to each stylesheet and its header; the namespace prelude parser is shared with the style crate. Use expansion now creates a distinct shadow fragment for each referenced subtree in that same view. Original XML nodes remain the source for attributes and reference resolution; instance nodes carry separate selector identity, ancestry, siblings and host relationships. The existing native href and cycle resolver drives expansion once. A scoped guard restores the enclosing context across nested uses and errors, including text descendants. Pointer-based match caches are cleared before the graph grows. The native SVG depth budget remains 1024, independently of HTML declarative-shadow depth; cumulative cloned XML nodes use the existing one-million-node budget and return the native typed node-limit error before attaching a denied instance. Host rules use the common matcher, with normal outer declarations (including presentation attributes) taking precedence and inner important declarations overriding outer important declarations. The backend no longer gives unconditional priority to the first important declaration when later sorted declarations compete. These changes do not complete the remaining common cascade, stylesheet applicability, external use references or author-font integration.

`Tree::from_str_with_image_resolver` receives original image references, including data URLs, synchronously after native styling and visibility processing. The callback need not be thread-safe; existing `Options` and resolver APIs retain their types. The caller can preserve resource policy, byte/depth budgets and typed errors without preloading inactive definitions or effects. Its optional viewport overrides used dimensions without changing authored root attributes or selector matches. Existing conversion paths use the same converter with no callback or viewport override. The images crate caches resolved resources by their original references and the initial converted tree, reusing the latter at its original viewport.

`src/images/tests/svg_contents.rs` verifies geometry, original selectors, computed inheritance, text layout, use instances and referenced viewports, conditional selection, keyword syntax/recovery, hidden roots, resource non-acquisition, typed denials, data budgets and resized resource reuse. These regressions do not certify full SVG/CSS support or independent browser equivalence. Broader stylesheet-source, cascade and font integration remains part of the open standards programme.

CSS component-depth exhaustion from the shared cssparser provider is retained separately from recoverable malformed CSS. Both stylesheet and inline declarations carry that condition to `Error::CssComponentDepthLimit` before native conversion. The common style parser and SVG structural validation use the same named budget; an unused custom-property value cannot bypass it or silently remove its containing rule.

Source-cluster boundaries reuse the common fonts helper used by document shaping.
Every glyph retains its complete source cluster, byte identity and em advance.
Adjacent successful fallback prefixes can re-form ligatures when a later cluster
needs a different face. Initial family provenance remains distinct from an
explicitly named family, including names that coincide with the UA default.

`FontProvider` and `Tree::from_str_with_resolvers` accept a synchronous borrowed font provider alongside the image callback. Existing `Options` and its Send/Sync callback types retain their interfaces. Fallback receives the original family/style request; variation resolution can apply selected-face descriptors before explicit axis settings. The parser preserves that explicit provenance. Shaping and output spans use the same per-face coordinates; coordinate changes split spans while retaining decoration ordering. Missing clusters retain already covered glyphs, use source bidi/context when reshaped, respect the owning span, and group adjacent missing clusters only when they select the same fallback face. This preserves fallback ligatures and family priority without assuming identical glyph counts. The embedding images adapter provides stable job-local font identities and lazy byte access, including nested SVGs. Full native SVG text/cascade equivalence remains unverified.

The module is compiled inside `ferri-pdf` with the `text` feature always enabled. The
command-line binary, the SVG writer (`Tree::to_string` and `WriteOptions`) and the
system-font and memory-mapping features are removed. COLR painting reports unsupported
sweep gradients and blend modes through `log` instead of printing to standard output.

`font-weight` takes any number from 1 to 1000 and the CSS Fonts 4 §2.2.1 relative weights,
`font-stretch` any percentage (§2.3) and `font-style: oblique` an angle (§2.4), so font
matching receives the weight, width and slant the document's cascade computed.

`feColorMatrix type="saturate"` keeps values above one, which oversaturate (Filter Effects 1
§9.6), instead of clamping them to one as SVG 1.1 did.

`direction: rtl` gives each text chunk a right-to-left bidi base level and swaps its `start` and
`end` anchors (SVG 2 §11.10.1.1, §11.10.2.4), so right-to-left documents place SVG text from its
anchor leftwards.

`transform-box: fill-box` and `stroke-box` on groups, shapes and `use` elements place
`transform-origin` in the element's object or stroke bounding box before its transform applies
(CSS Transforms 1 §2); other values keep the viewport as the reference.

`overflow` takes one keyword per axis, and a viewport clipped along one axis only spans its
content's extent along the other (SVG 2 §3.11).

`clip-path` takes CSS Shapes 1 §3 basic shapes and bare `<geometry-box>` keywords as well as
references (CSS Masking 1 §6.1). The document's computed basic shapes trace the outline in the
element's fill, stroke or view box, `stroke-box` by default, so SVG content and CSS boxes clip
alike.

`url()` and `href` references into other SVG documents (SVG 2 §5.3.2) load through a caller's
synchronous document callback, which keeps the resource policy, caching and typed errors. Each
referenced document, and the documents it references in turn, is parsed with its own style
sheets and appended as a second, unrendered root element, its ids and internal references
prefixed apart from the referencing document's, so paint servers, markers, clip paths, masks,
filters and offset paths resolve in the merged tree. A document that does not load or parse
leaves its references naming no element, so each property's invalid-reference fallback applies.
Image `href`s of a referenced document resolve against that document's URL.

`vector-effect` (SVG 2 §8.13, §13.6) takes effect: `non-scaling-stroke` strokes paths, and the
flattened outlines of text, in the host coordinate space while their paint stays in user space,
and `non-scaling-size`, `non-rotation` and `fixed-position` constrain the element's transform
by the §8.13.1 formulas.

`color-interpolation: linearRGB` on a gradient interpolates its stops in linear light (SVG 2
§13.9): stops are added where the sRGB curve of that interpolation departs from a straight line
by more than a quarter of an 8-bit step.

`mix-blend-mode` takes the Compositing 2 §9.1.14–§9.1.15 `plus-darker` and `plus-lighter`
operators; `feBlend` modes do not. `color-interpolation: linearRGB` marks groups whose layer and
direct graphics children composite in linear light (SVG 2 §13.9): an element whose colour space
differs from its parent's gets its own group, and text spans carry theirs into the flattened
outlines. Clipping path children keep neither. The outermost `svg`, `use` elements, markers and
clipped nested viewports, which SVG 2 §3.4.1 makes stacking contexts, are isolated where content
in them blends or composites in linear light.

Text content elements the document styles name their computed style, in the options'
`styling`, through `ferri-pdf-text-style`. A standalone document's elements compute theirs as
the document computes its own, from their presentation attributes and the declarations of
properties without attributes that won their own cascade, which the tree keeps per element,
with `lang` and the SVG 1.1 `kerning` attribute; a value SVG content cannot take fails the
conversion. Text shapes with that style as
the document's text does: the CSS Fonts 4 §6–§7 feature settings in precedence, with each face's
`@font-face` features and `@font-feature-values` alternates, capitals and positions native or
synthesized, the language system of `font-language-override` or the language, and the used size
of `font-size-adjust`. `text-transform` cases each chunk's text before shaping, and the glyphs map
back to the characters SVG 2 §11.5 positions. `text-shadow` layers paint each span's outlines and decorations opaquely in
the shadow's colour beneath the text, offset and blurred.

`white-space` sets the `white-space-collapse` of text, else `xml:space` does: preserved segment
breaks start line boxes stacked by the line heights of their spans and the text element's strut
(SVG 2 §11.6.1), and preserved tabs advance to tab stops of the text element's spaces.

Percentage baseline shifts of styled text refer to its used line height (CSS Inline 3 §4.2.3),
and `alignment-baseline` aligns a box's baseline with the same baseline of its parent. `sub` and
`super` shift by halves of the parent's descent and ascent, and the `vertical-align` longhand
aligns boxes as the document's line boxes do: its shifts add to `baseline-shift`, its keywords
align layout bounds, and `top` and `bottom` align a subtree with its line box's edges, growing
the line downwards when taller.

`inline-size`, the computed `width` or `height` of styled text along its inline axis, gives it
a wrapping area (SVG 2 §11.4): each paragraph breaks into lines where the document's line
breaking allows, with its hyphenation, emergency wrapping and `text-indent`, and each line
starts a line box anchored by `text-anchor` in the area. Positioning attributes then give only
the initial position, `textLength` does not apply, and `text-overflow` clips or ellipsizes
overflowing lines at the area's edges (§11.10.2.10).

`text-emphasis` marks each marked character of styled text as the document's text does: the
mark string in the span's font at half size with the `ruby` feature, or the shape no font has
synthesized, filled in `text-emphasis-color` over or under the content area, upright beside
vertical columns. Both renderers paint the marks as outlines, not text.

Decoration lines take the style, thickness, underline offset and position, and error
indications the declaring element's computed style gives them, placed and shaped as the
document's text places and shapes its lines; a `text-decoration-color` other than
`currentcolor` fills them in place of the declaring element's fill.

`overflow` on a pattern renders each tile's overflowing content over the tiles it reaches, rows
top to bottom and tiles left to right, and on a sliced image the parts beyond its rectangle.

`width` and `height` take `calc-size()` and the intrinsic size keywords (CSS Values 5 §10.2,
CSS Sizing 3 §3.1). Keywords, and `calc-size()` with a keyword basis, behave as `auto`; the
latter then calculates the used size from the automatic size each element gives `auto` (SVG 2
§7.8): none for a rectangle, `100%` for a viewport and the natural size for an image.

A `mask` value that `crate::svg::layer_mask_value` writes names the CSS mask layers, in the
options' `styling`, of SVG content the document styles. Its group keeps them as a
`LayerMask` with its fill, stroke and view boxes, the boxes `clip-path` basic shapes also take,
in points.

CSS Transforms 2 `perspective`, `perspective-origin`, `transform-style` and
`backface-visibility` style SVG content (§11). A transform whose `w` varies across the element,
or one placing a plane of a 3D rendering context, gives its group a 3D `matrix` in place of its
affine transform; bounding boxes map through its projection. Once converted, each group whose
used `transform-style` is `preserve-3d` outside a context becomes a scene whose children are the
context's planes, placed by their accumulated matrices. A `clipPath` ignores a transform list
with 3D functions, and `non-scaling-stroke` has no effect within a 3D rendering context.

Each face the provider gives styled text paints its glyphs as the document's text paints them:
COLR glyphs take the palette `font-palette` and `@font-palette-values` select (CSS Fonts 4
§9.1), and a face lacking the requested weight or slant, where `font-synthesis` allows, draws
each glyph a second time 0.02em along its inline axis and shears it about its baseline (§5.2,
§5.4). The face's synthesis also ranks faces as the document ranks them. `font-variant-emoji`
appends a presentation selector to each emoji its text does not select one for (§9.3); a face
lacking a variation sequence of a cluster gives way to a later face that has it, else keeps the
cluster, and selectors no sequence takes leave no glyph. COLR glyphs convert to SVG through
skrifa, at the span's variation coordinates, with the palette's entries and the fill colour,
else the stroke colour, as the foreground.

Each text chunk's bidi paragraph is resolved as the document's paragraphs are: its spans sit
in the inline boxes of their ancestors, so `unicode-bidi` on the text element and its tspans
embeds, isolates, overrides or sets a plaintext base direction (CSS Writing Modes 4 §2.4, SVG 2
§11.10.2.4), and upright vertical text and combinations take their overrides. In vertical
text, each `text-combine-upright` combination composes horizontally in its span's style with
the width-specific forms every unit has, compressed to fit one em and centred upright in one
cell of the column as a single cluster, whose glyphs keep their own characters (CSS Writing
Modes 4 §9.1). The SVG 2 §6.6 `unicode-bidi` presentation attribute takes part in the cascade.

Decoration lines skip as the document's do (CSS Text Decoration 4 §2.10): `text-decoration-skip`
keeps them from spacers, or from those starting or ending a chunk, with the spacing before them;
`text-decoration-inset` and `text-decoration-skip: edges` start them inside the declaring
element's content edges, where its first and last characters lie; and
`text-decoration-skip-ink: all` interrupts underlines and overlines where glyph outlines cross
their band, with the document's clearance.

`text-autospace` spaces each boundary between visually adjacent clusters of a chunk as the
document's lines do (CSS Text 4 §8.4): by the value of the innermost element containing it,
measured in that element's font, removing a single space between them where it replaces one.
Spans keep the font and size of each element containing them for this.
