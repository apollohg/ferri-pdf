//! Font shaping, bidi resolution, and measured inline layout.
mod autospace;
mod bidi;
mod breaking;
mod combine;
mod decoration;
mod ellipsis;
mod emphasis;
mod features;
mod hanging;
mod hyphenation;
mod initial_letter;
mod leaders;
mod line_boxes;
mod paragraph;
mod ruby;
mod shaping;
mod skip_ink;
mod spacing;
mod spacing_trim;
mod tabs;
mod tailoring;
#[cfg(test)]
mod tests;
mod whitespace;
use crate::document::Paragraph;
use crate::fonts::{
    FontBook, FontError, FontId, FontSelection, FontSynthesis, FontVariation, GlyphMatch,
};
use crate::resources::ResourceResolver;
use crate::style::{Style, TextAlign, WhiteSpace};
use bidi::resolve as paragraph_bidi;
use rustybuzz::Face;
use serde::Serialize;
use shaping::MissingGlyphs;
use std::ops::Range;
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    sync::Arc,
};

use unicode_linebreak::BreakOpportunity;
use unicode_script::{Script, UnicodeScript};
use unicode_segmentation::UnicodeSegmentation;

pub(crate) use initial_letter::{
    Surrounding as LetterSurrounding, font_scale as initial_letter_scale,
};
pub(crate) use line_boxes::{AtomicFragment, AtomicMetrics, BoxFragment, BoxMetrics, LinkFragment};

/// Synthetic bold draws a second copy of a run this far to the right.
const SYNTHETIC_BOLD_OFFSET_EM: f32 = 0.02;
/// Maximum conservative cluster-shaping and fallback-probe work in one text context.
pub(crate) const MAX_GLYPH_MATCH_WORK: usize = 100_000_000;
use line_boxes::{Piece, Slot};
pub(crate) use paragraph::{ParagraphClamp, PreparedParagraph, TextPosition, trim_line};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub(crate) enum GlyphOrientation {
    #[default]
    Horizontal,
    UprightRl,
    UprightLr,
    UprightSidewaysLr,
    ReflectedSideways,
}
impl GlyphOrientation {
    /// Font coordinates (y down) to the containing flow's horizontal layout coordinates.
    pub(crate) fn matrix(self) -> [f32; 4] {
        match self {
            Self::Horizontal => [1.0, 0.0, 0.0, 1.0],
            Self::UprightRl => [0.0, -1.0, 1.0, 0.0],
            Self::UprightLr => [0.0, 1.0, 1.0, 0.0],
            Self::UprightSidewaysLr => [0.0, 1.0, -1.0, 0.0],
            Self::ReflectedSideways => [1.0, 0.0, 0.0, -1.0],
        }
    }
    pub(crate) fn font_vector(self, [x, y]: [f32; 2]) -> [f32; 2] {
        let [a, b, c, d] = self.matrix();
        [a * x + b * y, c * x + d * y]
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct GlyphRun {
    pub text: String,
    /// Source coordinates refer to the containing paragraph's UTF-8 text and spans.
    #[serde(skip)]
    pub source_range: Range<usize>,
    #[serde(skip)]
    pub source_span: usize,
    pub rtl: bool,
    pub(crate) orientation: GlyphOrientation,
    pub(crate) inline_scale: f32,
    pub(crate) combined: bool,
    #[serde(skip)]
    bidi_scoped: bool,
    #[serde(skip)]
    leader: bool,
    /// Left edge from the containing line's left edge.
    pub x: f32,
    pub width: f32,
    /// Baseline offset below the line's baseline from `vertical-align`.
    pub baseline_shift: f32,
    pub font_size: f32,
    #[serde(skip)]
    metric_size: f32,
    pub font_id: FontId,
    pub synthesis: FontSynthesis,
    pub oblique_skew: f32,
    pub variations: Vec<FontVariation>,
    /// The colours of a COLR face's palette, where `font-palette` replaces its default.
    #[serde(skip)]
    pub palette: Option<Arc<[crate::graphics::Color]>>,
    /// The glyph fill, `-webkit-text-fill-color`.
    pub color: crate::graphics::Color,
    pub(crate) stroke: Option<TextStroke>,
    /// Emphasis marks over or under the run's characters, placed from the line's left edge
    /// and baseline like the run.
    pub(crate) marks: Vec<GlyphRun>,
    /// Emphasis marks the UA draws as shapes where no font has their character.
    pub(crate) mark_shapes: Vec<MarkShape>,
    /// `text-shadow` layers, the topmost first.
    #[serde(skip)]
    pub(crate) shadows: Arc<[crate::style::TextShadow]>,
    #[serde(skip)]
    pub white_space: WhiteSpace,
    #[serde(skip)]
    leading: Option<f32>,
    #[serde(skip)]
    ink_bounds: Option<[f32; 2]>,
    /// Conservative horizontal overhangs beyond the run's edges, in em.
    #[serde(skip)]
    ink_sides: [f32; 2],
    #[serde(skip)]
    metrics: crate::fonts::FaceMetrics,
    #[serde(skip)]
    glyphs: Vec<PositionedGlyph>,
    /// Run-text offsets of the U+0020 spaces that `text-autospace: replace` removed.
    #[serde(skip)]
    removed_spaces: Vec<usize>,
}

/// A synthesized emphasis mark: contours filled even-odd, placed like its run.
#[derive(Clone, Debug, Serialize)]
pub(crate) struct MarkShape {
    pub contours: Vec<crate::graphics::Contour>,
    pub color: crate::graphics::Color,
}
impl MarkShape {
    pub(crate) fn shape(&self) -> crate::graphics::Shape {
        crate::graphics::Shape::Path {
            contours: self.contours.clone(),
            even_odd: true,
            clip: None,
        }
    }
}

/// `-webkit-text-stroke`: a stroke of `width` centred on each glyph outline.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub(crate) struct TextStroke {
    pub width: f32,
    pub color: crate::graphics::Color,
}
impl TextStroke {
    pub(crate) fn of(style: &Style) -> Option<Self> {
        (style.text_paint.stroke_width > 0.0).then_some(Self {
            width: style.text_paint.stroke_width,
            color: style.text_paint.stroke_color.used,
        })
    }
}

fn script_ranges(text: &str, range: Range<usize>) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    let mut start = range.start;
    let mut current = Script::Common;
    for (offset, ch) in text[range.clone()].char_indices() {
        let script = ch.script();
        if matches!(script, Script::Common | Script::Inherited) {
            continue;
        }
        if current != Script::Common && script != current {
            ranges.push(start..range.start + offset);
            start = range.start + offset;
        }
        current = script;
    }
    ranges.push(start..range.end);
    ranges
}

