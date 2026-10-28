//! Font shaping, bidi resolution, and measured inline layout.
mod bidi;
mod breaking;
mod decoration;
mod ellipsis;
mod hyphenation;
mod leaders;
mod line_boxes;
mod paragraph;
mod shaping;
mod spacing;
mod whitespace;
use bidi::resolve as paragraph_bidi;
use ferri_pdf_document::Paragraph;
use ferri_pdf_fonts::{
    FontBook, FontError, FontId, FontSelection, FontSynthesis, FontVariation, GlyphMatch,
};
use ferri_pdf_resources::ResourceResolver;
use ferri_pdf_style::{Style, TextAlign, WhiteSpace};
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

pub(crate) use line_boxes::{AtomicFragment, AtomicMetrics, BoxFragment, LinkFragment};

/// Maximum conservative cluster-shaping and fallback-probe work in one text context.
pub(crate) const MAX_GLYPH_MATCH_WORK: usize = 100_000_000;
use line_boxes::{Piece, Slot};
pub(crate) use paragraph::{PreparedParagraph, TextPosition};

#[derive(Clone, Serialize)]
pub struct GlyphRun {
    pub text: String,
    /// Source coordinates refer to the containing paragraph's UTF-8 text and spans.
    #[serde(skip)]
    pub source_range: Range<usize>,
    #[serde(skip)]
    pub source_span: usize,
    pub rtl: bool,
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
    pub color: ferri_pdf_graphics::Color,
    #[serde(skip)]
    pub white_space: WhiteSpace,
    #[serde(skip)]
    leading: Option<f32>,
    #[serde(skip)]
    ink_bounds: Option<[f32; 2]>,
    #[serde(skip)]
    metrics: ferri_pdf_fonts::FaceMetrics,
    #[serde(skip)]
    glyphs: Vec<PositionedGlyph>,
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
            let (style, source) = (&piece.style, piece.range.clone());
            let shaping_context =
                source.start.max(context_range.start)..source.end.min(context_range.end);
            let mut runs = context.shape(
                &paragraph.text()[shaping_context.clone()],
                (r.start - shaping_context.start)..(r.end - shaping_context.start),
                rtl,
                style,
                MissingGlyphs::Substitute,
            )?;
            for run in &mut runs {
                run.bidi_scoped = bidi.has_controls();
                run.source_range.start += shaping_context.start;
                run.source_range.end += shaping_context.start;
                run.source_span = span;
                run.leader = piece.leader;
            }
            result.extend(runs.into_iter().map(Piece::Text));
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
    pub height: f32,
    pub ascent: f32,
    pub ink_bounds: Option<[f32; 2]>,
    pub align: TextAlign,
    /// Text decoration lines from the line's left edge and baseline, in painting order.
    pub decorations: Vec<ferri_pdf_graphics::DecorationPaint>,
    order: Vec<Slot>,
    present: Vec<usize>,
    /// Index of the first run of a `text-overflow` ellipsis, which ends `runs`.
    ellipsis: Option<usize>,
}

pub(crate) use ferri_pdf_style::IntrinsicWidths;

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
    pub(crate) fn alignment_offset(&self, available: f32, rtl: bool) -> f32 {
        let start = if rtl { 0.0 } else { self.indent };
        start + self.align.offset(available - self.indent, self.width, rtl)
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
    #[error("paragraph has {expected} atomic inlines but {actual} were measured")]
    AtomicMetrics { expected: usize, actual: usize },
    #[error("text continuation belongs to another paragraph")]
    ParagraphMismatch,
}

/// Borrows font data and lazily parses only the faces used by a rendering job.
struct ShapingInstance<'a> {
    face: Face<'a>,
    variations: Arc<[FontVariation]>,
    synthesis: FontSynthesis,
    oblique_skew: f32,
}

type FaceCache<'a> = HashMap<(FontId, FontSelection), Option<Arc<ShapingInstance<'a>>>>;

