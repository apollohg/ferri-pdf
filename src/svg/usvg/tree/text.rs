// Copyright 2018 the Resvg Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use std::sync::Arc;

use strict_num::NonZeroPositiveF32;
pub(crate) use svgtypes::FontFamily;

use crate::svg::usvg::layout::Span;
use crate::svg::usvg::{
    Fill, Group, NonEmptyString, NonZeroRect, PaintOrder, Rect, Stroke, TextRendering, Transform,
};

/// The `font-stretch` keyword of each CSS Fonts 4 §2.3 width, for the database's own query.
const STRETCH_KEYWORDS: [(crate::fonts::FontWidth, fontdb::Stretch); 9] = [
    (
        crate::fonts::FontWidth::ULTRA_CONDENSED,
        fontdb::Stretch::UltraCondensed,
    ),
    (
        crate::fonts::FontWidth::EXTRA_CONDENSED,
        fontdb::Stretch::ExtraCondensed,
    ),
    (
        crate::fonts::FontWidth::CONDENSED,
        fontdb::Stretch::Condensed,
    ),
    (
        crate::fonts::FontWidth::SEMI_CONDENSED,
        fontdb::Stretch::SemiCondensed,
    ),
    (crate::fonts::FontWidth::NORMAL, fontdb::Stretch::Normal),
    (
        crate::fonts::FontWidth::SEMI_EXPANDED,
        fontdb::Stretch::SemiExpanded,
    ),
    (crate::fonts::FontWidth::EXPANDED, fontdb::Stretch::Expanded),
    (
        crate::fonts::FontWidth::EXTRA_EXPANDED,
        fontdb::Stretch::ExtraExpanded,
    ),
    (
        crate::fonts::FontWidth::ULTRA_EXPANDED,
        fontdb::Stretch::UltraExpanded,
    ),
];

/// The keyword width nearest `width`.
pub(crate) fn nearest_stretch(width: crate::fonts::FontWidth) -> fontdb::Stretch {
    STRETCH_KEYWORDS
        .iter()
        .min_by(|a, b| {
            (a.0.value() - width.value())
                .abs()
                .total_cmp(&(b.0.value() - width.value()).abs())
        })
        .map_or(fontdb::Stretch::Normal, |(_, stretch)| *stretch)
}

/// A font variation axis setting.
///
/// Used for variable fonts to specify axis values like weight, width, etc.
#[derive(Clone, Copy, Debug)]
pub struct FontVariation {
    /// The 4-byte axis tag (e.g., b"wght" for weight).
    pub tag: [u8; 4],
    /// The axis value.
    pub value: f32,
}

impl FontVariation {
    /// Creates a new font variation.
    pub fn new(tag: [u8; 4], value: f32) -> Self {
        Self { tag, value }
    }
}

impl PartialEq for FontVariation {
    fn eq(&self, other: &Self) -> bool {
        self.tag == other.tag && self.value.to_bits() == other.value.to_bits()
    }
}

impl Eq for FontVariation {}

impl std::hash::Hash for FontVariation {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.tag.hash(state);
        self.value.to_bits().hash(state);
    }
}

/// A font style property.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum FontStyle {
    /// A face that is neither italic not obliqued.
    Normal,
    /// A form that is generally cursive in nature.
    Italic,
    /// An italic slanted clockwise.
    Left,
    /// An italic slanted counter-clockwise.
    Right,
    /// A typically-sloped version of the regular face.
    Oblique,
}

impl Default for FontStyle {
    #[inline]
    fn default() -> FontStyle {
        Self::Normal
    }
}

impl From<fontdb::Style> for FontStyle {
    fn from(style: fontdb::Style) -> Self {
        match style {
            fontdb::Style::Normal => FontStyle::Normal,
            fontdb::Style::Italic => FontStyle::Italic,
            fontdb::Style::Oblique => FontStyle::Oblique,
        }
    }
}

impl From<FontStyle> for fontdb::Style {
    fn from(style: FontStyle) -> Self {
        match style {
            FontStyle::Normal => fontdb::Style::Normal,
            FontStyle::Italic | FontStyle::Left | FontStyle::Right => fontdb::Style::Italic,
            FontStyle::Oblique => fontdb::Style::Oblique,
        }
    }
}