fn shape_line(
    context: &TextContext<'_>,
    text: &str,
    bidi: &bidi::ResolvedParagraph<'_>,
    range: Range<usize>,
    context_range: Range<usize>,
    paragraph: &Paragraph,
    has_rtl: bool,
) -> Result<Vec<Piece>, TextError> {
    if range.is_empty() {
        return Ok(Vec::new());
    }
    let start = range.start;
    let text = &text[range.clone()];
    let combined = combine::groups(paragraph);
    let emitted = RefCell::new(std::collections::HashSet::new());
    let shape_range = |range: Range<usize>, rtl: bool| -> Result<Vec<Piece>, TextError> {
        let absolute = (start + range.start)..(start + range.end);
        let first = paragraph
            .spans()
            .partition_point(|s| s.range.end <= absolute.start);
        let mut pieces: Vec<_> = paragraph.spans()[first..]
            .iter()
            .enumerate()
            .take_while(|(_, s)| s.range.start < absolute.end)
            .map(|(index, s)| {
                (
                    s.range.start.max(absolute.start)..s.range.end.min(absolute.end),
                    s,
                    first + index,
                )
            })
            .collect();
        if rtl {
            pieces.reverse();
        }
        let mut result = Vec::new();
        for (r, piece, span) in pieces {
            if let Some(index) = piece.atomic {
                result.push(Piece::Atomic {
                    index,
                    offset: piece.range.start,
                    owner: piece.inline_box,
                    rtl,
                });
                continue;
            }
            let mut boundaries = vec![r.start, r.end];
            for group in &combined {
                boundaries.extend(
                    [group.range.start, group.range.end]
                        .into_iter()
                        .filter(|offset| r.start < *offset && *offset < r.end),
                );
            }
            boundaries.sort_unstable();
            boundaries.dedup();
            let mut ranges: Vec<_> = boundaries.windows(2).map(|b| b[0]..b[1]).collect();
            if rtl {
                ranges.reverse();
            }
            for r in ranges {
                if let Some(group) = combined.iter().find(|group| group.range.contains(&r.start)) {
                    if emitted.borrow_mut().insert(group.range.start) {
                        result.extend(
                            combine::shape(context, paragraph, group)?
                                .into_iter()
                                .map(|run| Piece::Text(Box::new(run))),
                        );
                    }
                    continue;
                }
                let (style, source) = (&piece.style, piece.range.clone());
                let shaping_context =
                    source.start.max(context_range.start)..source.end.min(context_range.end);
                let mut runs = tabs::shape(
                    context,
                    &paragraph.text()[shaping_context.clone()],
                    (r.start - shaping_context.start)..(r.end - shaping_context.start),
                    rtl,
                    style,
                )?;
                for run in &mut runs {
                    run.bidi_scoped = bidi.has_controls();
                    run.source_range.start += shaping_context.start;
                    run.source_range.end += shaping_context.start;
                    run.source_span = span;
                    run.leader = piece.leader;
                }
                result.extend(runs.into_iter().map(|run| Piece::Text(Box::new(run))));
            }
        }
        Ok(result)
    };
    if !has_rtl {
        let mut result = Vec::new();
        for range in script_ranges(text, 0..text.len()) {
            result.extend(shape_range(range, false)?);
        }
        return Ok(result);
    }
    let mut result = Vec::new();
    for (run, rtl) in bidi.visual_runs(range) {
        let mut scripts = script_ranges(text, run.start - start..run.end - start);
        if rtl {
            scripts.reverse();
        }
        for script in scripts {
            result.extend(shape_range(script, rtl)?);
        }
    }
    Ok(result)
}