pub(crate) struct TextContext<'a> {
    fonts: &'a FontBook,
    resolver: &'a dyn ResourceResolver,
    faces: RefCell<FaceCache<'a>>,
    matching_work: Cell<Option<usize>>,
    bidi_work: Cell<Option<usize>>,
}
impl<'a> TextContext<'a> {
    /// `atomics` holds the min- and max-content margin-box widths of the paragraph's atomic inlines.
    pub(crate) fn intrinsic_widths(
        &self,
        paragraph: &Paragraph,
        atomics: &[IntrinsicWidths],
    ) -> Result<IntrinsicWidths, TextError> {
        if atomics.len() != paragraph.atomics().len() {
            return Err(TextError::AtomicMetrics {
                expected: paragraph.atomics().len(),
                actual: atomics.len(),
            });
        }
        let text = paragraph.text();
        let bidi = paragraph_bidi(paragraph, &self.bidi_work)?;
        let has_rtl = bidi.has_rtl();
        let opportunities = whitespace::opportunities(paragraph);
        let minimum: Vec<f32> = atomics.iter().map(|widths| widths.min).collect();
        let maximum: Vec<f32> = atomics.iter().map(|widths| widths.max).collect();
        let measure = |range: Range<usize>, atomic_widths: &[f32]| {
            let context_range = breaking::context_range(&opportunities, range.clone());
            let visible = whitespace::visible(paragraph, range.clone());
            let mut pieces = shape_line(
                self,
                text,
                &bidi,
                visible.clone(),
                context_range,
                paragraph,
                has_rtl,
            )?;
            if visible.end == range.end {
                hyphenation::decorate(self, paragraph, &bidi, range.end, &mut pieces)?;
            }
            let indent = paragraph::indent_at(paragraph, &opportunities, range.start, 0.0);
            let arranged = line_boxes::arrange(paragraph, pieces, range, atomic_widths, 0.0);
            let width =
                whitespace::wrapping_width(paragraph, visible.end, &arranged.runs, arranged.width)
                    + indent;
            if !width.is_finite() {
                return Err(TextError::NonFiniteGeometry);
            }
            Ok(width)
        };
        if text.is_empty() {
            let width = measure(0..0, &maximum)?;
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
                widths.min = widths.min.max(measure(word_start..offset, &minimum)?);
                word_start = offset;
            }
            if emergency.peek() == Some(&end) {
                emergency.next();
            }
            widths.min = widths.min.max(measure(word_start..end, &minimum)?);
            word_start = end;
            if opportunity == BreakOpportunity::Mandatory {
                widths.max = widths.max.max(measure(line_start..end, &maximum)?);
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
            matching_work: Cell::new(Some(MAX_GLYPH_MATCH_WORK)),
            bidi_work: Cell::new(Some(ferri_pdf_document::MAX_INLINE_TEXT_BYTES)),
        }
    }
    fn face(
        &self,
        id: FontId,
        style: &Style,
    ) -> Result<Option<Arc<ShapingInstance<'a>>>, TextError> {
        let selection = style.font_selection();
        let key = (id, selection);
        if let Some(face) = self.faces.borrow().get(&key) {
            return Ok(face.clone());
        }
        let face = self
            .fonts
            .instance(id, selection, self.resolver)?
            .map(|instance| {
                let (synthesis, oblique_skew) = instance.synthesis(selection);
                Arc::new(ShapingInstance {
                    face: Face::from_face(instance.face),
                    variations: instance.variations,
                    synthesis,
                    oblique_skew,
                })
            });
        self.faces.borrow_mut().insert(key, face.clone());
        Ok(face)
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
            let normal_space = match ferri_pdf_fonts::advance(face, ' ') {
                Some(width) => width,
                None => self
                    .shape(" ", 0..1, false, style, MissingGlyphs::Substitute)?
                    .iter()
                    .map(|run| run.width / style.size)
                    .sum(),
            };
            Some(normal_space * shaping::NARROW_SPACE_FACTOR)
        } else {
            None
        };
        shaping::shape_runs(
            shaping::ShapingFont {
                face,
                font_id: id,
                synthesis: instance.synthesis,
                oblique_skew: instance.oblique_skew,
                variations: &instance.variations,
                narrow_space_advance,
                missing,
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
            if !shaping::supports_variations(&instance.face, &paragraph[range.clone()]) {
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
                self.matching_font(paragraph, cluster.clone(), rtl, style, named.as_deref())?
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
                    self.matching_font(paragraph, prefix.clone(), rtl, style, named.as_deref())?
                {
                    remainder = prefix.end;
                    append(prefix, id, MissingGlyphs::Reject);
                    break;
                }
            }
            for (offset, ch) in paragraph[remainder..cluster.end].char_indices() {
                let part = remainder + offset..remainder + offset + ch.len_utf8();
                if let Some(id) =
                    self.matching_font(paragraph, part.clone(), rtl, style, named.as_deref())?
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
    pub(crate) fn prepare_paragraph<'p, 'context>(
        &'context self,
        paragraph: &'p Paragraph,
    ) -> PreparedParagraph<'p, 'context, 'a> {
        PreparedParagraph::new(self, paragraph)
    }
}