/// Text font properties.
#[derive(Clone, Debug)]
pub struct Font {
    pub(crate) families: Vec<FontFamily>,
    pub(crate) default_family: bool,
    pub(crate) style: FontStyle,
    /// The angle of an `oblique` style.
    pub(crate) slant: crate::fonts::FontSlant,
    pub(crate) width: crate::fonts::FontWidth,
    pub(crate) weight: crate::fonts::FontWeight,
    pub(crate) variations: Vec<FontVariation>,
    pub(crate) explicit_variations: Vec<FontVariation>,
    /// The bold and oblique `font-synthesis` allows, which also rank faces (CSS Fonts 4 §5.2).
    pub(crate) synthesis: crate::fonts::FontSynthesis,
}

impl PartialEq for Font {
    fn eq(&self, other: &Self) -> bool {
        self.families == other.families
            && self.default_family == other.default_family
            && self.style == other.style
            && self.numbers() == other.numbers()
            && self.variations == other.variations
            && self.explicit_variations == other.explicit_variations
            && self.synthesis == other.synthesis
    }
}

impl Eq for Font {}

impl std::hash::Hash for Font {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.families.hash(state);
        self.default_family.hash(state);
        self.style.hash(state);
        self.numbers().hash(state);
        self.variations.hash(state);
        self.explicit_variations.hash(state);
        self.synthesis.hash(state);
    }
}

impl Font {
    /// A list of family names.
    ///
    /// Never empty. Uses `crate::svg::usvg::Options::font_family` as fallback.
    pub fn families(&self) -> &[FontFamily] {
        &self.families
    }
    /// Whether the family came from parser defaults rather than the source document.
    pub fn uses_default_family(&self) -> bool {
        self.default_family
    }

    /// A font style.
    pub fn style(&self) -> FontStyle {
        self.style
    }

    /// The angle of an `oblique` style.
    pub fn slant(&self) -> crate::fonts::FontSlant {
        self.slant
    }

    /// A font width.
    pub fn width(&self) -> crate::fonts::FontWidth {
        self.width
    }

    /// A font weight.
    pub fn weight(&self) -> crate::fonts::FontWeight {
        self.weight
    }

    /// The bits of the slant, width and weight, which identify them exactly.
    fn numbers(&self) -> [u32; 3] {
        [self.slant.value(), self.width.value(), self.weight.value()].map(f32::to_bits)
    }

    /// Font variation settings for variable fonts.
    pub fn variations(&self) -> &[FontVariation] {
        &self.variations
    }
    /// Axis settings supplied explicitly, before standard font-property mappings.
    pub fn explicit_variations(&self) -> &[FontVariation] {
        &self.explicit_variations
    }
    /// The synthesis `font-synthesis` allows.
    pub(crate) fn synthesis(&self) -> crate::fonts::FontSynthesis {
        self.synthesis
    }
}

/// A dominant baseline property.
#[allow(missing_docs)]
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum DominantBaseline {
    Auto,
    UseScript,
    NoChange,
    ResetSize,
    Ideographic,
    Alphabetic,
    Hanging,
    Mathematical,
    Central,
    Middle,
    TextAfterEdge,
    TextBeforeEdge,
}

impl Default for DominantBaseline {
    fn default() -> Self {
        Self::Auto
    }
}

/// An alignment baseline property.
#[allow(missing_docs)]
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum AlignmentBaseline {
    Auto,
    Baseline,
    BeforeEdge,
    TextBeforeEdge,
    Middle,
    Central,
    AfterEdge,
    TextAfterEdge,
    Ideographic,
    Alphabetic,
    Hanging,
    Mathematical,
}

impl Default for AlignmentBaseline {
    fn default() -> Self {
        Self::Auto
    }
}

/// A baseline shift property.
#[allow(missing_docs)]
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum BaselineShift {
    Baseline,
    Subscript,
    Superscript,
    Number(f32),
}

impl Default for BaselineShift {
    #[inline]
    fn default() -> BaselineShift {
        BaselineShift::Baseline
    }
}

/// A length adjust property.
#[allow(missing_docs)]
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum LengthAdjust {
    Spacing,
    SpacingAndGlyphs,
}

impl Default for LengthAdjust {
    fn default() -> Self {
        Self::Spacing
    }
}