#[derive(Clone)]
pub(crate) struct Line {
    /// An ignored empty line box under CSS 2.1 §9.4.2.
    pub phantom: bool,
    pub wrapped: bool,
    /// Logical start-edge margin, excluded from shaped content width.
    pub indent: f32,
    pub text: String,
    /// Paragraph byte range assigned to this line, including collapsible edge spaces.
    pub range: Range<usize>,
    pub width: f32,
    pub wrap_width: f32,
    pub runs: Vec<GlyphRun>,
    /// Inline box fragments in tree order, each box's pieces from left to right.
    pub boxes: Vec<BoxFragment>,
    pub links: Vec<LinkFragment>,
    pub atomics: Vec<AtomicFragment>,
    /// Ruby annotation boxes, indexed after the paragraph's atomic inlines.
    pub annotations: Vec<AtomicFragment>,
    ruby: ruby::RubyLine,
    pub height: f32,
    pub ascent: f32,
    pub ink_bounds: Option<[f32; 2]>,
    pub align: TextAlign,
    /// Text decoration lines from the line's left edge and baseline, in painting order.
    pub decorations: Vec<crate::graphics::DecorationPaint>,
    order: Vec<Slot>,
    present: Vec<usize>,
    /// Index of the first run of a `text-overflow` ellipsis, which ends `runs`.
    ellipsis: Option<usize>,
    hangable: hanging::Hangable,
    /// Advance of punctuation hanging past the left and right edges, which `width` includes.
    hanging: [f32; 2],
    /// The paragraph's `::first-line` copy styles the line, which its runs, inline box fragments
    /// and decorations index.
    pub first_line_styles: bool,
}

pub(crate) use crate::style::IntrinsicWidths;

/// Font advances and offsets use em units; additional CSS spacing uses points. Y is downwards.
/// Cluster byte ranges index the glyph run's UTF-8 text.
#[derive(Clone, Debug)]
pub struct PositionedGlyph {
    pub id: u32,
    pub text_range: Range<usize>,
    pub x_advance: f32,
    pub x_spacing: f32,
    pub x_offset: f32,
    pub y_offset: f32,
    pub y_advance: f32,
}

impl PositionedGlyph {
    /// Used horizontal advance in points, including CSS spacing and justification.
    pub fn advance(&self, font_size: f32) -> f32 {
        self.x_advance * font_size + self.x_spacing
    }
    /// An em advance for nonzero drawing sizes, absent when normalization is non-finite.
    pub fn normalized_advance(&self, font_size: f32) -> Option<f32> {
        let advance = self.x_advance + self.x_spacing / font_size;
        advance.is_finite().then_some(advance)
    }
}

impl GlyphRun {
    pub fn has_bidi_scopes(&self) -> bool {
        self.bidi_scoped
    }
    /// Pen origins of the copies of the run drawn from its line's left edge and baseline
    /// `origin`: synthetic bold adds a second, offset copy.
    pub(crate) fn copies(&self, [x, y]: [f32; 2]) -> Vec<[f32; 2]> {
        let pen = [x + self.x, y + self.baseline_shift];
        let mut copies = vec![pen];
        if self.synthesis.bold {
            let [a, b, _, _] = self.orientation.matrix();
            let bold = self.font_size * SYNTHETIC_BOLD_OFFSET_EM * self.inline_scale;
            copies.push([pen[0] + a * bold, pen[1] + b * bold]);
        }
        copies
    }
    /// The synthetic oblique shear about the run's `baseline`, as an affine `[a b c d e f]`.
    pub(crate) fn shear(&self, baseline: f32) -> Option<[f32; 6]> {
        self.synthesis.oblique.then(|| {
            [
                1.0,
                0.0,
                -self.oblique_skew,
                1.0,
                self.oblique_skew * baseline,
                0.0,
            ]
        })
    }
    /// Each glyph and its origin, from the pen origin `pen`; y is downwards.
    pub(crate) fn glyph_origins(&self, [x, y]: [f32; 2]) -> Vec<(&PositionedGlyph, [f32; 2])> {
        let mut pen = x;
        self.glyphs
            .iter()
            .map(|glyph| {
                let origin = [
                    pen + glyph.x_offset * self.font_size,
                    y + glyph.y_offset * self.font_size,
                ];
                pen += glyph.advance(self.font_size);
                (glyph, origin)
            })
            .collect()
    }
    /// A bound on the ink of the run's glyphs and strokes, drawn from its line's left edge
    /// and baseline `origin`, within the face's global glyph box.
    pub(crate) fn ink_rect(&self, [x, y]: [f32; 2]) -> Option<crate::graphics::Rect> {
        let glyphs = self.synthesized_ink(
            [
                self.x + (self.ink_sides[0] * self.font_size).min(0.0),
                self.x + self.width + (self.ink_sides[1] * self.font_size).max(0.0),
            ],
            self.ink_bounds?,
            [x, y],
        );
        Some(
            self.marks
                .iter()
                .filter_map(|mark| mark.ink_rect([x, y]))
                .chain(self.mark_shapes.iter().filter_map(|mark| {
                    crate::paint::shape_bounds(&mark.shape()).map(|rect| crate::graphics::Rect {
                        x: rect.x + x,
                        y: rect.y + y,
                        ..rect
                    })
                }))
                .fold(glyphs, crate::graphics::Rect::union),
        )
    }
    /// The ink of glyphs spanning `[left, right]` from the line's left edge and `[top,
    /// bottom]` around the run's baseline, grown by its synthetic bold, oblique shear and
    /// stroke, drawn from its line's left edge and baseline `origin`.
    fn synthesized_ink(
        &self,
        [left, right]: [f32; 2],
        [top, bottom]: [f32; 2],
        [x, y]: [f32; 2],
    ) -> crate::graphics::Rect {
        let shear = if self.synthesis.oblique {
            self.oblique_skew.abs() * top.abs().max(bottom.abs())
        } else {
            0.0
        };
        let bold = if self.synthesis.bold {
            self.font_size * SYNTHETIC_BOLD_OFFSET_EM
        } else {
            0.0
        };
        let stroke = self.stroke.map_or(0.0, |stroke| stroke.width / 2.0);
        let left = x + left - shear - stroke;
        let right = x + right + shear + bold + stroke;
        let top = y + self.baseline_shift + top - stroke;
        let bottom = y + self.baseline_shift + bottom + stroke;
        crate::graphics::Rect {
            x: left,
            y: top,
            width: right - left,
            height: bottom - top,
        }
    }
    /// The run followed by its emphasis marks.
    pub(crate) fn with_marks(&self) -> impl Iterator<Item = &Self> {
        std::iter::once(self).chain(&self.marks)
    }
    pub(crate) fn set_math_glyphs(
        &mut self,
        mut glyphs: Vec<PositionedGlyph>,
        width: f32,
        face: &ttf_parser::Face<'_>,
    ) {
        for glyph in &mut glyphs {
            glyph.text_range = 0..self.text.len();
        }
        self.glyphs = glyphs;
        self.width = width;
        let bounds = self
            .outline_rects(face)
            .reduce(crate::graphics::Rect::union);
        self.ink_bounds = bounds.map(|rect| [rect.y, rect.y + rect.height]);
        self.ink_sides = bounds
            .filter(|_| self.font_size > 0.0)
            .map_or([0.0; 2], |rect| {
                [
                    (rect.x - self.x) / self.font_size,
                    (rect.x + rect.width - self.x - self.width) / self.font_size,
                ]
            });
    }
    fn outline_rects<'r>(
        &'r self,
        face: &'r ttf_parser::Face<'_>,
    ) -> impl Iterator<Item = crate::graphics::Rect> + 'r {
        let scale = self.font_size / f32::from(face.units_per_em());
        self.glyph_origins([self.x, 0.0])
            .into_iter()
            .filter_map(move |(glyph, [x, y])| {
                let outline = u16::try_from(glyph.id)
                    .ok()
                    .and_then(|id| face.glyph_bounding_box(ttf_parser::GlyphId(id)))?;
                Some(crate::graphics::Rect {
                    x: x + f32::from(outline.x_min) * scale,
                    y: y - f32::from(outline.y_max) * scale,
                    width: (f32::from(outline.x_max) - f32::from(outline.x_min)) * scale,
                    height: (f32::from(outline.y_max) - f32::from(outline.y_min)) * scale,
                })
            })
    }
    pub fn glyphs(&self) -> &[PositionedGlyph] {
        &self.glyphs
    }
    /// Advance of the glyph at `index` in points, including CSS spacing and justification.
    pub(crate) fn advance(&self, index: usize) -> f32 {
        let glyph = &self.glyphs[index];
        glyph.advance(self.font_size)
    }
    /// Distance from the run's left edge to paragraph `offset`, which the run contains.
    fn x_at(&self, offset: usize) -> f32 {
        let before: f32 = (0..self.glyphs.len())
            .filter(|&index| self.source_range.start + self.glyphs[index].text_range.start < offset)
            .map(|index| self.advance(index))
            .sum();
        if self.rtl {
            self.width - before
        } else {
            before
        }
    }
}
impl Line {
    pub(crate) fn refresh_ink_bounds(&mut self) {
        self.ink_bounds = self
            .runs
            .iter()
            .flat_map(GlyphRun::with_marks)
            .filter_map(|run| {
                run.ink_bounds
                    .map(|ink| ink.map(|y| y + run.baseline_shift))
            })
            .reduce(|[a, b], [c, d]| [a.min(c), b.max(d)]);
    }