/// A font optical sizing property.
///
/// Controls automatic adjustment of the `opsz` axis in variable fonts
/// based on font size. Matches CSS `font-optical-sizing`.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum FontOpticalSizing {
    /// Automatically set `opsz` to match font size (browser default).
    Auto,
    /// Do not automatically adjust `opsz`.
    None,
}

impl Default for FontOpticalSizing {
    fn default() -> Self {
        Self::Auto
    }
}

/// A text span decoration style.
///
/// In SVG, text decoration and text it's applied to can have different styles.
/// So you can have black text and green underline.
///
/// Also, in SVG you can specify text decoration stroking.
#[derive(Clone, Debug)]
pub struct TextDecorationStyle {
    pub(crate) fill: Option<Fill>,
    pub(crate) stroke: Option<Stroke>,
    /// The lines, style, colour and thickness the declaring element gives.
    pub(crate) declared: crate::style::TextDecoration,
    /// The declaring element's computed style, whose underline offset and position place the
    /// lines (CSS Text Decoration 4 §2.6–§2.7).
    pub(crate) style: Arc<crate::style::Style>,
    /// The declaring element's node index.
    pub(crate) element: usize,
}

impl TextDecorationStyle {
    /// A fill style.
    pub fn fill(&self) -> Option<&Fill> {
        self.fill.as_ref()
    }

    /// A stroke style.
    pub fn stroke(&self) -> Option<&Stroke> {
        self.stroke.as_ref()
    }
}

/// A text span decoration.
#[derive(Clone, Debug)]
pub struct TextDecoration {
    pub(crate) underline: Option<TextDecorationStyle>,
    pub(crate) overline: Option<TextDecorationStyle>,
    pub(crate) line_through: Option<TextDecorationStyle>,
}

impl TextDecoration {
    /// An optional underline and its style.
    pub fn underline(&self) -> Option<&TextDecorationStyle> {
        self.underline.as_ref()
    }

    /// An optional overline and its style.
    pub fn overline(&self) -> Option<&TextDecorationStyle> {
        self.overline.as_ref()
    }

    /// An optional line-through and its style.
    pub fn line_through(&self) -> Option<&TextDecorationStyle> {
        self.line_through.as_ref()
    }
}

/// An element whose box contains a span's text.
#[derive(Clone, Debug)]
pub(crate) struct Ancestor {
    /// The element's node index.
    pub(crate) element: usize,
    pub(crate) style: Arc<crate::style::Style>,
    pub(crate) font: Font,
    /// The used font size, in user units.
    pub(crate) font_size: f32,
}

/// A text style span.
///
/// Spans do not overlap inside a text chunk.
#[derive(Clone, Debug)]
pub struct TextSpan {
    pub(crate) start: usize,
    pub(crate) end: usize,
    pub(crate) fill: Option<Fill>,
    pub(crate) stroke: Option<Stroke>,
    pub(crate) paint_order: PaintOrder,
    pub(crate) font: Font,
    pub(crate) font_size: NonZeroPositiveF32,
    /// The computed style the span's text shapes, cases and paints with (SVG 2 §11.10).
    pub(crate) style: Arc<crate::style::Style>,
    /// The text element and the span's ancestors in it that have boxes, outermost first.
    pub(crate) ancestry: Arc<[Ancestor]>,
    /// The span's text node is a single character, which `text-transform: math-auto`
    /// italicizes (CSS Text 4 §2.1).
    pub(crate) single_character: bool,
    /// `color-interpolation: linearRGB` on the span's element (SVG 2 §13.9).
    pub(crate) linear_light: bool,
    pub(crate) text_orientation: crate::style::logical::TextOrientation,
    pub(crate) font_optical_sizing: FontOpticalSizing,
    pub(crate) decoration: TextDecoration,
    pub(crate) dominant_baseline: DominantBaseline,
    pub(crate) alignment_baseline: AlignmentBaseline,
    /// How far below the alignment point the parent box puts its `alignment_baseline`, with
    /// which the span's aligns (CSS Inline 3 §4.2.2).
    pub(crate) parent_alignment: f32,
    /// The shifts inside the subtree that `line_edge` aligns, or the whole text's without one.
    pub(crate) baseline_shift: Vec<BaselineShift>,
    /// The box aligning the span's subtree with an edge of its line box.
    pub(crate) line_edge: Option<LineEdge>,
    pub(crate) visible: bool,
    pub(crate) letter_spacing: f32,
    pub(crate) word_spacing: f32,
    pub(crate) text_length: Option<f32>,
    pub(crate) length_adjust: LengthAdjust,
}

impl TextSpan {
    /// A span start in bytes.
    ///
    /// Offset is relative to the parent text chunk and not the parent text element.
    pub fn start(&self) -> usize {
        self.start
    }

    /// A span end in bytes.
    ///
    /// Offset is relative to the parent text chunk and not the parent text element.
    pub fn end(&self) -> usize {
        self.end
    }

    /// A fill style.
    pub fn fill(&self) -> Option<&Fill> {
        self.fill.as_ref()
    }

    /// A stroke style.
    pub fn stroke(&self) -> Option<&Stroke> {
        self.stroke.as_ref()
    }

    /// A paint order style.
    pub fn paint_order(&self) -> PaintOrder {
        self.paint_order
    }

    /// A font.
    pub fn font(&self) -> &Font {
        &self.font
    }

    /// A font size.
    pub fn font_size(&self) -> NonZeroPositiveF32 {
        self.font_size
    }

    /// Font optical sizing mode.
    ///
    /// When `Auto` (default), the `opsz` axis will be automatically set
    /// to match the font size for variable fonts that support it.
    /// This matches the CSS `font-optical-sizing: auto` behavior.
    pub fn font_optical_sizing(&self) -> FontOpticalSizing {
        self.font_optical_sizing
    }

    /// A span decorations.
    pub fn decoration(&self) -> &TextDecoration {
        &self.decoration
    }

    /// A span dominant baseline.
    pub fn dominant_baseline(&self) -> DominantBaseline {
        self.dominant_baseline
    }

    /// A span alignment baseline.
    pub fn alignment_baseline(&self) -> AlignmentBaseline {
        self.alignment_baseline
    }

    /// A list of all baseline shift that should be applied to this span.
    ///
    /// Ordered from `text` element down to the actual `span` element.
    pub fn baseline_shift(&self) -> &[BaselineShift] {
        &self.baseline_shift
    }

    /// A visibility property.
    pub fn is_visible(&self) -> bool {
        self.visible
    }

    /// A letter spacing property.
    pub fn letter_spacing(&self) -> f32 {
        self.letter_spacing
    }

    /// A word spacing property.
    pub fn word_spacing(&self) -> f32 {
        self.word_spacing
    }

    /// A text length property.
    pub fn text_length(&self) -> Option<f32> {
        self.text_length
    }

    /// A length adjust property.
    pub fn length_adjust(&self) -> LengthAdjust {
        self.length_adjust
    }
}

/// A text chunk anchor property.
#[allow(missing_docs)]
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum TextAnchor {
    Start,
    Middle,
    End,
}

impl Default for TextAnchor {
    fn default() -> Self {
        Self::Start
    }
}

/// A path used by text-on-path.
#[derive(Debug)]
pub struct TextPath {
    pub(crate) id: NonEmptyString,
    pub(crate) start_offset: f32,
    pub(crate) path: Arc<tiny_skia_path::Path>,
}

impl TextPath {
    /// Element's ID.
    ///
    /// Taken from the SVG itself.
    pub fn id(&self) -> &str {
        self.id.get()
    }

    /// A text offset in SVG coordinates.
    ///
    /// Percentage values already resolved.
    pub fn start_offset(&self) -> f32 {
        self.start_offset
    }

    /// A path.
    pub fn path(&self) -> &tiny_skia_path::Path {
        &self.path
    }
}

/// A text chunk flow property.
#[derive(Clone, Debug)]
pub enum TextFlow {
    /// A linear layout.
    ///
    /// Includes left-to-right, right-to-left and top-to-bottom.
    Linear,
    /// A text-on-path layout.
    Path(Arc<TextPath>),
}