    pub(crate) fn alignment_offset(&self, available: f32, rtl: bool) -> f32 {
        let start = if rtl { 0.0 } else { self.indent };
        let [left, right] = self.hanging;
        start
            + self
                .align
                .offset(available - self.indent, self.width - left - right, rtl)
            - left
    }
    /// Distance from the line's left edge to paragraph `offset`. Offsets outside the
    /// line's runs resolve to its logical start or end in the paragraph direction.
    pub(crate) fn x_at(&self, offset: usize, rtl: bool) -> f32 {
        if let Some(run) = self
            .runs
            .iter()
            .find(|run| run.source_range.contains(&offset))
        {
            return run.x + run.x_at(offset);
        }
        if let Some(atomic) = self.atomics.iter().find(|atomic| atomic.offset == offset) {
            return if rtl {
                atomic.x + atomic.width
            } else {
                atomic.x
            };
        }
        let at_start = self
            .runs
            .iter()
            .map(|run| run.source_range.start)
            .chain(self.atomics.iter().map(|atomic| atomic.offset))
            .min()
            .is_none_or(|start| offset <= start);
        if at_start == rtl { self.width } else { 0.0 }
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum TextError {
    #[error("{0}")]
    Font(#[from] FontError),
    #[error("computed text style has no font candidates")]
    NoFontCandidates,
    #[error("missing glyph in font: {text:?}")]
    MissingGlyph { text: String },
    #[error("glyph matching work exceeds {limit}")]
    GlyphMatchingLimit { limit: usize },
    #[error("bidirectional control expansion work exceeds {limit} bytes")]
    BidiExpansionLimit { limit: usize },
    #[error("non-finite text geometry")]
    NonFiniteGeometry,
    #[error("small-caps casing exceeds the inline text expansion limit")]
    CaseExpansionLimit,
    #[error("leader expansion exceeds {limit} text bytes")]
    LeaderExpansionLimit { limit: usize },
    #[error(
        "paragraph has {expected} atomic inlines and ruby annotations but {actual} were measured"
    )]
    AtomicMetrics { expected: usize, actual: usize },
    #[error("text continuation belongs to another paragraph")]
    ParagraphMismatch,
    #[error("atomic inline {index} on a ::first-line inherits first-line styles into its contents")]
    FirstLineAtomic { index: usize },
    #[error("unsupported ruby layout: {feature}")]
    UnsupportedRuby { feature: &'static str },
}

/// The content language, or the OpenType language system `font-language-override` names
/// where the face defines it; other tags do not affect glyph selection (CSS Fonts 4 §6.13).
/// The shaper takes an explicit system as a `-x-hbot` private-use subtag, which has
/// uppercase letters and digits followed by padding.
fn shaping_language(face: &Face<'_>, style: &Style) -> Option<rustybuzz::Language> {
    const PRIVATE_SYSTEM_SUBTAG: &str = "x-hbot";
    const PADDING: char = ' ';
    let content = || style.language.parse().ok();
    let Some(tag) = style
        .font_features
        .language_override
        .as_deref()
        .and_then(crate::style::language_system_tag)
    else {
        return content();
    };
    let name = std::str::from_utf8(&tag)
        .expect("language system tags are ASCII")
        .trim_end_matches(PADDING);
    if !shaping::has_language_system(face, tag)
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
    {
        return content();
    }
    let source = if style.language.is_empty() {
        format!("{PRIVATE_SYSTEM_SUBTAG}{name}")
    } else {
        format!("{}-{PRIVATE_SYSTEM_SUBTAG}{name}", style.language)
    };
    source.parse().ok()
}

/// Whether `font-variant-emoji` appends a presentation selector anywhere in `text`.
fn implies_presentation(style: &Style, text: &str) -> bool {
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if shaping::implied_presentation(style.font_features.emoji, ch, chars.peek().copied())
            .is_some()
        {
            return true;
        }
    }
    false
}

/// Borrows font data and lazily parses only the faces used by a rendering job.
struct ShapingInstance<'a> {
    face: Face<'a>,
    variations: Arc<[FontVariation]>,
    synthesis: FontSynthesis,
    oblique_skew: f32,
    /// Used font size per computed size, from `font-size-adjust`.
    scale: f32,
}
impl ShapingInstance<'_> {
    fn used_size(&self, style: &Style) -> f32 {
        style.size * self.scale
    }
}

/// A face as matching selects it and the adjustments apply, with the computed size where it
/// selects an optical size or an adjusted size.
type FaceKey = (
    FontId,
    FontSelection,
    crate::fonts::FontAdjustments,
    Option<u32>,
);
type FaceCache<'a> = HashMap<FaceKey, Option<Arc<ShapingInstance<'a>>>>;

pub(crate) struct TextContext<'a> {
    fonts: &'a FontBook,
    resolver: &'a dyn ResourceResolver,
    faces: RefCell<FaceCache<'a>>,
    optical_axes: RefCell<HashMap<FontId, bool>>,
    matching_work: Cell<Option<usize>>,
    bidi_work: Cell<Option<usize>>,
}
/// A paragraph's bidi, break and ruby context for intrinsic measurement.
struct IntrinsicSource<'p, 'c, 'f> {
    paragraph: &'p Paragraph,
    bidi: bidi::ResolvedParagraph<'p>,
    automatic: Vec<usize>,
    opportunities: Vec<(usize, BreakOpportunity)>,
    ruby: Option<ruby::RubyLayout<'p, 'c, 'f>>,
}
impl<'p, 'c, 'f> IntrinsicSource<'p, 'c, 'f> {
    /// `source` holds the annotation boxes of `paragraph`, the paragraph itself or the one its
    /// `::first-line` copies.
    fn new(
        context: &'c TextContext<'f>,
        paragraph: &'p Paragraph,
        source: &'p Paragraph,
    ) -> Result<Self, TextError> {
        let automatic = hyphenation::automatic(paragraph);
        Ok(Self {
            paragraph,
            bidi: paragraph_bidi(paragraph, &context.bidi_work)?,
            opportunities: whitespace::opportunities(paragraph, &automatic),
            automatic,
            ruby: ruby::RubyLayout::of(context, paragraph, source),
        })
    }
}

impl<'a> TextContext<'a> {
    pub(crate) fn fonts(&self) -> &'a FontBook {
        self.fonts
    }
    pub(crate) fn resolver(&self) -> &'a dyn ResourceResolver {
        self.resolver
    }
    /// `atomics` holds the min- and max-content margin-box widths of the paragraph's atomic inlines.
    pub(crate) fn intrinsic_widths(
        &self,
        paragraph: &Paragraph,
        atomics: &[IntrinsicWidths],
    ) -> Result<IntrinsicWidths, TextError> {
        let expected = paragraph.inline_blocks().count();
        if atomics.len() != expected {
            return Err(TextError::AtomicMetrics {
                expected,
                actual: atomics.len(),
            });
        }
        let text = paragraph.text();
        let base = IntrinsicSource::new(self, paragraph, paragraph)?;
        let first_line = paragraph
            .first_line()
            .map(|variant| IntrinsicSource::new(self, variant, paragraph))
            .transpose()?;
        let opportunities = &base.opportunities;
        let base_autospace = autospace::Autospace::of(self, paragraph);
        let first_line_autospace = first_line
            .as_ref()
            .and_then(|source| autospace::Autospace::of(self, source.paragraph));
        let minimum: Vec<f32> = atomics.iter().map(|widths| widths.min).collect();
        let maximum: Vec<f32> = atomics.iter().map(|widths| widths.max).collect();
        let measure = |range: Range<usize>, atomic_widths: &[f32], min_content: bool| {
            // Content from the start sets the first formatted line at either intrinsic size.
            let (source, autospace) = match first_line.as_ref().filter(|_| range.start == 0) {
                Some(source) => (source, first_line_autospace.as_ref()),
                None => (&base, base_autospace.as_ref()),
            };
            let spacers = line_boxes::Spacers {
                autospace,
                ruby: source.ruby.as_ref(),
            };
            let paragraph = source.paragraph;
            let context_range = breaking::context_range(&source.opportunities, range.clone());
            let visible = whitespace::visible(paragraph, range.clone());
            let mut pieces = shape_line(
                self,
                text,
                &source.bidi,
                visible.clone(),
                context_range,
                paragraph,
                source.bidi.has_rtl(),
            )?;
            if visible.end == range.end {
                hyphenation::decorate(
                    self,
                    paragraph,
                    &source.automatic,
                    &source.bidi,
                    range.end,
                    &mut pieces,
                )?;
            }
            let indent = paragraph::indent_at(base.paragraph, opportunities, range.start, 0.0);
            let mut arranged = line_boxes::arrange(
                paragraph,
                pieces,
                range.clone(),
                atomic_widths,
                0.0,
                spacers,
            )?;
            let end_trim = spacing_trim::trim(
                self,
                paragraph,
                &mut arranged,
                paragraph::starts_block_line(base.paragraph, opportunities, range.start),
            )?;
            // Min-content fits no closing punctuation that is trimmed only where it overflows.
            if min_content && let Some(end_trim) = end_trim {
                end_trim.apply(&mut arranged);
            }
            tabs::resolve(self, paragraph, &mut arranged, indent)?;
            let hangable = hanging::hangable(
                paragraph,
                range.clone(),
                visible.clone(),
                &arranged.runs,
                &arranged.order,
            );
            let width = if min_content {
                whitespace::wrapping_width(
                    paragraph,
                    visible.end,
                    &arranged.runs,
                    &arranged.order,
                    arranged.width,
                )
            } else {
                arranged.width
            } + indent
                - hangable.intrinsic(min_content);
            if !width.is_finite() {
                return Err(TextError::NonFiniteGeometry);
            }
            Ok(width)
        };
        if text.is_empty() {
            let width = measure(0..0, &maximum, false)?;
            return Ok(IntrinsicWidths {
                min: width,
                max: width,
            });
        }
        let mut widths = IntrinsicWidths::default();
        let mut line_start = 0;
        let mut word_start = 0;
        let emergency = whitespace::emergency_opportunities(paragraph, true);
        let mut emergency = emergency.into_iter().peekable();
        for (end, opportunity) in opportunities.iter().copied() {
            while emergency.peek().is_some_and(|&offset| offset < end) {
                let offset = emergency.next().unwrap();
                widths.min = widths.min.max(measure(word_start..offset, &minimum, true)?);
                word_start = offset;
            }
            if emergency.peek() == Some(&end) {
                emergency.next();
            }
            widths.min = widths.min.max(measure(word_start..end, &minimum, true)?);
            word_start = end;
            if opportunity == BreakOpportunity::Mandatory {
                widths.max = widths.max.max(measure(line_start..end, &maximum, false)?);
                line_start = end;
            }
        }
        widths.max = widths.max.max(widths.min);
        Ok(widths)
    }
    pub(crate) fn new(fonts: &'a FontBook, resolver: &'a dyn ResourceResolver) -> Self {
        Self {
            fonts,
            resolver,
            faces: RefCell::new(HashMap::new()),
            optical_axes: RefCell::new(HashMap::new()),
            matching_work: Cell::new(Some(MAX_GLYPH_MATCH_WORK)),
            bidi_work: Cell::new(Some(crate::document::MAX_INLINE_TEXT_BYTES)),
        }
    }
    fn face(
        &self,
        id: FontId,
        style: &Style,
    ) -> Result<Option<Arc<ShapingInstance<'a>>>, TextError> {
        let selection = style.font_selection();
        let adjustments = &style.font_adjustments;
        let sized = adjustments.size_adjust.is_some()
            || (adjustments.optical_sizing && self.has_optical_axis(id)?);
        let key = (
            id,
            selection,
            adjustments.clone(),
            sized.then(|| style.size.to_bits()),
        );
        if let Some(face) = self.faces.borrow().get(&key) {
            return Ok(face.clone());
        }
        let face = self
            .fonts
            .sized_instance(id, selection, adjustments, style.size, self.resolver)?
            .map(|(instance, used)| {
                let (synthesis, oblique_skew) = instance.synthesis(selection);
                Arc::new(ShapingInstance {
                    face: Face::from_face(instance.face),
                    variations: instance.variations,
                    synthesis,
                    oblique_skew,
                    scale: if style.size == 0.0 {
                        1.0
                    } else {
                        used / style.size
                    },
                })
            });
        self.faces.borrow_mut().insert(key, face.clone());
        Ok(face)
    }
    fn has_optical_axis(&self, id: FontId) -> Result<bool, TextError> {
        if let Some(optical) = self.optical_axes.borrow().get(&id) {
            return Ok(*optical);
        }
        let optical = self.fonts.has_optical_axis(id, self.resolver)?;
        self.optical_axes.borrow_mut().insert(id, optical);
        Ok(optical)
    }
    fn primary(&self, style: &Style) -> Result<FontId, TextError> {
        for &id in style.font_candidates() {
            if self.face(id, style)?.is_some() {
                return Ok(id);
            }
        }
        Err(TextError::NoFontCandidates)
    }
    fn shape_run(
        &self,
        id: FontId,
        paragraph: &str,
        range: Range<usize>,
        rtl: bool,
        style: &Style,
        missing: MissingGlyphs,
    ) -> Result<Vec<GlyphRun>, TextError> {
        let instance = self.face(id, style)?.ok_or(TextError::NoFontCandidates)?;
        let face = &instance.face;
        let narrow_space_advance = if style.size > 0.0
            && paragraph[range.clone()].contains(shaping::NARROW_NO_BREAK_SPACE)
        {
            let normal_space = match crate::fonts::advance(face, ' ') {
                Some(width) => width,
                None => self
                    .shape(" ", 0..1, false, style, MissingGlyphs::Substitute)?
                    .iter()
                    .filter(|run| run.font_size > 0.0)
                    .map(|run| run.width / run.font_size)
                    .sum(),
            };
            Some(normal_space * shaping::NARROW_SPACE_FACTOR)
        } else {
            None
        };
        let entry = self.fonts.face(id).ok_or(FontError::UnknownFont(id))?;
        let family = &entry.descriptor().family;
        let alternates =
            features::alternate_features(&style.font_features.alternates, |kind, name| {
                self.fonts
                    .feature_value(style.font_namespace(), family, kind, name)
            });
        let palette = entry.data().and_then(|data| {
            self.fonts.palette(
                style.font_namespace(),
                &style.font_features.palette,
                family,
                data,
            )
        });
        let language = shaping_language(face, style);
        shaping::shape_runs(
            shaping::ShapingFont {
                face,
                font_id: id,
                synthesis: instance.synthesis,
                oblique_skew: instance.oblique_skew,
                variations: &instance.variations,
                narrow_space_advance,
                missing,
                size: instance.used_size(style),
                features: features::FaceFeatures {
                    descriptor: &entry.settings().features,
                    alternates: &alternates,
                    ..Default::default()
                },
                language: language.as_ref(),
                palette: palette.as_ref(),
            },
            paragraph,
            range,
            rtl,
            style,
        )
    }
    fn spend_matching_work(&self, units: usize) -> Result<(), TextError> {
        let remaining = self
            .matching_work
            .get()
            .and_then(|work| work.checked_sub(units.max(1)));
        self.matching_work.set(remaining);
        if remaining.is_none() {
            return Err(TextError::GlyphMatchingLimit {
                limit: MAX_GLYPH_MATCH_WORK,
            });
        }
        Ok(())
    }

    fn matching_font(
        &self,
        paragraph: &str,
        range: Range<usize>,
        rtl: bool,
        style: &Style,
        named: Option<&[FontId]>,
        presentation: bool,
    ) -> Result<Option<FontId>, TextError> {
        let candidates = if paragraph[range.clone()].chars().any(shaping::private_use) {
            named.expect("private-use candidates were resolved for the run")
        } else {
            style.font_candidates()
        };
        let matched = self.fonts.find_glyph(candidates, |id| {
            self.spend_matching_work(
                range
                    .len()
                    .saturating_mul(paragraph[range.clone()].chars().count()),
            )?;
            let Some(instance) = self.face(id, style)? else {
                return Ok(GlyphMatch::Unavailable);
            };
            if !shaping::supports_variations(
                &instance.face,
                &paragraph[range.clone()],
                presentation.then_some(style.font_features.emoji),
            ) {
                return Ok(GlyphMatch::Missing);
            }
            match self.shape_run(
                id,
                paragraph,
                range.clone(),
                rtl,
                style,
                MissingGlyphs::Reject,
            ) {
                Ok(_) => Ok(GlyphMatch::Found(())),
                Err(TextError::MissingGlyph { .. }) => Ok(GlyphMatch::Missing),
                Err(error) => Err(error),
            }
        })?;
        Ok(matched.map(|(id, ())| id))
    }

    /// A face with the presentation `font-variant-emoji` implies, else one without it: an
    /// unsupported presentation selector is ignored (CSS Fonts 4 §9.3).
    fn presented_font(
        &self,
        paragraph: &str,
        range: Range<usize>,
        rtl: bool,
        style: &Style,
        named: Option<&[FontId]>,
    ) -> Result<Option<FontId>, TextError> {
        let presented = self.matching_font(paragraph, range.clone(), rtl, style, named, true)?;
        if presented.is_some() || !implies_presentation(style, &paragraph[range.clone()]) {
            return Ok(presented);
        }
        self.matching_font(paragraph, range, rtl, style, named, false)
    }

    fn shape(
        &self,
        paragraph: &str,
        range: Range<usize>,
        rtl: bool,
        style: &Style,
        missing: MissingGlyphs,
    ) -> Result<Vec<GlyphRun>, TextError> {
        if self.matching_work.get().is_none() {
            return Err(TextError::GlyphMatchingLimit {
                limit: MAX_GLYPH_MATCH_WORK,
            });
        }
        let text = &paragraph[range.clone()];
        let mut has_private = false;
        let mut has_variation = false;
        if !text.is_ascii() {
            for cluster in text.graphemes(true) {
                let mut scalars = 0;
                for ch in cluster.chars() {
                    scalars += 1;
                    has_private |= shaping::private_use(ch);
                    has_variation |= shaping::variation_selector(ch);
                }
                if scalars > 1 {
                    self.spend_matching_work(cluster.len().saturating_mul(scalars))?;
                }
            }
        }
        has_variation |= implies_presentation(style, text);
        let primary = self.primary(style)?;
        let named = has_private.then(|| style.named_font_candidates(self.fonts));
        if named.is_none() && !has_variation {
            match self.shape_run(
                primary,
                paragraph,
                range.clone(),
                rtl,
                style,
                MissingGlyphs::Reject,
            ) {
                Ok(runs) => return Ok(runs),
                Err(TextError::MissingGlyph { .. }) => {}
                Err(error) => return Err(error),
            }
        }
        let symbol_font = self.fonts.missing_glyph_font();
        let mut pieces: Vec<(Range<usize>, FontId, MissingGlyphs)> = Vec::new();
        let mut append = |range: Range<usize>, id, policy| {
            if let Some((last, previous, last_policy)) = pieces.last_mut()
                && *previous == id
                && *last_policy == policy
            {
                last.end = range.end;
            } else {
                pieces.push((range, id, policy));
            }
        };
        for (offset, grapheme) in paragraph[range.clone()].grapheme_indices(true) {
            let cluster = range.start + offset..range.start + offset + grapheme.len();
            if let Some(id) =
                self.presented_font(paragraph, cluster.clone(), rtl, style, named.as_deref())?
            {
                append(cluster, id, MissingGlyphs::Reject);
                continue;
            }
            if missing == MissingGlyphs::Reject {
                return Err(TextError::MissingGlyph {
                    text: grapheme.into(),
                });
            }
            if grapheme.chars().nth(1).is_none() {
                append(
                    cluster,
                    symbol_font.ok_or(TextError::NoFontCandidates)?,
                    MissingGlyphs::Substitute,
                );
                continue;
            }
            // CSS Fonts 4 §5.3 retains the longest supported prefix before matching marks.
            let mut remainder = cluster.start;
            for (offset, _) in grapheme
                .char_indices()
                .rev()
                .filter(|(offset, _)| *offset > 0)
            {
                let prefix = cluster.start..cluster.start + offset;
                if let Some(id) =
                    self.presented_font(paragraph, prefix.clone(), rtl, style, named.as_deref())?
                {
                    remainder = prefix.end;
                    append(prefix, id, MissingGlyphs::Reject);
                    break;
                }
            }
            for (offset, ch) in paragraph[remainder..cluster.end].char_indices() {
                let part = remainder + offset..remainder + offset + ch.len_utf8();
                if let Some(id) =
                    self.presented_font(paragraph, part.clone(), rtl, style, named.as_deref())?
                {
                    append(part, id, MissingGlyphs::Reject);
                } else {
                    append(
                        part,
                        symbol_font.ok_or(TextError::NoFontCandidates)?,
                        MissingGlyphs::Substitute,
                    );
                }
            }
        }
        if rtl {
            pieces.reverse();
        }
        let mut runs = Vec::new();
        for (range, id, policy) in pieces {
            runs.extend(self.shape_run(id, paragraph, range, rtl, style, policy)?);
        }
        Ok(runs)
    }
    pub(crate) fn layout_paragraph(
        &self,
        paragraph: &Paragraph,
        width: f32,
        atomics: &[AtomicMetrics],
    ) -> Result<Vec<Line>, TextError> {
        let prepared = self.prepare_paragraph(paragraph);
        prepared.lines(prepared.start(), width, atomics)?.collect()
    }
    /// The bounding box of the glyph outlines `line` of `paragraph` paints, from the line's
    /// left edge and baseline; `None` where it paints none. Initial letters fit and kern to
    /// these outlines rather than the faces' global boxes (CSS Inline 3 §§7.5.5, 7.6.2).
    pub(crate) fn outline_bounds(
        &self,
        paragraph: &Paragraph,
        line: &Line,
    ) -> Result<Option<crate::graphics::Rect>, TextError> {
        let mut bounds: Option<crate::graphics::Rect> = None;
        for run in &line.runs {
            let style = &paragraph.spans()[run.source_span].style;
            let Some(instance) = self.face(run.font_id, style)? else {
                continue;
            };
            for rect in run.outline_rects(&instance.face) {
                let ink = run.synthesized_ink(
                    [rect.x, rect.x + rect.width],
                    [rect.y, rect.y + rect.height],
                    [0.0, 0.0],
                );
                bounds = Some(bounds.map_or(ink, |bounds| bounds.union(ink)));
            }
        }
        Ok(bounds)
    }
    pub(crate) fn place_line_emphasis(
        &self,
        paragraph: &Paragraph,
        line: &mut Line,
    ) -> Result<(), TextError> {
        emphasis::place(self, paragraph, line)
    }

    pub(crate) fn finish_line_paint(
        &self,
        paragraph: &Paragraph,
        line: &mut Line,
    ) -> Result<(), TextError> {
        line.refresh_ink_bounds();
        line.links = line_boxes::links(paragraph, line);
        line.decorations = decoration::paint(self, paragraph, line)?;
        Ok(())
    }

    pub(crate) fn prepare_paragraph<'p, 'context>(
        &'context self,
        paragraph: &'p Paragraph,
    ) -> PreparedParagraph<'p, 'context, 'a> {
        PreparedParagraph::new(self, paragraph)
    }
}