/// A text chunk.
///
/// Text alignment and BIDI reordering can only be done inside a text chunk.
#[derive(Clone, Debug)]
pub struct TextChunk {
    pub(crate) x: Option<f32>,
    pub(crate) y: Option<f32>,
    pub(crate) anchor: TextAnchor,
    /// The chunk's base direction is right-to-left.
    pub(crate) rtl: bool,
    pub(crate) spans: Vec<TextSpan>,
    pub(crate) text_flow: TextFlow,
    pub(crate) text: String,
    /// The preserved segment breaks before the chunk, each of which starts a line (SVG 2
    /// §11.6.1).
    pub(crate) line_breaks: usize,
    /// How far along the inline axis a line of wrapped text moves from its anchor point.
    pub(crate) inline_offset: f32,
    /// The bytes ending the chunk that hang beyond its line's end, which anchoring ignores.
    pub(crate) hanging: usize,
}

impl TextChunk {
    /// An absolute X axis offset.
    pub fn x(&self) -> Option<f32> {
        self.x
    }

    /// An absolute Y axis offset.
    pub fn y(&self) -> Option<f32> {
        self.y
    }

    /// A text anchor.
    pub fn anchor(&self) -> TextAnchor {
        self.anchor
    }

    /// A list of text chunk style spans.
    pub fn spans(&self) -> &[TextSpan] {
        &self.spans
    }

    /// A text chunk flow.
    pub fn text_flow(&self) -> TextFlow {
        self.text_flow.clone()
    }

    /// A text chunk actual text.
    pub fn text(&self) -> &str {
        &self.text
    }
}

/// A writing mode.
#[allow(missing_docs)]
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum WritingMode {
    LeftToRight,
    TopToBottom,
}

/// A text element.
///
/// `text` element in SVG.
#[derive(Clone, Debug)]
pub struct Text {
    pub(crate) id: String,
    pub(crate) rendering_mode: TextRendering,
    pub(crate) dx: Vec<f32>,
    pub(crate) dy: Vec<f32>,
    pub(crate) rotate: Vec<f32>,
    pub(crate) writing_mode: WritingMode,
    pub(crate) chunks: Vec<TextChunk>,
    pub(crate) abs_transform: Transform,
    pub(crate) bounding_box: Rect,
    pub(crate) abs_bounding_box: Rect,
    pub(crate) stroke_bounding_box: Rect,
    pub(crate) abs_stroke_bounding_box: Rect,
    pub(crate) flattened: Box<Group>,
    pub(crate) layouted: Vec<Span>,
    /// `vector-effect: non-scaling-stroke`, which strokes the flattened outlines.
    pub(crate) non_scaling_stroke: bool,
    /// `color-interpolation: linearRGB` on the text element (SVG 2 §13.9).
    pub(crate) linear_light: bool,
    /// The text element's own font, which its line boxes start from (CSS 2.1 §10.8.1).
    pub(crate) strut: Strut,
    /// The wrapping area `inline-size` gives the text (SVG 2 §11.4).
    pub(crate) wrapping: Option<Wrapping>,
    /// What `text-overflow: clip` keeps of wrapped text, in its user space.
    pub(crate) clip: Option<NonZeroRect>,
}

/// The inline size of a text element's wrapping area (SVG 2 §11.4.1), whose other dimension is
/// infinite.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum WrapWidth {
    Length(f32),
    /// The text's min-content inline size (CSS Sizing 3 §5.1): lines break at every
    /// opportunity.
    MinContent,
}

/// How a text element wraps its lines in its wrapping area.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Wrapping {
    pub(crate) width: WrapWidth,
    /// `text-overflow` at the line-left and line-right edges (SVG 2 §11.10.2.10).
    pub(crate) overflow: [crate::style::TextOverflow; 2],
}

/// `vertical-align: top` or `bottom` on a text content element, which aligns the line-over or
/// line-under edge of its aligned subtree with that of the line box (CSS 2.1 §10.8.1, CSS
/// Inline 3 §4.2.3).
#[derive(Clone, Debug)]
pub(crate) struct LineEdge {
    pub(crate) bottom: bool,
    /// Identifies the aligned element among those of the text.
    pub(crate) root: usize,
    /// The aligned element's own font, whose layout bounds its subtree includes.
    pub(crate) strut: Strut,
}

/// The font and computed style of a text element, whose strut every line box contains and
/// whose space measures its tab stops.
#[derive(Clone, Debug)]
pub(crate) struct Strut {
    pub(crate) font: Font,
    pub(crate) font_size: f32,
    pub(crate) style: Arc<crate::style::Style>,
}

impl Text {
    /// Element's ID.
    ///
    /// Taken from the SVG itself.
    /// Isn't automatically generated.
    /// Can be empty.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Rendering mode.
    ///
    /// `text-rendering` in SVG.
    pub fn rendering_mode(&self) -> TextRendering {
        self.rendering_mode
    }

    /// A relative X axis offsets.
    ///
    /// One offset for each Unicode codepoint. Aka `char` in Rust.
    pub fn dx(&self) -> &[f32] {
        &self.dx
    }

    /// A relative Y axis offsets.
    ///
    /// One offset for each Unicode codepoint. Aka `char` in Rust.
    pub fn dy(&self) -> &[f32] {
        &self.dy
    }

    /// A list of rotation angles.
    ///
    /// One angle for each Unicode codepoint. Aka `char` in Rust.
    pub fn rotate(&self) -> &[f32] {
        &self.rotate
    }

    /// A writing mode.
    pub fn writing_mode(&self) -> WritingMode {
        self.writing_mode
    }

    /// A list of text chunks.
    pub fn chunks(&self) -> &[TextChunk] {
        &self.chunks
    }

    /// Element's absolute transform.
    ///
    /// Contains all ancestors transforms including elements's transform.
    ///
    /// Note that this is not the relative transform present in SVG.
    /// The SVG one would be set only on groups.
    pub fn abs_transform(&self) -> Transform {
        self.abs_transform
    }

    /// Element's text bounding box.
    ///
    /// Text bounding box is special in SVG and doesn't represent
    /// tight bounds of the element's content.
    /// You can find more about it
    /// [here](https://razrfalcon.github.io/notes-on-svg-parsing/text/bbox.html).
    ///
    /// `objectBoundingBox` in SVG terms. Meaning it isn't affected by parent transforms.
    ///
    /// Returns `None` when the `text` build feature was disabled.
    /// This is because we have to perform a text layout before calculating a bounding box.
    pub fn bounding_box(&self) -> Rect {
        self.bounding_box
    }

    /// Element's text bounding box in canvas coordinates.
    ///
    /// `userSpaceOnUse` in SVG terms.
    pub fn abs_bounding_box(&self) -> Rect {
        self.abs_bounding_box
    }

    /// Element's object bounding box including stroke.
    ///
    /// Similar to `bounding_box`, but includes stroke.
    ///
    /// Will have the same value as `bounding_box` when path has no stroke.
    pub fn stroke_bounding_box(&self) -> Rect {
        self.stroke_bounding_box
    }

    /// Element's bounding box including stroke in canvas coordinates.
    pub fn abs_stroke_bounding_box(&self) -> Rect {
        self.abs_stroke_bounding_box
    }

    /// Text converted into paths, ready to render.
    ///
    /// Note that this is only a
    /// "best-effort" attempt: The text will be converted into group/paths/image
    /// primitives, so that they can be rendered with the existing infrastructure.
    /// This process is in general lossless and should lead to correct output, with
    /// two notable exceptions:
    /// 1. For glyphs based on the `SVG` table, only glyphs that are pure SVG 1.1/2.0
    ///    are supported. Glyphs that make use of features in the OpenType specification
    ///    that are not part of the original SVG specification are not supported.
    /// 2. For glyphs based on the `COLR` table, there are a certain number of features
    ///    that are not (correctly) supported, such as conical
    ///    gradients, certain gradient transforms and some blend modes. But this shouldn't
    ///    cause any issues in 95% of the cases, as most of those are edge cases.
    ///    If the two above are not acceptable, then you will need to implement your own
    ///    glyph rendering logic based on the layouted glyphs (see the `layouted` method).
    pub fn flattened(&self) -> &Group {
        &self.flattened
    }

    /// Whether the stroke is computed in the host coordinate space (SVG 2 §13.6).
    pub fn non_scaling_stroke(&self) -> bool {
        self.non_scaling_stroke
    }

    /// The positioned glyphs and decoration spans of the text.
    ///
    /// This should only be used if you need more low-level access
    /// to the glyphs that make up the text. If you just need the
    /// outlines of the text, you should use `flattened` instead.
    pub fn layouted(&self) -> &[Span] {
        &self.layouted
    }

    pub(crate) fn subroots(&self, f: &mut dyn FnMut(&Group)) {
        f(&self.flattened);
    }
}
