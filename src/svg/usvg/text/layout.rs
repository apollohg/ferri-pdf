// Copyright 2022 the Resvg Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use std::collections::HashMap;

use crate::svg::usvg::work::{self, Work};
use std::num::NonZeroU16;
use std::sync::Arc;

use fontdb::{Database, ID};
use kurbo::{ParamCurve, ParamCurveArclen, ParamCurveDeriv};
use rustybuzz::ttf_parser;
use rustybuzz::ttf_parser::GlyphId;

use strict_num::NonZeroPositiveF32;
use tiny_skia_path::{NonZeroRect, Transform};
use unicode_script::UnicodeScript;

use crate::paint::DecorationLine;
use crate::svg::usvg::tree::{BBox, IsValidLength};
use crate::svg::usvg::{
    AlignmentBaseline, ApproxZeroUlps, BaselineShift, DominantBaseline, Fill, FillRule, Font,
    FontProvider, LengthAdjust, PaintOrder, Path, ShapeRendering, Stroke, Text, TextAnchor,
    TextChunk, TextDecorationStyle, TextFlow, TextPath, TextSpan, WritingMode,
};

#[cfg(test)]
mod tests;

const VERTICAL_ROTATION: f32 = 90.0;
type OrientationSpans<'a> = Option<
    &'a [(
        std::ops::Range<usize>,
        crate::style::logical::TextOrientation,
    )],
>;

/// A glyph that has already been positioned correctly.
///
/// Note that the transform already takes the font size into consideration, so applying the
/// transform to the outline of the glyphs is all that is necessary to display it correctly.
#[derive(Clone, Debug)]
pub struct PositionedGlyph {
    /// Returns the transform of the glyph itself within the cluster. For example,
    /// for zalgo text, it contains the transform to position the glyphs above/below
    /// the main glyph.
    glyph_ts: Transform,
    /// Returns the transform of the whole cluster that the glyph is part of.
    cluster_ts: Transform,
    /// Returns the transform of the span that the glyph is a part of.
    span_ts: Transform,
    /// The units per em of the font the glyph belongs to.
    units_per_em: u16,
    /// The font size the glyph should be scaled to.
    font_size: f32,
    /// The ID of the glyph.
    pub id: GlyphId,
    /// The text from the original string that corresponds to that glyph.
    pub text: String,
    /// The ID of the font the glyph should be taken from. Can be used with the
    /// [font database of the tree](crate::svg::usvg::Tree::fontdb) this glyph is part of.
    pub font: ID,
    /// UTF-8 source cluster offset within the text chunk.
    pub cluster: usize,
    /// Horizontal advance in font em units before cluster positioning.
    pub x_advance: f32,
    /// The palette and synthesis of the glyph's face.
    pub(crate) paint: Arc<crate::svg::usvg::GlyphPaint>,
}

impl PositionedGlyph {
    /// The glyph and the copy synthetic bold draws offset along its inline axis.
    pub(crate) fn copies(&self) -> impl Iterator<Item = std::borrow::Cow<'_, Self>> {
        let bold = self.paint.bold.then(|| {
            let offset = crate::text::SYNTHETIC_BOLD_OFFSET_EM * f32::from(self.units_per_em);
            std::borrow::Cow::Owned(Self {
                glyph_ts: self.glyph_ts.pre_translate(offset, 0.0),
                ..self.clone()
            })
        });
        std::iter::once(std::borrow::Cow::Borrowed(self)).chain(bold)
    }

    /// Returns the font size for this glyph.
    pub fn font_size(&self) -> f32 {
        self.font_size
    }

    /// Returns the transform of glyph.
    pub fn transform(&self) -> Transform {
        let sx = self.font_size / self.units_per_em as f32;

        self.span_ts
            .pre_concat(self.cluster_ts)
            .pre_concat(Transform::from_scale(sx, sx))
            .pre_concat(self.glyph_ts)
    }

    /// Returns the transform of glyph, assuming that an outline
    /// glyph is being used (i.e. from the `glyf` or `CFF/CFF2` table).
    pub fn outline_transform(&self) -> Transform {
        // Outlines are mirrored by default.
        self.transform()
            .pre_concat(Transform::from_scale(1.0, -1.0))
    }

    /// Returns the transform for the glyph, assuming that a CBTD-based raster glyph
    /// is being used.
    pub fn cbdt_transform(&self, x: f32, y: f32, pixels_per_em: f32, height: f32) -> Transform {
        self.transform()
            .pre_concat(Transform::from_scale(
                self.units_per_em as f32 / pixels_per_em,
                self.units_per_em as f32 / pixels_per_em,
            ))
            // Right now, the top-left corner of the image would be placed in
            // on the "text cursor", but we want the bottom-left corner to be there,
            // so we need to shift it up and also apply the x/y offset.
            .pre_translate(x, -height - y)
    }

    /// Returns the transform for the glyph, assuming that a sbix-based raster glyph
    /// is being used.
    pub fn sbix_transform(
        &self,
        x: f32,
        y: f32,
        x_min: f32,
        y_min: f32,
        pixels_per_em: f32,
        height: f32,
    ) -> Transform {
        // In contrast to CBDT, we also need to look at the outline bbox of the glyph and add a shift if necessary.
        let bbox_x_shift = -x_min;

        let bbox_y_shift = if y_min.approx_zero_ulps(4) {
            // For unknown reasons, using Apple Color Emoji will lead to a vertical shift on MacOS, but this shift
            // doesn't seem to be coming from the font and most likely is somehow hardcoded. On Windows,
            // this shift will not be applied. However, if this shift is not applied the emojis are a bit
            // too high up when being together with other text, so we try to imitate this.
            // See also https://github.com/harfbuzz/harfbuzz/issues/2679#issuecomment-1345595425
            // So whenever the y-shift is 0, we approximate this vertical shift that seems to be produced by it.
            // This value seems to be pretty close to what is happening on MacOS.
            // We can still remove this if it turns out to be a problem, but Apple Color Emoji is pretty
            // much the only `sbix` font out there and they all seem to have a y-shift of 0, so it
            // makes sense to keep it.
            0.128 * self.units_per_em as f32
        } else {
            -y_min
        };

        self.transform()
            .pre_concat(Transform::from_translate(bbox_x_shift, bbox_y_shift))
            .pre_concat(Transform::from_scale(
                self.units_per_em as f32 / pixels_per_em,
                self.units_per_em as f32 / pixels_per_em,
            ))
            // Right now, the top-left corner of the image would be placed in
            // on the "text cursor", but we want the bottom-left corner to be there,
            // so we need to shift it up and also apply the x/y offset.
            .pre_translate(x, -height - y)
    }

    /// Returns the transform for the glyph, assuming that an SVG glyph is
    /// being used.
    pub fn svg_transform(&self) -> Transform {
        self.transform()
    }

    /// Returns the transform for the glyph, assuming that a COLR glyph is
    /// being used.
    pub fn colr_transform(&self) -> Transform {
        self.outline_transform()
    }
}

/// A span contains a number of layouted glyphs that share the same fill, stroke, paint order and
/// visibility.
#[derive(Clone, Debug)]
pub struct Span {
    /// The fill of the span.
    pub fill: Option<Fill>,
    /// The stroke of the span.
    pub stroke: Option<Stroke>,
    /// The paint order of the span.
    pub paint_order: PaintOrder,
    /// The font size of the span.
    pub font_size: NonZeroPositiveF32,
    /// Font variation settings for variable fonts.
    pub variations: Vec<crate::svg::usvg::FontVariation>,
    /// Font optical sizing mode.
    pub font_optical_sizing: crate::svg::usvg::FontOpticalSizing,
    /// The visibility of the span.
    pub visible: bool,
    /// `color-interpolation: linearRGB` on the span's element.
    pub linear_light: bool,
    /// The glyphs that make up the span.
    pub positioned_glyphs: Vec<PositionedGlyph>,
    /// An underline text decoration of the span.
    /// Needs to be rendered before all glyphs.
    pub underline: Option<Path>,
    /// An overline text decoration of the span.
    /// Needs to be rendered before all glyphs.
    pub overline: Option<Path>,
    /// A line-through text decoration of the span.
    /// Needs to be rendered after all glyphs.
    pub line_through: Option<Path>,
    /// `text-shadow`, in points.
    pub(crate) shadows: Arc<[crate::style::TextShadow]>,
    /// The span's emphasis marks, painted after its glyphs; they are not text.
    pub(crate) emphasis: Option<Path>,
}

#[derive(Clone, Debug)]
pub(crate) struct GlyphCluster {
    pub(crate) byte_idx: ByteIndex,
    codepoint: char,
    vertical: bool,
    width: f32,
    pub(crate) advance: f32,
    ascent: f32,
    descent: f32,
    has_relative_shift: bool,
    glyphs: Vec<PositionedGlyph>,
    transform: Transform,
    visible: bool,
    /// The cluster's resolved direction is right-to-left.
    rtl: bool,
}

impl GlyphCluster {
    pub(crate) fn height(&self) -> f32 {
        self.ascent - self.descent
    }

    pub(crate) fn transform(&self) -> Transform {
        self.transform
    }
}

/// Pre-formatted `text` whose chunks on its first line, those before its first preserved
/// segment break, are the first line's `line`.
fn with_first_line(text: &Text, line: &crate::svg::usvg::FirstLine) -> Text {
    let count = text
        .chunks
        .iter()
        .skip(1)
        .take_while(|chunk| chunk.line_breaks == 0)
        .count()
        + 1;
    let first = line.chunks.iter().take(count);
    Text {
        chunks: first
            .chain(text.chunks.iter().skip(count))
            .cloned()
            .collect(),
        ..text.clone()
    }
}

/// The text's laid out spans, its bounding box, and the area `text-overflow: clip` keeps of
/// it.
pub(crate) fn layout_text(
    text_node: &Text,
    resolver: &dyn FontProvider,
    fontdb: &mut Arc<Database>,
) -> Option<(Vec<Span>, NonZeroRect, Option<NonZeroRect>)> {
    let mut fonts_cache: FontsCache = HashMap::new();

    let first_line = text_node.first_line.as_ref();
    let line_chunks = first_line.into_iter().flat_map(|line| &line.chunks);
    let struts = std::iter::once(&text_node.strut).chain(first_line.map(|line| &line.strut));
    let span_fonts = text_node
        .chunks
        .iter()
        .chain(line_chunks)
        .flat_map(|chunk| {
            chunk.spans.iter().flat_map(|span| {
                let edge = span.line_edge.as_ref().map(|edge| &edge.strut.font);
                let ancestors = span.ancestry.iter().map(|ancestor| &ancestor.font);
                std::iter::once(&span.font).chain(edge).chain(ancestors)
            })
        });
    for font in span_fonts.chain(struts.map(|strut| &strut.font)) {
        if !fonts_cache.contains_key(font) {
            if let Some(loaded) = resolver
                .select_font(font, fontdb)
                .and_then(|id| fontdb.load_font(id, resolver.metric_overrides(id)))
            {
                fonts_cache.insert(font.clone(), Arc::new(loaded));
            }
        }
    }

    let laid_out = match (text_node.wrapping, first_line) {
        (Some(wrapping), _) => Some(super::wrap::wrap(
            text_node,
            wrapping,
            &fonts_cache,
            resolver,
            fontdb,
        )?),
        (None, Some(line)) => Some(with_first_line(text_node, line)),
        (None, None) => None,
    };
    let text_node = laid_out.as_ref().unwrap_or(text_node);
    let lines = LineBoxes::new(text_node, &fonts_cache);

    let mut spans = vec![];
    let mut char_offset = 0;
    let mut last_x = 0.0;
    let mut last_y = 0.0;
    let mut line_origin = None;
    let mut line = 0;
    let mut block_offset = 0.0;
    let mut bbox = BBox::default();
    let elements = ElementSpans::of(text_node);
    for (chunk_index, chunk) in text_node.chunks.iter().enumerate() {
        char_offset += chunk.line_breaks;
        for _ in 0..chunk.line_breaks {
            block_offset += lines.advance(line);
            line += 1;
        }
        let (x, y) = match chunk.text_flow {
            TextFlow::Linear => {
                let (start_x, start_y) = *line_origin
                    .get_or_insert((chunk.x.unwrap_or(last_x), chunk.y.unwrap_or(last_y)));
                if chunk.line_breaks == 0 {
                    (chunk.x.unwrap_or(last_x), chunk.y.unwrap_or(last_y))
                } else {
                    let (line_x, line_y) = match text_node.writing_mode {
                        WritingMode::LeftToRight => (start_x, start_y + block_offset),
                        WritingMode::TopToBottom => {
                            (start_x - lines.progression * block_offset, start_y)
                        }
                    };
                    (chunk.x.unwrap_or(line_x), chunk.y.unwrap_or(line_y))
                }
            }
            TextFlow::Path(_) => (0.0, 0.0),
        };
        let (x, y) = match text_node.writing_mode {
            WritingMode::LeftToRight => (x + chunk.inline_offset, y),
            WritingMode::TopToBottom => (x, y + chunk.inline_offset),
        };

        let mut clusters = spaced_clusters(text_node, chunk, &fonts_cache, resolver, fontdb)?;
        if clusters.is_empty() {
            char_offset += chunk.text.chars().count();
            continue;
        }

        let strut = text_node.line_strut(line);
        if let (Some(font), TextFlow::Linear) = (fonts_cache.get(&strut.font), &chunk.text_flow) {
            let (start_x, start_y) = line_origin.unwrap_or((x, y));
            let inline_start = match text_node.writing_mode {
                WritingMode::LeftToRight => x - start_x,
                WritingMode::TopToBottom => y - start_y,
            };
            apply_tab_stops(
                chunk,
                strut,
                font,
                &fonts_cache,
                inline_start,
                &mut clusters,
            );
        }

        apply_length_adjust(chunk, &mut clusters);
        let mut curr_pos = resolve_clusters_positions(
            text_node,
            chunk,
            char_offset,
            (&lines, line),
            &fonts_cache,
            &mut clusters,
        );

        let mut text_ts = Transform::default();
        if text_node.writing_mode == WritingMode::TopToBottom {
            if let TextFlow::Linear = chunk.text_flow {
                text_ts = text_ts.pre_rotate_at(VERTICAL_ROTATION, x, y);
            }
        }

        let skipped = skipped_clusters(chunk, &clusters);
        let owned = owned_clusters(chunk, &clusters);
        let chunk_ts = text_ts.pre_translate(x, y);
        let extent = clusters_extent(&clusters, chunk_ts);
        for (span_index, span) in chunk.spans.iter().enumerate() {
            let font = match fonts_cache.get(&span.font) {
                Some(v) => v,
                None => continue,
            };
            let owned = &owned[span_index];

            let decoration_spans = collect_decoration_spans(owned, &clusters, &skipped);

            let mut span_ts = chunk_ts;
            if let TextFlow::Linear = chunk.text_flow {
                let shift = lines.baseline(line, span, font, text_node.writing_mode);

                // In case of a horizontal flow, shift transform and not clusters,
                // because clusters can be rotated and an additional shift will lead
                // to invalid results.
                span_ts = span_ts.pre_translate(0.0, shift);
            }

            let [mut underline, mut overline, mut line_through] = [
                (DecorationLine::Underline, &span.decoration.underline),
                (DecorationLine::Overline, &span.decoration.overline),
                (DecorationLine::LineThrough, &span.decoration.line_through),
            ]
            .map(|(line, decoration)| {
                let decoration = decoration.as_ref()?;
                let path = convert_decoration(
                    line,
                    span,
                    font,
                    decoration,
                    Decorated {
                        spans: &decoration_spans,
                        clusters: &clusters,
                        edges: elements.edges(decoration.element, (chunk_index, span_index)),
                    },
                    text_node.writing_mode,
                    span_ts,
                    (resolver, &mut *fontdb),
                )?;
                bbox = bbox.expand(path.data.bounds());
                Some(path)
            });

            let mut emphasis = emphasis_marks(
                span,
                owned.iter().map(|&index| &clusters[index]),
                font,
                text_node.writing_mode,
                span_ts,
                (&fonts_cache, resolver, fontdb),
            );
            if let Some(path) = &emphasis {
                bbox = bbox.expand(path.data.bounds());
            }

            let mut fill = span.fill.clone();
            if let Some(ref mut fill) = fill {
                // The `fill-rule` should be ignored.
                // https://www.w3.org/TR/SVG2/text.html#TextRenderingOrder
                //
                // 'Since the fill-rule property does not apply to SVG text elements,
                // the specific order of the subpaths within the equivalent path does not matter.'
                fill.rule = FillRule::NonZero;
            }

            // The extent of the chunk's clusters on the span's baseline.
            if let Some(extent) = extent.and_then(|extent| {
                extent.translate_to(
                    extent.x() + span_ts.tx - chunk_ts.tx,
                    extent.y() + span_ts.ty - chunk_ts.ty,
                )
            }) {
                bbox = bbox.expand(extent);

                let positioned_glyphs: Vec<PositionedGlyph> = owned
                    .iter()
                    .map(|&index| &clusters[index])
                    .filter(|cluster| cluster.visible)
                    .cloned()
                    .flat_map(|mut gc| {
                        let cluster_ts = gc.transform();
                        gc.glyphs.iter_mut().for_each(|pg| {
                            pg.cluster_ts = cluster_ts;
                            pg.span_ts = span_ts;
                        });
                        gc.glyphs
                    })
                    .collect();

                let mut settings = HashMap::new();
                let mut groups: Vec<(Vec<crate::svg::usvg::FontVariation>, Vec<PositionedGlyph>)> =
                    Vec::new();
                for glyph in positioned_glyphs {
                    let variations = settings
                        .entry(glyph.font)
                        .or_insert_with(|| resolver.variations(&span.font, glyph.font));
                    if groups.last().is_none_or(|group| group.0 != *variations) {
                        groups.push((variations.clone(), Vec::new()));
                    }
                    groups.last_mut().unwrap().1.push(glyph);
                }
                if groups.is_empty() {
                    groups.push((span.font.variations.clone(), Vec::new()));
                }
                let count = groups.len();
                for (index, (variations, positioned_glyphs)) in groups.into_iter().enumerate() {
                    spans.push(Span {
                        fill: fill.clone(),
                        stroke: span.stroke.clone(),
                        paint_order: span.paint_order,
                        font_size: span.font_size,
                        variations,
                        font_optical_sizing: span.font_optical_sizing,
                        visible: span.visible,
                        linear_light: span.linear_light,
                        positioned_glyphs,
                        underline: if index == 0 { underline.take() } else { None },
                        overline: if index == 0 { overline.take() } else { None },
                        line_through: if index + 1 == count {
                            line_through.take()
                        } else {
                            None
                        },
                        shadows: span.style.text_paint.shadows.clone(),
                        emphasis: if index + 1 == count {
                            emphasis.take()
                        } else {
                            None
                        },
                    });
                }
            }
        }

        char_offset += chunk.text.chars().count();

        if text_node.writing_mode == WritingMode::TopToBottom {
            if let TextFlow::Linear = chunk.text_flow {
                std::mem::swap(&mut curr_pos.0, &mut curr_pos.1);
            }
        }

        last_x = x + curr_pos.0;
        last_y = y + curr_pos.1;
    }

    let bbox = bbox.to_non_zero_rect()?;

    Some((spans, bbox, text_node.clip))
}

/// CSS Text Decoration 3 §3: the emphasis marks of `span`, one centred over or under each of
/// its marked typographic character units with its content area abutting the span's, as the
/// document's text places them: the mark string in the span's font at half size with the
/// `ruby` feature, or the synthesized shape no font has, filled in `text-emphasis-color`.
/// Vertical text keeps marks upright beside its column.
fn emphasis_marks<'c>(
    span: &TextSpan,
    clusters: impl Iterator<Item = &'c GlyphCluster>,
    font: &ResolvedFont,
    writing_mode: WritingMode,
    span_ts: Transform,
    (fonts_cache, resolver, fontdb): (&FontsCache, &dyn FontProvider, &mut Arc<Database>),
) -> Option<Path> {
    use crate::svg::usvg::text::flatten::DatabaseExt as _;
    use crate::text::emphasis;
    let style = &span.style;
    let text = style.text_paint.emphasis.mark()?;
    let marked: Vec<&GlyphCluster> = clusters
        .filter(|cluster| {
            cluster.visible
                && cluster.glyphs.first().is_some_and(|glyph| {
                    emphasis::marked(&glyph.text, style.text_paint.emphasis_skip)
                })
        })
        .collect();
    if marked.is_empty() {
        return None;
    }
    let size = span.font_size.get();
    let mark_size = size * emphasis::MARK_SCALE;
    let mark_chunk = TextChunk {
        x: None,
        y: None,
        anchor: TextAnchor::Start,
        rtl: false,
        spans: vec![TextSpan {
            start: 0,
            end: text.len(),
            font_size: NonZeroPositiveF32::new(mark_size)?,
            style: Arc::new(emphasis::mark_style(style)),
            letter_spacing: 0.0,
            word_spacing: 0.0,
            ..span.clone()
        }],
        text_flow: TextFlow::Linear,
        text: text.clone(),
        line_breaks: 0,
        inline_offset: 0.0,
        hanging: [0.0; 2],
        soft_wrapped: false,
        overflowing: false,
    };
    let shaped = process_chunk(
        &mark_chunk,
        WritingMode::LeftToRight,
        fonts_cache,
        resolver,
        fontdb,
    )?;
    let missing = shaped
        .iter()
        .flat_map(|cluster| &cluster.glyphs)
        .any(|glyph| glyph.id.0 == 0);
    let synthesized = style.text_paint.emphasis.shape().filter(|_| missing);
    let vertical = writing_mode == WritingMode::TopToBottom;
    let (base, mark) = if vertical {
        (font.upright_metrics, font.upright_metrics)
    } else {
        (font.metrics, font.metrics)
    };
    let [ascent, descent] = [base.ascent * size, base.descent * size];
    let [mark_ascent, mark_descent] = [mark.ascent * mark_size, mark.descent * mark_size];
    let baseline = if style.text_paint.emphasis_position.under(style) {
        descent + mark_ascent
    } else {
        -(ascent + mark_descent)
    };
    let middle = baseline - f32::midpoint(mark_ascent, -mark_descent);
    let width: f32 = shaped.iter().map(|cluster| cluster.advance).sum();
    let mut builder = tiny_skia_path::PathBuilder::new();
    for cluster in marked {
        let centre = cluster.advance / 2.0;
        // Marks stay upright beside a vertical column.
        let place = |transform: Transform| {
            let upright = if vertical {
                Transform::from_rotate_at(-VERTICAL_ROTATION, centre, middle)
            } else {
                Transform::default()
            };
            span_ts
                .pre_concat(cluster.transform)
                .pre_concat(upright)
                .pre_concat(transform)
        };
        if let Some((shape, filled)) = synthesized {
            let contours = emphasis::synthesized(shape, filled, [centre, middle], mark_size);
            if let Some(path) = crate::paint::shape_outline(&crate::paint::Shape::Path {
                contours,
                even_odd: true,
                clip: None,
            })
            .and_then(|path| path.transform(place(Transform::default())))
            {
                builder.push_path(&path);
            }
            continue;
        }
        let mut pen = centre - width / 2.0;
        for mark_cluster in &shaped {
            for glyph in &mark_cluster.glyphs {
                let positioned = PositionedGlyph {
                    span_ts: place(Transform::from_translate(pen, baseline)),
                    cluster_ts: mark_cluster.transform,
                    ..glyph.clone()
                };
                for copy in positioned.copies() {
                    if let Some(path) = fontdb
                        .outline(glyph.font, glyph.id)
                        .and_then(|path| path.transform(copy.outline_transform()))
                    {
                        builder.push_path(&path);
                    }
                }
            }
            pen += mark_cluster.advance;
        }
    }
    let fill = Fill {
        rule: FillRule::EvenOdd,
        ..Fill::color(style.text_paint.emphasis_color.used)
    };
    Path::new(
        String::new(),
        span.visible,
        Some(fill),
        None,
        PaintOrder::default(),
        ShapeRendering::default(),
        Arc::new(builder.finish()?),
        Transform::default(),
    )
}

/// The clusters of `chunk`, shaped, oriented and spaced by `letter-spacing` and
/// `word-spacing`, but not yet positioned.
pub(crate) fn spaced_clusters(
    text: &Text,
    chunk: &TextChunk,
    fonts_cache: &FontsCache,
    resolver: &dyn FontProvider,
    fontdb: &mut Arc<Database>,
) -> Option<Vec<GlyphCluster>> {
    let mut clusters = process_chunk(chunk, text.writing_mode, fonts_cache, resolver, fontdb)?;
    apply_writing_mode(text.writing_mode, &mut clusters);
    apply_letter_spacing(chunk, &mut clusters);
    apply_word_spacing(chunk, &mut clusters);
    apply_autospace(chunk, &mut clusters, fonts_cache, resolver, fontdb);
    apply_spacing_trim(chunk, &mut clusters, fonts_cache, resolver, fontdb);
    Some(clusters)
}

/// CSS Text 4 §8.5 `text-spacing-trim`: the fullwidth punctuation of the chunk's line set
/// half-width as the document's lines set it, in the logical order of its characters. A line
/// starting the text or following a forced break keeps its first opening punctuation where
/// `space-first` does, and closing punctuation ending a line that does not otherwise fit its
/// wrapping area is trimmed where the value allows only that.
fn apply_spacing_trim(
    chunk: &TextChunk,
    clusters: &mut [GlyphCluster],
    fonts_cache: &FontsCache,
    resolver: &dyn FontProvider,
    fontdb: &Database,
) {
    use crate::text::spacing_trim::{self, Trimming};
    // Fullwidth punctuation, middle dots and ideographic spaces are all outside ASCII.
    if chunk.text.is_ascii()
        || chunk
            .spans
            .iter()
            .all(|span| span.style.text_spacing.trim == crate::style::TextSpacingTrim::SpaceAll)
    {
        return;
    }
    let mut order: Vec<usize> = (0..clusters.len())
        .filter(|&index| !clusters[index].glyphs.is_empty())
        .collect();
    order.sort_by_key(|&index| clusters[index].byte_idx.value());
    // Each cluster's face, as the font of its first glyph, and the span setting it.
    let faces: Vec<Option<(Arc<ResolvedFont>, &TextSpan)>> = order
        .iter()
        .map(|&index| {
            let cluster = &clusters[index];
            let span = chunk_span_at(chunk, cluster.byte_idx)?;
            let font = fonts_cache
                .values()
                .find(|font| font.id == cluster.glyphs[0].font)
                .cloned()
                .or_else(|| {
                    let id = cluster.glyphs[0].font;
                    fontdb
                        .load_font(id, resolver.metric_overrides(id))
                        .map(Arc::new)
                })?;
            Some((font, span))
        })
        .collect();
    let settings = |position: usize, clusters: &[GlyphCluster], face: &rustybuzz::Face<'_>| {
        let cluster = &clusters[order[position]];
        let (font, span) = faces[position].as_ref()?;
        let upem = f32::from(font.units_per_em.get());
        let mut ink: Option<[f32; 2]> = None;
        for glyph in &cluster.glyphs {
            let Some(bounds) = face.glyph_bounding_box(glyph.id) else {
                continue;
            };
            // The box along the cluster's inline axis, in em from its origin.
            let to_cluster = glyph.glyph_ts.pre_scale(1.0, -1.0);
            for (x, y) in [
                (bounds.x_min, bounds.y_min),
                (bounds.x_max, bounds.y_max),
                (bounds.x_min, bounds.y_max),
                (bounds.x_max, bounds.y_min),
            ] {
                let mut point = tiny_skia_path::Point::from_xy(f32::from(x), f32::from(y));
                to_cluster.map_point(&mut point);
                let along = point.x / upem;
                ink = Some(ink.map_or([along; 2], |[left, right]| {
                    [left.min(along), right.max(along)]
                }));
            }
        }
        let metrics = if cluster.vertical {
            font.upright_metrics
        } else {
            font.metrics
        };
        Some((
            cluster
                .glyphs
                .iter()
                .map(|glyph| glyph.x_advance)
                .sum::<f32>(),
            ink,
            metrics.size_metric(crate::fonts::SizeMetric::IcWidth),
            *span,
        ))
    };
    let with_face = |position: usize, act: &mut dyn FnMut(&rustybuzz::Face<'_>)| {
        if let Some((font, _)) = &faces[position] {
            fontdb.with_face_data(font.id, |data, index| {
                if let Some(face) = rustybuzz::Face::from_slice(data, index) {
                    act(&face);
                }
            });
        }
    };
    let mut trimmings = Vec::with_capacity(order.len());
    for (position, &index) in order.iter().enumerate() {
        let cluster = &clusters[index];
        let Ok(class) = spacing_trim::classify(cluster.codepoint, || {
            let mut drawn_left = false;
            with_face(position, &mut |face| {
                drawn_left = settings(position, clusters, face)
                    .is_some_and(|(advance, ink, _, _)| spacing_trim::left_half(ink, advance));
            });
            Ok::<_, std::convert::Infallible>(drawn_left)
        });
        let span = chunk_span_at(chunk, cluster.byte_idx);
        trimmings.push(Trimming {
            class,
            size: span.map_or(0.0, |span| span.font_size.get()),
            trim: span.map_or_else(Default::default, |span| span.style.text_spacing.trim),
        });
    }
    let (mut trimmed, overflow) = spacing_trim::halved(&trimmings, !chunk.soft_wrapped);
    if chunk.overflowing {
        trimmed.extend(overflow);
    }
    for position in trimmed {
        let mut kerning = None;
        with_face(position, &mut |face| {
            let Some((advance, ink, full, span)) = settings(position, clusters, face) else {
                return;
            };
            let cluster = &clusters[order[position]];
            let language = crate::text::shaping_language(face, &span.style);
            kerning = spacing_trim::half_width(
                &spacing_trim::Setting {
                    face,
                    text: &cluster.glyphs[0].text,
                    rtl: cluster.rtl,
                    language: language.as_ref(),
                    advance,
                    ink,
                    full,
                },
                trimmings[position].class,
            );
        });
        let Some(kerning) = kerning else {
            continue;
        };
        let cluster = &mut clusters[order[position]];
        let size = cluster.glyphs[0].font_size;
        for glyph in &mut cluster.glyphs {
            let offset = kerning.offset * f32::from(glyph.units_per_em);
            glyph.glyph_ts = Transform::from_translate(offset, 0.0).pre_concat(glyph.glyph_ts);
        }
        cluster.advance += kerning.advance * size;
        cluster.width += kerning.advance * size;
    }
}

/// The clusters of `text` set alone in `span`'s style, horizontally, in `chunk`.
pub(crate) fn styled_clusters(
    chunk: &TextChunk,
    span: &TextSpan,
    text: &str,
    fonts_cache: &FontsCache,
    resolver: &dyn FontProvider,
    fontdb: &mut Arc<Database>,
) -> Option<Vec<GlyphCluster>> {
    let chunk = chunk.holding(
        text.to_owned(),
        vec![TextSpan {
            start: 0,
            end: text.len(),
            ..span.clone()
        }],
    );
    process_chunk(
        &chunk,
        WritingMode::LeftToRight,
        fonts_cache,
        resolver,
        fontdb,
    )
}

/// The element whose `text-autospace` spaces a boundary: a span's own, or an ancestor's.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Owner {
    Span(usize),
    Element(usize),
}

/// CSS Text 4 §8.4 `text-autospace`: the spacing the innermost element containing the boundary
/// of two visually adjacent clusters of `chunk` inserts after the first, measured in that
/// element's font (§8.4.2); a single space between them is removed where the spacing replaces
/// it. SVG text has no box edges or atomic inlines that keep characters apart.
fn apply_autospace(
    chunk: &TextChunk,
    clusters: &mut [GlyphCluster],
    fonts_cache: &FontsCache,
    resolver: &dyn FontProvider,
    fontdb: &mut Arc<Database>,
) {
    use crate::text::autospace::{Spacing, replaceable};
    let span_index = |cluster: &GlyphCluster| chunk_span_index(chunk, cluster.byte_idx);
    let replaceable_at = |cluster: Option<&GlyphCluster>| {
        cluster
            .and_then(|cluster| cluster.glyphs.first())
            .is_some_and(|glyph| replaceable(&glyph.text))
    };
    let mut spacings: HashMap<Owner, (Spacing<'_>, TextSpan)> = HashMap::new();
    let mut insertions = Vec::new();
    for left in 0..clusters.len() {
        let (right, removed) = if replaceable_at(clusters.get(left + 1)) {
            (left + 2, Some(left + 1))
        } else {
            (left + 1, None)
        };
        let (Some(first), Some(second)) = (clusters.get(left), clusters.get(right)) else {
            continue;
        };
        let (Some(first_span), Some(second_span)) = (span_index(first), span_index(second)) else {
            continue;
        };
        let owner = if first_span == second_span {
            Owner::Span(first_span)
        } else {
            let [before, after] =
                [first_span, second_span].map(|index| &chunk.spans[index].ancestry);
            match before
                .iter()
                .zip(after.iter())
                .take_while(|(before, after)| before.element == after.element)
                .last()
            {
                Some((ancestor, _)) => Owner::Element(ancestor.element),
                None => continue,
            }
        };
        let (spacing, measuring) = spacings.entry(owner).or_insert_with(|| {
            let span = &chunk.spans[first_span];
            match owner {
                Owner::Span(_) => (Spacing::new(&span.style), span.clone()),
                Owner::Element(element) => {
                    let ancestor = span
                        .ancestry
                        .iter()
                        .find(|ancestor| ancestor.element == element)
                        .expect("the owner is an ancestor of the span");
                    (
                        Spacing::new(&ancestor.style),
                        TextSpan {
                            style: ancestor.style.clone(),
                            font: ancestor.font.clone(),
                            font_size: NonZeroPositiveF32::new(ancestor.font_size)
                                .unwrap_or(span.font_size),
                            letter_spacing: 0.0,
                            word_spacing: 0.0,
                            ..span.clone()
                        },
                    )
                }
            }
        });
        if removed.is_some() && !spacing.replaces() {
            continue;
        }
        let (before, after) = if first.byte_idx.value() <= second.byte_idx.value() {
            (first.codepoint, second.codepoint)
        } else {
            (second.codepoint, first.codepoint)
        };
        let Some(gap) = spacing.gap(before, after) else {
            continue;
        };
        let size = measuring.font_size.get();
        let Ok(width) = spacing.width(gap, size, |character| {
            Ok::<_, std::convert::Infallible>(
                styled_clusters(
                    chunk,
                    measuring,
                    character.encode_utf8(&mut [0; 4]),
                    fonts_cache,
                    resolver,
                    fontdb,
                )
                .filter(|clusters| {
                    clusters
                        .iter()
                        .flat_map(|cluster| &cluster.glyphs)
                        .all(|glyph| glyph.id.0 != 0)
                })
                .map(|clusters| clusters.iter().map(|cluster| cluster.advance).sum()),
            )
        });
        insertions.push((left, removed, width));
    }
    for (left, removed, width) in insertions {
        clusters[left].advance += width;
        if let Some(removed) = removed {
            clusters[removed].advance = 0.0;
        }
    }
}

/// The indices of the clusters of `chunk` in each of its spans, in visual order.
fn owned_clusters(chunk: &TextChunk, clusters: &[GlyphCluster]) -> Vec<Vec<usize>> {
    let mut owned = vec![Vec::new(); chunk.spans.len()];
    for (index, cluster) in clusters.iter().enumerate() {
        if let Some(span) = chunk_span_index(chunk, cluster.byte_idx) {
            owned[span].push(index);
        }
    }
    owned
}

/// The extent of the font metrics of the visible clusters, not their glyph shapes, through
/// `transform`.
fn clusters_extent(clusters: &[GlyphCluster], transform: Transform) -> Option<NonZeroRect> {
    let mut bboxes_builder = tiny_skia_path::PathBuilder::new();
    for cluster in clusters.iter().filter(|cluster| cluster.visible) {
        let advance = if cluster.advance <= 0.0 {
            1.0
        } else {
            cluster.advance
        };
        if let Some(r) = NonZeroRect::from_xywh(0.0, -cluster.ascent, advance, cluster.height())
            .and_then(|r| r.transform(cluster.transform()))
        {
            bboxes_builder.push_rect(r.to_rect());
        }
    }
    bboxes_builder
        .finish()?
        .transform(transform)?
        .compute_tight_bounds()?
        .to_non_zero_rect()
}

/// The pieces of the decoration lines over the clusters a span `owns`, which split where a relative
/// shift moves a cluster or `skipped` keeps lines from it. A skipped piece takes the spacing
/// after the cluster before it too, as the document's decorations skip spacers.
fn collect_decoration_spans(
    owns: &[usize],
    clusters: &[GlyphCluster],
    skipped: &[bool],
) -> Vec<DecorationSpan> {
    let mut spans = Vec::new();
    let mut current: Option<DecorationSpan> = None;
    for &index in owns {
        let cluster = &clusters[index];
        // A cluster of another span ends the piece.
        if current
            .as_ref()
            .is_some_and(|piece| piece.clusters.end != index)
        {
            spans.extend(current.take());
        }
        if cluster.has_relative_shift {
            spans.extend(current.take());
        }
        if !skipped[index] {
            match &mut current {
                Some(piece) => {
                    piece.width += cluster.advance;
                    piece.clusters.end = index + 1;
                }
                None => {
                    current = Some(DecorationSpan {
                        width: cluster.advance,
                        transform: cluster.transform,
                        clusters: index..index + 1,
                    });
                }
            }
        } else if let Some(mut piece) = current.take() {
            let before = &clusters[index - 1];
            piece.width -= before.advance - before.width;
            spans.push(piece);
        }
    }
    spans.extend(current);
    spans
}

/// Which of `clusters`, the chunk's in visual order, `text-decoration-skip` keeps decoration
/// lines from (CSS Text Decoration 4 §2.10.4): spacers under `spaces` of their element, and
/// those starting or ending the chunk's line under `leading-spaces` or `trailing-spaces`. SVG
/// text has neither the atomic inlines nor the box decorations that `objects` and
/// `box-decoration` skip (SVG 2 §11.10).
fn skipped_clusters(chunk: &TextChunk, clusters: &[GlyphCluster]) -> Vec<bool> {
    let skip = |cluster: &GlyphCluster| {
        chunk_span_at(chunk, cluster.byte_idx)
            .map(|span| span.style.text_decorations.skip)
            .unwrap_or_default()
    };
    let spacer = |cluster: &GlyphCluster| crate::text::decoration::is_spacer(cluster.codepoint);
    let mut skipped: Vec<bool> = clusters
        .iter()
        .map(|cluster| spacer(cluster) && skip(cluster).spaces)
        .collect();
    let forwards: Vec<usize> = (0..clusters.len()).collect();
    let backwards: Vec<usize> = forwards.iter().rev().copied().collect();
    let (start, end) = if chunk.rtl {
        (backwards, forwards)
    } else {
        (forwards, backwards)
    };
    for (order, leading) in [(start, true), (end, false)] {
        for index in order {
            let cluster = &clusters[index];
            if !spacer(cluster) {
                break;
            }
            let skip = skip(cluster);
            skipped[index] |= if leading {
                skip.leading_spaces
            } else {
                skip.trailing_spaces
            };
        }
    }
    skipped
}

/// The first and last spans, by chunk and span index, holding each element's text.
struct ElementSpans {
    extents: HashMap<usize, [(usize, usize); 2]>,
    /// The text's own first and last spans, which hold the text of elements outside it.
    whole: [(usize, usize); 2],
}
impl ElementSpans {
    fn of(text: &Text) -> Self {
        let mut extents = HashMap::new();
        let mut whole: Option<[(usize, usize); 2]> = None;
        for (chunk_index, chunk) in text.chunks.iter().enumerate() {
            for (span_index, span) in chunk.spans.iter().enumerate() {
                let at = (chunk_index, span_index);
                for ancestor in span.ancestry.iter() {
                    extents
                        .entry(ancestor.element)
                        .and_modify(|extent: &mut [(usize, usize); 2]| extent[1] = at)
                        .or_insert([at; 2]);
                }
                whole.get_or_insert([at; 2])[1] = at;
            }
        }
        Self {
            extents,
            whole: whole.unwrap_or_default(),
        }
    }
    /// Whether the span at `at` holds the start and the end of `element`'s text, where the
    /// content edges of its box lie.
    fn edges(&self, element: usize, at: (usize, usize)) -> [bool; 2] {
        self.extents
            .get(&element)
            .unwrap_or(&self.whole)
            .map(|edge| edge == at)
    }
}

/// The pieces of a span's decoration lines, the chunk's clusters they lie over, and whether
/// the span holds the start and end edges of the declaring element's box.
pub(crate) struct Decorated<'a> {
    spans: &'a [DecorationSpan],
    clusters: &'a [GlyphCluster],
    edges: [bool; 2],
}

/// The `line` a span's decoration draws, as the HTML text's lines are placed and styled (CSS
/// Text Decoration 4 §2.3–§2.7) from the span's font, around its central baseline in vertical
/// text. The declaring element's fill and stroke paint it (SVG 2 §11.11), a
/// `text-decoration-color` other than `currentcolor` filling it instead; error indications
/// keep their user agent appearance.
pub(crate) fn convert_decoration(
    line: DecorationLine,
    span: &TextSpan,
    font: &ResolvedFont,
    decoration: &TextDecorationStyle,
    decorated: Decorated<'_>,
    writing_mode: WritingMode,
    transform: Transform,
    (resolver, fontdb): (&dyn FontProvider, &mut Arc<Database>),
) -> Option<Path> {
    use crate::style::POINTS_PER_PIXEL;

    // The declaring element's style measures in points.
    let size = span.font_size.get() * POINTS_PER_PIXEL;
    let thickness = font.underline_thickness(size);
    let metrics = match writing_mode {
        WritingMode::LeftToRight => crate::text::decoration::DecoratingFont {
            underline: [-font.underline_position(size), thickness],
            strikeout: [-font.line_through_position(size), thickness],
            ascent: font.ascent(size),
            descent: -font.descent(size),
        },
        WritingMode::TopToBottom => {
            let half = font.height(size) / 2.0;
            crate::text::decoration::DecoratingFont {
                underline: [half, thickness],
                strikeout: [-thickness / 2.0, thickness],
                ascent: half,
                descent: half,
            }
        }
    };
    let placed = metrics.place(line, &decoration.declared, &decoration.style);
    // CSS Text Decoration 4 `text-decoration-inset`, at the declaring box's content edges on
    // its start and end sides.
    let [start, end] = crate::text::decoration::inset_lengths(&decoration.declared, placed[1])
        .map(|length| length / POINTS_PER_PIXEL);
    let [top, thickness] = placed.map(|length| length / POINTS_PER_PIXEL);
    let (style, color) = decoration.declared.paint();
    let [mut left, mut right] = [(decorated.edges[0], start), (decorated.edges[1], end)]
        .map(|(edge, inset)| if edge { inset } else { 0.0 });
    if decoration.style.used_rtl() {
        std::mem::swap(&mut left, &mut right);
    }
    let last = decorated.spans.len().saturating_sub(1);
    let skip_ink = decoration.declared.skip_ink && line != DecorationLine::LineThrough;

    let mut builder = tiny_skia_path::PathBuilder::new();
    for (index, dec_span) in decorated.spans.iter().enumerate() {
        let x = if index == 0 { left } else { 0.0 };
        let width = dec_span.width - x - if index == last { right } else { 0.0 };
        if width <= 0.0 {
            continue;
        }
        let paint = crate::paint::DecorationPaint {
            line,
            style,
            color: crate::paint::Color::BLACK,
            rect: crate::paint::Rect {
                x,
                y: top,
                width,
                height: thickness,
            },
            owner: None,
            positioned: None,
        };
        let pieces = if skip_ink {
            let Ok(pieces) = crate::text::decoration::interrupted(paint, |band| {
                Ok::<_, std::convert::Infallible>(glyph_ink(
                    span,
                    &decorated.clusters[dec_span.clusters.clone()],
                    dec_span.transform,
                    band,
                    (resolver, &mut *fontdb),
                ))
            });
            pieces
        } else {
            vec![paint]
        };
        for fill in pieces.iter().flat_map(crate::paint::DecorationPaint::fills) {
            if let Some(path) = crate::paint::shape_outline(&fill.shape)
                .and_then(|path| path.transform(dec_span.transform))
            {
                builder.push_path(&path);
            }
        }
    }
    let path_data = builder.finish()?.transform(transform)?;

    let (fill, stroke) = if decoration.declared.lines.error.is_some() {
        (Some(Fill::color(color)), None)
    } else if decoration.declared.current_color {
        (decoration.fill.clone(), decoration.stroke.clone())
    } else {
        (Some(Fill::color(color)), decoration.stroke.clone())
    };
    Path::new(
        String::new(),
        span.visible,
        fill,
        stroke,
        PaintOrder::default(),
        ShapeRendering::default(),
        Arc::new(path_data),
        Transform::default(),
    )
}

/// A text decoration span.
///
/// Basically a horizontal line, that will be used for underline, overline and line-through.
/// It doesn't have a height, since it depends on the Font metrics.
#[derive(Clone)]
pub(crate) struct DecorationSpan {
    pub(crate) width: f32,
    pub(crate) transform: Transform,
    /// The chunk's clusters it lies over.
    clusters: std::ops::Range<usize>,
}

/// The sorted, disjoint extents along a decoration piece, whose frame `frame` places, that the
/// glyph ink of `clusters` of `span` puts in its band `[top, bottom]` (CSS Text Decoration 4
/// §2.10.5).
fn glyph_ink(
    span: &TextSpan,
    clusters: &[GlyphCluster],
    frame: Transform,
    band: [f32; 2],
    (resolver, fontdb): (&dyn FontProvider, &mut Arc<Database>),
) -> Vec<[f32; 2]> {
    use crate::svg::usvg::text::flatten::DatabaseExt;
    use rustybuzz::ttf_parser::OutlineBuilder;
    let Some(inverse) = frame.invert() else {
        return Vec::new();
    };
    let mut extents = Vec::new();
    for cluster in clusters {
        for glyph in cluster.glyphs.iter().flat_map(PositionedGlyph::copies) {
            let Some(outline) = fontdb
                .outline_with_variations(
                    glyph.font,
                    glyph.id,
                    &resolver.variations(&span.font, glyph.font),
                    glyph.font_size(),
                    span.font_optical_sizing,
                )
                .and_then(|outline| {
                    outline.transform(
                        inverse
                            .pre_concat(cluster.transform)
                            .pre_concat(glyph.outline_transform()),
                    )
                })
            else {
                continue;
            };
            let mut polygons = crate::text::skip_ink::Polygons::default();
            for segment in outline.segments() {
                match segment {
                    tiny_skia_path::PathSegment::MoveTo(point) => {
                        polygons.move_to(point.x, point.y);
                    }
                    tiny_skia_path::PathSegment::LineTo(point) => {
                        polygons.line_to(point.x, point.y);
                    }
                    tiny_skia_path::PathSegment::QuadTo(control, point) => {
                        polygons.quad_to(control.x, control.y, point.x, point.y);
                    }
                    tiny_skia_path::PathSegment::CubicTo(first, second, point) => {
                        polygons.curve_to(first.x, first.y, second.x, second.y, point.x, point.y)
                    }
                    tiny_skia_path::PathSegment::Close => polygons.close(),
                }
            }
            extents.extend(crate::text::skip_ink::glyph_extent(
                &polygons.contours,
                band,
            ));
        }
    }
    crate::text::skip_ink::merge(extents)
}

/// Resolves clusters positions.
///
/// Mainly sets the `transform` property.
///
/// Returns the last text position. The next text chunk should start from that position.
fn resolve_clusters_positions(
    text: &Text,
    chunk: &TextChunk,
    char_offset: usize,
    line: (&LineBoxes, usize),
    fonts_cache: &FontsCache,
    clusters: &mut [GlyphCluster],
) -> (f32, f32) {
    match chunk.text_flow {
        TextFlow::Linear => resolve_clusters_positions_horizontal(
            text,
            chunk,
            char_offset,
            text.writing_mode,
            clusters,
        ),
        TextFlow::Path(ref path) => resolve_clusters_positions_path(
            text,
            chunk,
            char_offset,
            path,
            line,
            fonts_cache,
            clusters,
        ),
    }
}

/// CSS Text 4 §9.2 `hanging-punctuation` on a line of pre-formatted text, a chunk of `text`
/// with `clusters`: the advance of the opening punctuation starting the text's first line and
/// of the stop, comma or closing punctuation ending a line that hang at its logical start and
/// end. Without a wrapping area no line overflows, so `allow-end` hangs nothing.
fn hanging_punctuation(text: &Text, chunk: &TextChunk, clusters: &[GlyphCluster]) -> [f32; 2] {
    use crate::text::hanging::{end_hang, hangs_first};
    let hanging = |cluster: &GlyphCluster| {
        chunk_span_at(chunk, cluster.byte_idx)
            .map(|span| span.style.line_breaking.hanging_punctuation)
            .unwrap_or_default()
    };
    let first = clusters
        .iter()
        .min_by_key(|cluster| cluster.byte_idx.value());
    let last = clusters
        .iter()
        .max_by_key(|cluster| cluster.byte_idx.value());
    let is_first_line = text
        .chunks
        .first()
        .is_some_and(|first| std::ptr::eq(first, chunk));
    let is_last_line = text
        .chunks
        .last()
        .is_some_and(|last| std::ptr::eq(last, chunk));
    let start = first
        .filter(|cluster| is_first_line && hangs_first(hanging(cluster), cluster.codepoint))
        .map_or(0.0, |cluster| cluster.advance);
    let end = last
        .filter(|cluster| {
            end_hang(hanging(cluster), cluster.codepoint, is_last_line) == Some(false)
        })
        .map_or(0.0, |cluster| cluster.advance);
    [start, end]
}

fn clusters_length(clusters: &[GlyphCluster]) -> f32 {
    clusters.iter().fold(0.0, |w, cluster| w + cluster.advance)
}

fn resolve_clusters_positions_horizontal(
    text: &Text,
    chunk: &TextChunk,
    offset: usize,
    writing_mode: WritingMode,
    clusters: &mut [GlyphCluster],
) -> (f32, f32) {
    // What hangs at the line's edges does not count for its alignment.
    let [start, end] = if text.wrapping.is_some() {
        chunk.hanging
    } else {
        hanging_punctuation(text, chunk, clusters)
    };
    let [left, right] = if chunk.rtl {
        [end, start]
    } else {
        [start, end]
    };
    let aligned = clusters_length(clusters) - left - right;
    let mut x = process_anchor(chunk.anchor, aligned) - left;
    let mut y = 0.0;

    let code_points = CodePoints::of(&chunk.text);
    for cluster in clusters {
        let cp = offset + code_points.at(cluster.byte_idx);
        if let (Some(dx), Some(dy)) = (text.dx.get(cp), text.dy.get(cp)) {
            if writing_mode == WritingMode::LeftToRight {
                x += dx;
                y += dy;
            } else {
                y -= dx;
                x += dy;
            }
            cluster.has_relative_shift = !dx.approx_zero_ulps(4) || !dy.approx_zero_ulps(4);
        }

        cluster.transform = cluster.transform.pre_translate(x, y);

        if let Some(angle) = text.rotate.get(cp).cloned() {
            if !angle.approx_zero_ulps(4) {
                cluster.transform = cluster.transform.pre_rotate(angle);
                cluster.has_relative_shift = true;
            }
        }

        x += cluster.advance;
    }

    (x, y)
}

// Baseline resolving in SVG is a mess.
// Not only it's poorly documented, but as soon as you start mixing
// `dominant-baseline` and `alignment-baseline` each application/browser will produce
// different results.
//
// For now, resvg simply tries to match Chrome's output and not the mythical SVG spec output.
//
// See `alignment_baseline_shift` method comment for more details.
pub(crate) fn resolve_baseline(
    span: &TextSpan,
    font: &ResolvedFont,
    writing_mode: WritingMode,
) -> f32 {
    let mut shift = -resolve_baseline_shift(&span.baseline_shift, font, span.font_size.get());

    // TODO: support vertical layout as well
    if writing_mode == WritingMode::LeftToRight {
        if span.alignment_baseline == AlignmentBaseline::Auto
            || span.alignment_baseline == AlignmentBaseline::Baseline
        {
            shift += font.dominant_baseline_shift(span.dominant_baseline, span.font_size.get());
        } else {
            shift += font.alignment_baseline_shift(span.alignment_baseline, span.font_size.get())
                + span.parent_alignment;
        }
    }

    shift
}

fn resolve_baseline_shift(baselines: &[BaselineShift], font: &ResolvedFont, font_size: f32) -> f32 {
    let mut shift = 0.0;
    for baseline in baselines.iter().rev() {
        match baseline {
            BaselineShift::Baseline => {}
            BaselineShift::Subscript => shift -= font.subscript_offset(font_size),
            BaselineShift::Superscript => shift += font.superscript_offset(font_size),
            BaselineShift::Number(n) => shift += n,
        }
    }

    shift
}

fn resolve_clusters_positions_path(
    text: &Text,
    chunk: &TextChunk,
    char_offset: usize,
    path: &TextPath,
    (lines, line): (&LineBoxes, usize),
    fonts_cache: &FontsCache,
    clusters: &mut [GlyphCluster],
) -> (f32, f32) {
    let writing_mode = text.writing_mode;
    let mut last_x = 0.0;
    let mut last_y = 0.0;

    let mut dy = 0.0;

    // In the text path mode, chunk's x/y coordinates provide an additional offset along the path.
    // The X coordinate is used in a horizontal mode, and Y in vertical.
    let chunk_offset = match writing_mode {
        WritingMode::LeftToRight => chunk.x.unwrap_or(0.0),
        WritingMode::TopToBottom => chunk.y.unwrap_or(0.0),
    };

    let start_offset =
        chunk_offset + path.start_offset + process_anchor(chunk.anchor, clusters_length(clusters));

    let code_points = CodePoints::of(&chunk.text);
    let normals = collect_normals(
        text,
        &code_points,
        clusters,
        &path.path,
        char_offset,
        start_offset,
    );
    for (cluster, normal) in clusters.iter_mut().zip(normals) {
        let (x, y, angle) = match normal {
            Some(normal) => (normal.x, normal.y, normal.angle),
            None => {
                // Hide clusters that are outside the text path.
                cluster.visible = false;
                continue;
            }
        };

        // We have to break a decoration line for each cluster during text-on-path.
        cluster.has_relative_shift = true;

        let orig_ts = cluster.transform;

        // Clusters should be rotated by the x-midpoint x baseline position.
        let half_width = cluster.width / 2.0;
        cluster.transform = Transform::default();
        cluster.transform = cluster.transform.pre_translate(x - half_width, y);
        cluster.transform = cluster.transform.pre_rotate_at(angle, half_width, 0.0);

        let cp = char_offset + code_points.at(cluster.byte_idx);
        dy += text.dy.get(cp).cloned().unwrap_or(0.0);

        let baseline_shift = chunk_span_at(chunk, cluster.byte_idx)
            .map(|span| {
                let font = match fonts_cache.get(&span.font) {
                    Some(v) => v,
                    None => return 0.0,
                };
                -lines.baseline(line, span, font, writing_mode)
            })
            .unwrap_or(0.0);

        // Shift only by `dy` since we already applied `dx`
        // during offset along the path calculation.
        if !dy.approx_zero_ulps(4) || !baseline_shift.approx_zero_ulps(4) {
            let shift = kurbo::Vec2::new(0.0, (dy - baseline_shift) as f64);
            cluster.transform = cluster
                .transform
                .pre_translate(shift.x as f32, shift.y as f32);
        }

        if let Some(angle) = text.rotate.get(cp).cloned() {
            if !angle.approx_zero_ulps(4) {
                cluster.transform = cluster.transform.pre_rotate(angle);
            }
        }

        // The possible `lengthAdjust` transform should be applied after text-on-path positioning.
        cluster.transform = cluster.transform.pre_concat(orig_ts);

        last_x = x + cluster.advance;
        last_y = y;
    }

    (last_x, last_y)
}

pub(crate) fn process_anchor(a: TextAnchor, text_width: f32) -> f32 {
    match a {
        TextAnchor::Start => 0.0, // Nothing.
        TextAnchor::Middle => -text_width / 2.0,
        TextAnchor::End => -text_width,
    }
}

pub(crate) struct PathNormal {
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) angle: f32,
}

fn collect_normals(
    text: &Text,
    code_points: &CodePoints,
    clusters: &[GlyphCluster],
    path: &tiny_skia_path::Path,
    char_offset: usize,
    offset: f32,
) -> Vec<Option<PathNormal>> {
    let mut offsets = Vec::with_capacity(clusters.len());
    let mut normals = Vec::with_capacity(clusters.len());
    {
        let mut advance = offset;
        for cluster in clusters {
            // Clusters should be rotated by the x-midpoint x baseline position.
            let half_width = cluster.width / 2.0;

            // Include relative position.
            let cp = char_offset + code_points.at(cluster.byte_idx);
            advance += text.dx.get(cp).cloned().unwrap_or(0.0);

            let offset = advance + half_width;

            // Clusters outside the path have no normals.
            if offset < 0.0 {
                normals.push(None);
            }

            offsets.push(offset as f64);
            advance += cluster.advance;
        }
    }

    let mut prev_mx = path.points()[0].x;
    let mut prev_my = path.points()[0].y;
    let mut prev_x = prev_mx;
    let mut prev_y = prev_my;

    fn create_curve_from_line(px: f32, py: f32, x: f32, y: f32) -> kurbo::CubicBez {
        let line = kurbo::Line::new(
            kurbo::Point::new(px as f64, py as f64),
            kurbo::Point::new(x as f64, y as f64),
        );
        let p1 = line.eval(0.33);
        let p2 = line.eval(0.66);
        kurbo::CubicBez {
            p0: line.p0,
            p1,
            p2,
            p3: line.p1,
        }
    }

    let mut length: f64 = 0.0;
    for seg in path.segments() {
        let curve = match seg {
            tiny_skia_path::PathSegment::MoveTo(p) => {
                prev_mx = p.x;
                prev_my = p.y;
                prev_x = p.x;
                prev_y = p.y;
                continue;
            }
            tiny_skia_path::PathSegment::LineTo(p) => {
                create_curve_from_line(prev_x, prev_y, p.x, p.y)
            }
            tiny_skia_path::PathSegment::QuadTo(p1, p) => kurbo::QuadBez {
                p0: kurbo::Point::new(prev_x as f64, prev_y as f64),
                p1: kurbo::Point::new(p1.x as f64, p1.y as f64),
                p2: kurbo::Point::new(p.x as f64, p.y as f64),
            }
            .raise(),
            tiny_skia_path::PathSegment::CubicTo(p1, p2, p) => kurbo::CubicBez {
                p0: kurbo::Point::new(prev_x as f64, prev_y as f64),
                p1: kurbo::Point::new(p1.x as f64, p1.y as f64),
                p2: kurbo::Point::new(p2.x as f64, p2.y as f64),
                p3: kurbo::Point::new(p.x as f64, p.y as f64),
            },
            tiny_skia_path::PathSegment::Close => {
                create_curve_from_line(prev_x, prev_y, prev_mx, prev_my)
            }
        };

        let arclen_accuracy = {
            let base_arclen_accuracy = 0.5;
            // Accuracy depends on a current scale.
            // When we have a tiny path scaled by a large value,
            // we have to increase out accuracy accordingly.
            let (sx, sy) = text.abs_transform.get_scale();
            // 1.0 acts as a threshold to prevent division by 0 and/or low accuracy.
            base_arclen_accuracy / (sx * sy).sqrt().max(1.0)
        };

        let curve_len = curve.arclen(arclen_accuracy as f64);

        for offset in &offsets[normals.len()..] {
            if *offset >= length && *offset <= length + curve_len {
                let mut offset = curve.inv_arclen(offset - length, arclen_accuracy as f64);
                // some rounding error may occur, so we give offset a little tolerance
                debug_assert!((-1.0e-3..=1.0 + 1.0e-3).contains(&offset));
                offset = offset.clamp(0.0, 1.0);

                let pos = curve.eval(offset);
                let d = curve.deriv().eval(offset);
                let d = kurbo::Vec2::new(-d.y, d.x); // tangent
                let angle = d.atan2().to_degrees() - 90.0;

                normals.push(Some(PathNormal {
                    x: pos.x as f32,
                    y: pos.y as f32,
                    angle: angle as f32,
                }));

                if normals.len() == offsets.len() {
                    break;
                }
            }
        }

        length += curve_len;
        prev_x = curve.p3.x as f32;
        prev_y = curve.p3.y as f32;
    }

    // If path ended and we still have unresolved normals - set them to `None`.
    for _ in 0..(offsets.len() - normals.len()) {
        normals.push(None);
    }

    normals
}

/// The chunk as the document's text sees it: its spans' text and styles inside the inline
/// boxes of their ancestors, in a paragraph of its text element whose base direction is the
/// chunk's (SVG 2 §11.10.2.4).
pub(crate) fn document_paragraph(chunk: &TextChunk) -> crate::document::Paragraph {
    let mut root = chunk
        .spans
        .first()
        .and_then(|span| span.ancestry.first())
        .map_or_else(crate::style::Style::default, |ancestor| {
            (*ancestor.style).clone()
        });
    root.rtl = chunk.rtl;
    root.principal_flow = None;
    let mut boxes: Vec<(
        std::ops::Range<usize>,
        Arc<crate::style::Style>,
        Option<usize>,
    )> = Vec::new();
    let mut elements = Vec::new();
    let mut open: Vec<usize> = Vec::new();
    let mut spans = Vec::with_capacity(chunk.spans.len());
    for span in &chunk.spans {
        let ancestors = span.ancestry.get(1..).unwrap_or_default();
        let shared = open
            .iter()
            .zip(ancestors)
            .take_while(|(index, ancestor)| elements[**index] == ancestor.element)
            .count();
        open.truncate(shared);
        for index in &open {
            boxes[*index].0.end = span.end;
        }
        for ancestor in &ancestors[shared..] {
            boxes.push((
                span.start..span.end,
                ancestor.style.clone(),
                open.last().copied(),
            ));
            elements.push(ancestor.element);
            open.push(boxes.len() - 1);
        }
        spans.push((
            span.start..span.end,
            (*span.style).clone(),
            open.last().copied(),
        ));
    }
    crate::document::Paragraph::styled(chunk.text.clone(), &root, boxes, spans)
}

pub(crate) fn process_chunk(
    chunk: &TextChunk,
    writing_mode: WritingMode,
    fonts_cache: &FontsCache,
    resolver: &dyn FontProvider,
    fontdb: &mut Arc<Database>,
) -> Option<Vec<GlyphCluster>> {
    // CSS Text 3 §2.1: text shapes as `text-transform` cases it, while SVG 2 §11.5 positions
    // the characters before the transform; glyphs map back to the characters they show.
    let glyphs = match CasedChunk::of(chunk) {
        None => shape_chunk(chunk, writing_mode, fonts_cache, resolver, fontdb)?,
        Some(cased) => {
            let cased = cased?;
            let mut glyphs =
                shape_chunk(&cased.chunk, writing_mode, fonts_cache, resolver, fontdb)?;
            cased.to_source(&mut glyphs);
            glyphs
        }
    };

    // Convert glyphs to clusters.
    let mut clusters = Vec::new();
    for (range, byte_idx) in GlyphClusters::new(&glyphs) {
        if let Some(span) = chunk_span_at(chunk, byte_idx) {
            let font_size = span.font_size.get() * glyphs[range.start].scale;
            clusters.push(form_glyph_clusters(&glyphs[range], &chunk.text, font_size));
        }
    }

    Some(clusters)
}

/// A text chunk as `text-transform` cases it, with the source characters each part shows.
struct CasedChunk {
    chunk: TextChunk,
    /// Each source character's byte range, and the range of its cased form.
    units: Vec<(std::ops::Range<usize>, std::ops::Range<usize>)>,
}
impl CasedChunk {
    /// `None` when no span transforms its text; `Some(None)` when casing exceeds the inline
    /// text limit.
    fn of(chunk: &TextChunk) -> Option<Option<Self>> {
        if chunk
            .spans
            .iter()
            .all(|span| span.style.text_transform == crate::style::TextTransform::default())
        {
            return None;
        }
        let runs: Vec<_> = chunk
            .spans
            .iter()
            .map(|span| crate::document::CaseRun {
                range: span.start..span.end,
                transform: span.style.text_transform,
                language: &span.style.language,
                preserve_spaces: !span.style.white_space.collapses_spaces(),
                single_character: span.single_character,
            })
            .collect();
        let Some((text, cased)) = crate::document::transform_text(&chunk.text, &runs) else {
            return Some(None);
        };
        let sources = chunk.spans.iter().flat_map(|span| {
            chunk.text[span.start..span.end]
                .char_indices()
                .map(move |(offset, ch)| span.start + offset..span.start + offset + ch.len_utf8())
        });
        let units: Vec<_> = sources.zip(cased).collect();
        let mut spans = chunk.spans.clone();
        let mut unit = 0;
        let mut position = 0;
        for span in &mut spans {
            let count = chunk.text[span.start..span.end].chars().count();
            span.start = position;
            if count > 0 {
                position = units[unit + count - 1].1.end;
            }
            span.end = position;
            unit += count;
        }
        spans.retain(|span| span.start < span.end);
        Some(Some(Self {
            chunk: chunk.holding(text, spans),
            units,
        }))
    }
    /// Moves glyphs shaped from the cased text to the source characters they show.
    fn to_source(&self, glyphs: &mut [Glyph]) {
        for glyph in glyphs {
            let start = glyph.byte_idx.value();
            let end = start + glyph.cluster_len;
            let first = self.units.partition_point(|(_, cased)| cased.end <= start);
            let last = self
                .units
                .partition_point(|(_, cased)| cased.end < end)
                .max(first);
            let Some(((source, _), (last_source, _))) =
                self.units.get(first).zip(self.units.get(last))
            else {
                continue;
            };
            glyph.byte_idx = ByteIndex::new(source.start);
            glyph.cluster_len = last_source.end - source.start;
        }
    }
}

/// Shapes a chunk's spans with their fonts into glyphs in visual order; `None` where the
/// bidi controls of its inline boxes exceed the inline text limit.
fn shape_chunk(
    chunk: &TextChunk,
    writing_mode: WritingMode,
    fonts_cache: &FontsCache,
    resolver: &dyn FontProvider,
    fontdb: &mut Arc<Database>,
) -> Option<Vec<Glyph>> {
    // CSS Writing Modes 4 §2.4: the embeddings, isolates and overrides of `unicode-bidi` on the
    // text element and its inline boxes, upright typesetting and combinations take part.
    let paragraph = document_paragraph(chunk);
    let bidi = crate::text::bidi::resolve(
        &paragraph,
        &std::cell::Cell::new(Some(crate::document::MAX_INLINE_TEXT_BYTES)),
    )
    .ok()?;
    let orientation_spans = (writing_mode == WritingMode::TopToBottom).then(|| {
        chunk
            .spans
            .iter()
            .map(|span| (span.start..span.end, span.text_orientation))
            .collect::<Vec<_>>()
    });
    let orientations = orientation_spans.as_deref();
    // Each run of spans styled alike shapes once, in the context of the chunk's text; its
    // glyphs follow the visual order of the bidi runs they fall in.
    let mut runs = Vec::new();
    let mut spans = chunk.spans.iter().peekable();
    while let Some(span) = spans.next() {
        let mut end = span.end;
        while let Some(next) = spans.next_if(|next| next.start == end && shapes_alike(span, next)) {
            end = next.end;
        }
        let Some(font) = fonts_cache.get(&span.font).filter(|_| span.start < end) else {
            continue;
        };
        runs.push((
            span.start,
            shape_text(
                &chunk.text,
                &bidi,
                orientations,
                span.start..end,
                font.clone(),
                SpanShaping {
                    request: &span.font,
                    style: &span.style,
                    font_size: span.font_size.get(),
                    optical_sizing: span.font_optical_sizing,
                    tracked: crate::text::shaping::tracked(
                        span.letter_spacing,
                        span.font_size.get(),
                    ),
                },
                resolver,
                fontdb,
            ),
        ));
    }
    if runs.is_empty() {
        return Some(Vec::new());
    }
    let mut visual_runs: Vec<_> = bidi
        .visual_runs(0..chunk.text.len())
        .into_iter()
        .enumerate()
        .map(|(order, (range, rtl))| (range.start, order, rtl))
        .collect();
    visual_runs.sort_unstable();
    let placement = |offset: usize| {
        let index = visual_runs
            .partition_point(|&(start, ..)| start <= offset)
            .saturating_sub(1);
        visual_runs
            .get(index)
            .map_or((0, false), |&(_, order, rtl)| (order, rtl))
    };
    let mut glyphs: Vec<_> = runs
        .into_iter()
        .flat_map(|(start, glyphs)| glyphs.into_iter().map(move |glyph| (start, glyph)))
        .collect();
    // Within a bidi run, the runs of spans follow its direction.
    glyphs.sort_by_cached_key(|(start, glyph)| {
        let (order, rtl) = placement(glyph.byte_idx.value());
        (order, if rtl { usize::MAX - start } else { *start })
    });
    let mut glyphs: Vec<Glyph> = glyphs.into_iter().map(|(_, glyph)| glyph).collect();

    if writing_mode == WritingMode::TopToBottom {
        // Each combination's glyphs replace those of its text where the first of them was;
        // the combinations follow each other in the text.
        let groups = crate::text::combine::groups(&paragraph);
        let mut combined = groups
            .iter()
            .map(|group| combined_glyphs(chunk, group, fonts_cache, resolver, fontdb).map(Some))
            .collect::<Option<Vec<_>>>()?;
        let group_at = |offset: usize| {
            let index = groups.partition_point(|group| group.range.end <= offset);
            groups
                .get(index)
                .filter(|group| group.range.contains(&offset))
                .map(|_| index)
        };
        glyphs = glyphs
            .into_iter()
            .flat_map(|glyph| match group_at(glyph.byte_idx.value()) {
                Some(index) => combined[index].take().unwrap_or_default(),
                None => vec![glyph],
            })
            .collect();
    }

    Some(glyphs)
}

/// CSS Writing Modes 4 §9.1: the glyphs of `group`, a combination in vertical text, composed
/// horizontally in its span's style with the width-specific forms every unit has, compressed to
/// fit 1em and centred upright in one cell of the column, as the document composes them.
fn combined_glyphs(
    chunk: &TextChunk,
    group: &crate::text::combine::Group,
    fonts_cache: &FontsCache,
    resolver: &dyn FontProvider,
    fontdb: &mut Arc<Database>,
) -> Option<Vec<Glyph>> {
    use crate::text::combine;
    use unicode_segmentation::UnicodeSegmentation;
    let span = &chunk.spans[group.span];
    let source = &chunk.text[group.range.clone()];
    let (text, offsets) = combine::composition(source);
    let shape = |style: &crate::style::Style, text: &str, fontdb: &mut Arc<Database>| {
        let composed = TextChunk {
            x: None,
            y: None,
            anchor: TextAnchor::Start,
            rtl: false,
            spans: vec![TextSpan {
                start: 0,
                end: text.len(),
                style: Arc::new(style.clone()),
                ancestry: Arc::default(),
                letter_spacing: 0.0,
                text_orientation: crate::style::logical::TextOrientation::Mixed,
                ..span.clone()
            }],
            text_flow: TextFlow::Linear,
            text: text.to_owned(),
            line_breaks: 0,
            inline_offset: 0.0,
            hanging: [0.0; 2],
            soft_wrapped: false,
            overflowing: false,
        };
        shape_chunk(
            &composed,
            WritingMode::LeftToRight,
            fonts_cache,
            resolver,
            fontdb,
        )
    };
    let plain = combine::horizontal(&span.style);
    let mut glyphs = shape(&plain, &text, fontdb)?;
    if let Some(feature) = combine::width_feature(text.graphemes(true).count()) {
        let mut variant = plain.clone();
        let mut settings = variant.font_features.settings.to_vec();
        settings.push(crate::fonts::FontFeature::on(feature));
        variant.font_features.settings = settings.into();
        let ids = |glyphs: &[Glyph]| {
            glyphs
                .iter()
                .map(|glyph| (glyph.font.id, glyph.id))
                .collect::<Vec<_>>()
        };
        let mut complete = true;
        for unit in text
            .graphemes(true)
            .filter(|unit| !unit.chars().all(char::is_whitespace))
        {
            if ids(&shape(&plain, unit, fontdb)?) == ids(&shape(&variant, unit, fontdb)?) {
                complete = false;
                break;
            }
        }
        if complete {
            glyphs = shape(&variant, &text, fontdb)?;
        }
    }
    let em = span.font_size.get();
    // User units per font unit of `glyph`.
    let unit = |glyph: &Glyph| em * glyph.scale / f32::from(glyph.font.units_per_em.get());
    let width: f32 = glyphs
        .iter()
        .map(|glyph| glyph.width as f32 * unit(glyph))
        .sum();
    let compression = combine::compression(width, em);
    // Upright glyphs place their origins across the column from its centre and down from the
    // cell's top; each face's ascent and descent centre in the cell.
    let mut pen = -width * compression / 2.0;
    let last = glyphs.len().saturating_sub(1);
    for (index, glyph) in glyphs.iter_mut().enumerate() {
        let unit = unit(glyph);
        let size = em * glyph.scale;
        let baseline = (em + glyph.font.ascent(size) + glyph.font.descent(size)) / 2.0;
        let across = pen + glyph.dx as f32 * unit * compression;
        let down = baseline - glyph.dy as f32 * unit;
        pen += glyph.width as f32 * unit * compression;
        glyph.dx = (across / unit).round() as i32;
        glyph.dy = -(down / unit).round() as i32;
        glyph.width = if index == last {
            (em / unit).round() as i32
        } else {
            0
        };
        glyph.vertical = true;
        glyph.compression = compression;
        // Each glyph keeps the source text it shows within the combination's one cluster.
        let start = offsets[glyph.byte_idx.value()];
        let end = offsets[glyph.byte_idx.value() + glyph.cluster_len];
        glyph.text = source[start..end].to_owned();
        glyph.text_offset = start;
        glyph.byte_idx = ByteIndex::new(group.range.start);
        glyph.cluster_len = group.range.len();
    }
    Some(glyphs)
}

/// The line boxes of pre-formatted text, which preserved segment breaks start (SVG 2 §11.6.1),
/// stacked as CSS stacks them (CSS 2.1 §10.8): each spans the half-leading extents of its
/// inline boxes and of the text element's strut above and below the baseline.
struct LineBoxes {
    /// Each line's extents before and after its baseline in the block direction.
    extents: Vec<[f32; 2]>,
    /// How far below its line's baseline each subtree aligned with a line edge puts its root's
    /// baseline, by line and root.
    edges: HashMap<(usize, usize), f32>,
    /// The physical direction lines advance in vertical text: one leftwards, minus one
    /// rightwards.
    progression: f32,
}
impl LineBoxes {
    fn new(text: &Text, fonts: &FontsCache) -> Self {
        let vertical = text.writing_mode == WritingMode::TopToBottom;
        let extents_of = |font: Option<&ResolvedFont>, strut: &crate::svg::usvg::Strut| {
            font.map_or([0.0; 2], |font| {
                line_extents(font, strut.font_size, &strut.style, vertical)
            })
        };
        let strut = |line: usize| {
            let strut = text.line_strut(line);
            extents_of(fonts.get(&strut.font).map(Arc::as_ref), strut)
        };
        let mut extents = vec![strut(0)];
        // Each line's subtrees aligned with an edge: whether the bottom edge, and their
        // extents around their roots' baselines.
        let mut subtrees: Vec<HashMap<usize, (bool, [f32; 2])>> = vec![HashMap::new()];
        for chunk in &text.chunks {
            for _ in 0..chunk.line_breaks {
                extents.push(strut(extents.len()));
                subtrees.push(HashMap::new());
            }
            let line = extents.len() - 1;
            for span in &chunk.spans {
                let Some(font) = fonts.get(&span.font) else {
                    continue;
                };
                let [above, below] =
                    line_extents(font, span.font_size.get(), &span.style, vertical);
                match &span.line_edge {
                    None => {
                        let extent = &mut extents[line];
                        *extent = [extent[0].max(above), extent[1].max(below)];
                    }
                    Some(edge) => {
                        let shift = resolve_baseline(span, font, text.writing_mode);
                        let (_, extent) = subtrees[line].entry(edge.root).or_insert_with(|| {
                            let root = fonts.get(&edge.strut.font).map(Arc::as_ref);
                            (edge.bottom, extents_of(root, &edge.strut))
                        });
                        *extent = [extent[0].max(above - shift), extent[1].max(below + shift)];
                    }
                }
            }
        }
        // CSS 2.1 §10.8: a subtree taller than its line box grows it, and baseline-aligned
        // content stays at the top, as the document's line boxes keep it.
        let mut edges = HashMap::new();
        for (line, trees) in subtrees.iter().enumerate() {
            let extent = &mut extents[line];
            for &(_, [above, below]) in trees.values() {
                extent[1] = extent[1].max(above + below - extent[0]);
            }
            for (&root, &(bottom, [above, below])) in trees {
                let offset = if bottom {
                    extent[1] - below
                } else {
                    above - extent[0]
                };
                edges.insert((line, root), offset);
            }
        }
        let progression =
            if text.strut.style.writing_mode == crate::style::logical::WritingMode::VerticalLr {
                -1.0
            } else {
                1.0
            };
        Self {
            extents,
            edges,
            progression,
        }
    }
    /// The distance from line `line`'s baseline to the next line's.
    fn advance(&self, line: usize) -> f32 {
        self.extents[line][1] + self.extents[line + 1][0]
    }
    /// How far below line `line`'s baseline `span` puts its baseline.
    fn baseline(
        &self,
        line: usize,
        span: &TextSpan,
        font: &ResolvedFont,
        writing_mode: WritingMode,
    ) -> f32 {
        let edge = span.line_edge.as_ref().map_or(0.0, |edge| {
            self.edges.get(&(line, edge.root)).copied().unwrap_or(0.0)
        });
        resolve_baseline(span, font, writing_mode) + edge
    }
}

/// The extents of an inline box of `font` at `size` above and below its baseline, with
/// half the leading its `line-height` adds on each side (CSS 2.1 §10.8.1); vertical text
/// centres its em box.
pub(crate) fn line_extents(
    font: &ResolvedFont,
    size: f32,
    style: &crate::style::Style,
    vertical: bool,
) -> [f32; 2] {
    let metrics = if vertical {
        font.upright_metrics
    } else {
        font.metrics
    };
    let content = [metrics.ascent * size, metrics.descent * size];
    // `line-height` resolves in points; user units are CSS pixels.
    let height = style
        .line_height
        .resolve(size * crate::style::POINTS_PER_PIXEL)
        .map_or(metrics.normal_line_height() * size, |points| {
            points / crate::style::POINTS_PER_PIXEL
        });
    let half_leading = (height - content[0] - content[1]) / 2.0;
    content.map(|extent| extent + half_leading)
}

/// Gives preserved tabs the width to the next tab stop from the line's start, `inline_start`
/// before the chunk (CSS Text 3 §4.2); stops are `tab-size` spaces of the text element apart,
/// with its letter and word spacing. Tabs paint nothing.
pub(crate) fn apply_tab_stops(
    chunk: &TextChunk,
    strut: &crate::svg::usvg::Strut,
    font: &ResolvedFont,
    fonts: &FontsCache,
    inline_start: f32,
    clusters: &mut [GlyphCluster],
) {
    if !clusters.iter().any(|cluster| cluster.codepoint == TAB) {
        return;
    }
    let style = &strut.style;
    let size = strut.font_size;
    let points = |length: f32| length / crate::style::POINTS_PER_PIXEL;
    let space = font.space_advance.unwrap_or_default() * size
        + points(style.letter_spacing)
        + points(
            style
                .word_spacing
                .resolve(size * crate::style::POINTS_PER_PIXEL),
        );
    // Each tab takes its own `tab-size`, and measures half a stop by its own font's "0".
    let stops = |span: &TextSpan| {
        let interval = match span.style.tab_size {
            crate::style::TabSize::Number(count) => count * space,
            crate::style::TabSize::Length(length) => points(length),
        };
        let size = span.font_size.get();
        let ch = match fonts.get(&span.font) {
            Some(font) if !span.style.vertical_font_units()[0] => {
                font.zero_advance
                    .unwrap_or(crate::fonts::FALLBACK_ZERO_ADVANCE_EM)
                    * size
            }
            _ => size,
        };
        (interval, ch)
    };
    let mut position = inline_start;
    let mut visit = |cluster: &mut GlyphCluster| {
        if cluster.codepoint == TAB
            && let Some(span) = chunk_span_at(chunk, cluster.byte_idx)
        {
            let (interval, ch) = stops(span);
            let width = if interval > 0.0 {
                crate::text::tabs::tab_width(interval, position, ch)
            } else {
                0.0
            };
            cluster.width = width;
            cluster.advance = width;
            cluster.glyphs.clear();
        }
        position += cluster.advance;
    };
    if chunk.rtl {
        clusters.iter_mut().rev().for_each(&mut visit);
    } else {
        clusters.iter_mut().for_each(&mut visit);
    }
}

/// A preserved tab (CSS Text 3 §4.1.1).
const TAB: char = '\t';

fn apply_length_adjust(chunk: &TextChunk, clusters: &mut [GlyphCluster]) {
    let is_horizontal = matches!(chunk.text_flow, TextFlow::Linear);

    for span in &chunk.spans {
        let target_width = match span.text_length {
            Some(v) => v,
            None => continue,
        };

        let mut width = 0.0;
        let mut cluster_indexes = Vec::new();
        for i in span.start..span.end {
            if let Some(index) = clusters.iter().position(|c| c.byte_idx.value() == i) {
                cluster_indexes.push(index);
            }
        }
        // Complex scripts can have multi-codepoint clusters therefore we have to remove duplicates.
        cluster_indexes.sort();
        cluster_indexes.dedup();

        for i in &cluster_indexes {
            // Use the original cluster `width` and not `advance`.
            // This method essentially discards any `word-spacing` and `letter-spacing`.
            width += clusters[*i].width;
        }

        if cluster_indexes.is_empty() {
            continue;
        }

        if span.length_adjust == LengthAdjust::Spacing {
            let factor = if cluster_indexes.len() > 1 {
                (target_width - width) / (cluster_indexes.len() - 1) as f32
            } else {
                0.0
            };

            for i in cluster_indexes {
                clusters[i].advance = clusters[i].width + factor;
            }
        } else {
            let factor = target_width / width;
            // Prevent multiplying by zero.
            if factor < 0.001 {
                continue;
            }

            for i in cluster_indexes {
                clusters[i].transform = clusters[i].transform.pre_scale(factor, 1.0);

                // Technically just a hack to support the current text-on-path algorithm.
                if !is_horizontal {
                    clusters[i].advance *= factor;
                    clusters[i].width *= factor;
                }
            }
        }
    }
}

/// Centres sideways glyphs on the vertical column; upright glyphs already have vertical origins.
fn apply_writing_mode(writing_mode: WritingMode, clusters: &mut [GlyphCluster]) {
    if writing_mode != WritingMode::TopToBottom {
        return;
    }
    for cluster in clusters.iter_mut().filter(|cluster| !cluster.vertical) {
        cluster.transform = cluster
            .transform
            .pre_translate(0.0, (cluster.ascent + cluster.descent) / 2.0);
    }
}

/// Applies the `letter-spacing` property to a text chunk clusters.
///
/// [In the CSS spec](https://www.w3.org/TR/css-text-3/#letter-spacing-property).
fn apply_letter_spacing(chunk: &TextChunk, clusters: &mut [GlyphCluster]) {
    // At least one span should have a non-zero spacing.
    if !chunk
        .spans
        .iter()
        .any(|span| !span.letter_spacing.approx_zero_ulps(4))
    {
        return;
    }

    let num_clusters = clusters.len();
    for (i, cluster) in clusters.iter_mut().enumerate() {
        // Spacing must be applied only to characters that belongs to the script
        // that supports spacing.
        // We are checking only the first code point, since it should be enough.
        // https://www.w3.org/TR/css-text-3/#cursive-tracking
        let script = cluster.codepoint.script();
        if script_supports_letter_spacing(script) {
            if let Some(span) = chunk_span_at(chunk, cluster.byte_idx) {
                // A space after the last cluster should be ignored,
                // since it affects the bbox and text alignment.
                if i != num_clusters - 1 {
                    cluster.advance += span.letter_spacing;
                }

                // If the cluster advance became negative - clear it.
                // This is an UB so we can do whatever we want, and we mimic Chrome's behavior.
                if !cluster.advance.is_valid_length() {
                    cluster.width = 0.0;
                    cluster.advance = 0.0;
                    cluster.glyphs = vec![];
                }
            }
        }
    }
}

/// Applies the `word-spacing` property to a text chunk clusters.
///
/// [In the CSS spec](https://www.w3.org/TR/css-text-3/#propdef-word-spacing).
fn apply_word_spacing(chunk: &TextChunk, clusters: &mut [GlyphCluster]) {
    // At least one span should have a non-zero spacing.
    if !chunk
        .spans
        .iter()
        .any(|span| !span.word_spacing.approx_zero_ulps(4))
    {
        return;
    }

    for cluster in clusters {
        if is_word_separator_characters(cluster.codepoint) {
            if let Some(span) = chunk_span_at(chunk, cluster.byte_idx) {
                // Technically, word spacing 'should be applied half on each
                // side of the character', but it doesn't affect us in any way,
                // so we are ignoring this.
                cluster.advance += span.word_spacing;

                // After word spacing, `advance` can be negative.
            }
        }
    }
}

fn form_glyph_clusters(glyphs: &[Glyph], text: &str, font_size: f32) -> GlyphCluster {
    debug_assert!(!glyphs.is_empty());

    let mut width = 0.0;
    let mut x: f32 = 0.0;

    let mut positioned_glyphs = vec![];

    for glyph in glyphs {
        let sx = glyph.font.scale(font_size);

        // Apply offset.
        //
        // The first glyph in the cluster will have an offset from 0x0,
        // but the later one will have an offset from the "current position".
        // So we have to keep an advance.
        // TODO: should be done only inside a single text span
        let ts = if glyph.vertical {
            Transform::from_translate(x, 0.0)
                .pre_rotate(-VERTICAL_ROTATION)
                .pre_translate(glyph.dx as f32, -glyph.dy as f32)
                .pre_scale(glyph.compression, 1.0)
        } else {
            Transform::from_translate(x + glyph.dx as f32, -glyph.dy as f32)
        }
        // The synthetic oblique shears the glyph about its baseline, y being downwards.
        .pre_concat(Transform::from_row(
            1.0,
            0.0,
            -glyph.paint.oblique_skew,
            1.0,
            0.0,
            0.0,
        ));

        positioned_glyphs.push(PositionedGlyph {
            glyph_ts: ts,
            // Will be set later.
            cluster_ts: Transform::default(),
            // Will be set later.
            span_ts: Transform::default(),
            units_per_em: glyph.font.units_per_em.get(),
            font_size,
            font: glyph.font.id,
            cluster: glyph.byte_idx.value() + glyph.text_offset,
            x_advance: glyph.width as f32 / f32::from(glyph.font.units_per_em.get()),
            text: glyph.text.clone(),
            id: glyph.id,
            paint: glyph.paint.clone(),
        });

        x += glyph.width as f32;

        let glyph_width = glyph.width as f32 * sx;
        if glyph_width > width {
            width = glyph_width;
        }
    }

    let byte_idx = glyphs[0].byte_idx;
    let font = glyphs[0].font.clone();
    GlyphCluster {
        byte_idx,
        codepoint: byte_idx.char_from(text),
        vertical: glyphs[0].vertical,
        width,
        advance: width,
        ascent: if glyphs[0].vertical {
            font_size / 2.0
        } else {
            font.ascent(font_size)
        },
        descent: if glyphs[0].vertical {
            -font_size / 2.0
        } else {
            font.descent(font_size)
        },
        has_relative_shift: false,
        transform: Transform::default(),
        glyphs: positioned_glyphs,
        visible: true,
        rtl: glyphs[0].rtl,
    }
}

pub(crate) trait DatabaseExt {
    /// The face `id` with the line metrics `overrides` replace.
    fn load_font(&self, id: ID, overrides: crate::fonts::MetricOverrides) -> Option<ResolvedFont>;
    fn has_char(&self, id: ID, c: char) -> bool;
}

impl DatabaseExt for Database {
    #[inline(never)]
    fn load_font(&self, id: ID, overrides: crate::fonts::MetricOverrides) -> Option<ResolvedFont> {
        self.with_face_data(id, |data, face_index| -> Option<ResolvedFont> {
            let font = ttf_parser::Face::parse(data, face_index).ok()?;

            let units_per_em = NonZeroU16::new(font.units_per_em())?;

            let ascent = font.ascender();
            let descent = font.descender();

            let x_height = font
                .x_height()
                .and_then(|x| u16::try_from(x).ok())
                .and_then(NonZeroU16::new);
            let x_height = match x_height {
                Some(height) => height,
                None => {
                    // If not set - fallback to height * 45%.
                    // 45% is what Firefox uses.
                    u16::try_from((f32::from(ascent - descent) * 0.45) as i32)
                        .ok()
                        .and_then(NonZeroU16::new)?
                }
            };

            let line_through = font.strikeout_metrics();
            let line_through_position = match line_through {
                Some(metrics) => metrics.position,
                None => x_height.get() as i16 / 2,
            };

            let (underline_position, underline_thickness) = match font.underline_metrics() {
                Some(metrics) => {
                    let thickness = u16::try_from(metrics.thickness)
                        .ok()
                        .and_then(NonZeroU16::new)
                        // `ttf_parser` guarantees that units_per_em is >= 16
                        .unwrap_or_else(|| NonZeroU16::new(units_per_em.get() / 12).unwrap());

                    (metrics.position, thickness)
                }
                None => (
                    -(units_per_em.get() as i16) / 9,
                    NonZeroU16::new(units_per_em.get() / 12).unwrap(),
                ),
            };

            // 0.2 and 0.4 are generic offsets used by some applications (Inkscape/librsvg).
            let mut subscript_offset = (units_per_em.get() as f32 / 0.2).round() as i16;
            let mut superscript_offset = (units_per_em.get() as f32 / 0.4).round() as i16;
            if let Some(metrics) = font.subscript_metrics() {
                subscript_offset = metrics.y_offset;
            }

            if let Some(metrics) = font.superscript_metrics() {
                superscript_offset = metrics.y_offset;
            }

            let metrics = crate::fonts::FaceMetrics::of(&font);
            // CSS Fonts 4 §4.11: the overrides replace the horizontal line metrics, in em.
            let units = |em: f32| (em * f32::from(units_per_em.get())).round() as i16;
            let ascent = overrides.ascent.map_or(ascent, units);
            let descent = overrides.descent.map_or(descent, |em| -units(em));
            Some(ResolvedFont {
                id,
                metrics: metrics.overridden(overrides),
                upright_metrics: metrics.upright(&font),
                space_advance: crate::fonts::advance(&font, ' '),
                zero_advance: crate::fonts::advance(&font, crate::fonts::ZERO),
                units_per_em,
                ascent,
                descent,
                x_height,
                underline_position,
                underline_thickness,
                line_through_position,
                subscript_offset,
                superscript_offset,
            })
        })?
    }

    #[inline(never)]
    fn has_char(&self, id: ID, c: char) -> bool {
        let res = self.with_face_data(id, |font_data, face_index| -> Option<bool> {
            let font = ttf_parser::Face::parse(font_data, face_index).ok()?;
            font.glyph_index(c)?;
            Some(true)
        });

        res == Some(Some(true))
    }
}

/// Whether the text of `next`, which follows that of `span`, shapes on with it: shaping is not
/// broken where formatting changes nothing the glyphs depend on (CSS Text 3 §7.3).
fn shapes_alike(span: &TextSpan, next: &TextSpan) -> bool {
    span.font == next.font
        && span.font_size == next.font_size
        && span.font_optical_sizing == next.font_optical_sizing
        && span.letter_spacing == next.letter_spacing
        && span.text_orientation == next.text_orientation
        && span.dominant_baseline == next.dominant_baseline
        && span.alignment_baseline == next.alignment_baseline
        && span.baseline_shift == next.baseline_shift
        && span.style.same_text_style(&next.style)
}

/// A span's text as it shapes with each face (SVG 2 §11.10, CSS Fonts 4 §6–§7).
#[derive(Clone, Copy)]
pub(crate) struct SpanShaping<'a> {
    pub(crate) request: &'a Font,
    pub(crate) style: &'a crate::style::Style,
    pub(crate) font_size: f32,
    pub(crate) optical_sizing: crate::svg::usvg::FontOpticalSizing,
    /// Letter spacing tracks the text too far apart for common ligatures (CSS Text 3 §8.2).
    pub(crate) tracked: bool,
}

/// Text shaping with font fallback.
pub(crate) fn shape_text(
    text: &str,
    bidi: &crate::text::bidi::ResolvedParagraph<'_>,
    orientations: OrientationSpans<'_>,
    active: std::ops::Range<usize>,
    font: Arc<ResolvedFont>,
    span: SpanShaping<'_>,
    resolver: &dyn FontProvider,
    fontdb: &mut Arc<Database>,
) -> Vec<Glyph> {
    let shape = |fontdb: &Database, font: &Arc<ResolvedFont>, source: std::ops::Range<usize>| {
        shape_text_with_font(
            text,
            bidi,
            source,
            orientations,
            font.clone(),
            span,
            &resolver.face_settings(span.request, font.id, span.style),
            &resolver.variations(span.request, font.id),
            fontdb,
        )
    };
    let mut glyphs = shape(fontdb, &font, active.clone()).unwrap_or_default();
    struct FallbackRun {
        start: usize,
        end: usize,
        source: std::ops::Range<usize>,
        font: Arc<ResolvedFont>,
    }
    let mut previous: Option<FallbackRun> = None;
    let mut index = 0;
    while index < glyphs.len() {
        let source = glyphs[index].byte_idx;
        let end = glyphs[index..]
            .iter()
            .position(|glyph| glyph.byte_idx != source)
            .map_or(glyphs.len(), |count| index + count);
        let Some(missing) = glyphs[index..end]
            .iter()
            .find(|glyph| glyph.wants_fallback() && active.contains(&glyph.byte_idx.value()))
        else {
            previous = None;
            index = end;
            continue;
        };
        let character = missing.byte_idx.char_from(text);
        let source_end = source.value()
            + glyphs[index..end]
                .iter()
                .map(|glyph| glyph.cluster_len)
                .max()
                .unwrap_or(0);
        let source_end = source_end.max(source.value() + character.len_utf8());
        // A face showing the cluster without the sequences it lacks replaces a missing glyph
        // where none has them; the unsupported selectors are ignored (CSS Fonts 4 §9.3).
        let shown = !glyphs[index..end].iter().any(Glyph::is_missing);
        let mut unpresented = None;
        let mut used_fonts = vec![font.id];
        let mut replacement = None;
        loop {
            let Some(id) = resolver.select_fallback(span.request, character, &used_fonts, fontdb)
            else {
                break;
            };
            if used_fonts.contains(&id) {
                break;
            }
            used_fonts.push(id);
            let Some(fallback) = fontdb.load_font(id, resolver.metric_overrides(id)) else {
                continue;
            };
            let fallback = Arc::new(fallback);
            let mut group_end = end;
            let mut group_source = source.value()..source_end;
            while group_end < glyphs.len() {
                let next_source = glyphs[group_end].byte_idx;
                let next_end = glyphs[group_end..]
                    .iter()
                    .position(|glyph| glyph.byte_idx != next_source)
                    .map_or(glyphs.len(), |count| group_end + count);
                if !glyphs[group_end..next_end]
                    .iter()
                    .any(Glyph::wants_fallback)
                    || !active.contains(&next_source.value())
                {
                    break;
                }
                let character = next_source.char_from(text);
                let next_source_end = next_source.value()
                    + glyphs[group_end..next_end]
                        .iter()
                        .map(|glyph| glyph.cluster_len)
                        .max()
                        .unwrap_or(0)
                        .max(character.len_utf8());
                if group_source.end != next_source.value() && next_source_end != group_source.start
                {
                    break;
                }
                if resolver.select_fallback(span.request, character, &[font.id], fontdb) != Some(id)
                {
                    break;
                }
                group_source.start = group_source.start.min(next_source.value());
                group_source.end = group_source.end.max(next_source_end);
                group_end = next_end;
            }
            let candidate = shape(fontdb, &fallback, group_source.clone());
            if let Some(candidate) = &candidate
                && !candidate.iter().any(Glyph::wants_fallback)
            {
                replacement = Some((group_end, group_source, fallback.clone(), candidate.clone()));
                break;
            }
            let candidate = if group_end == end {
                candidate
            } else {
                shape(fontdb, &fallback, source.value()..source_end)
            };
            if let Some(candidate) = candidate {
                if !candidate.iter().any(Glyph::wants_fallback) {
                    replacement = Some((end, source.value()..source_end, fallback, candidate));
                    break;
                }
                if !shown && unpresented.is_none() && !candidate.iter().any(Glyph::is_missing) {
                    unpresented = Some((end, source.value()..source_end, fallback, candidate));
                }
            }
        }
        let complete = replacement.is_some();
        if let Some((end, mut source, fallback, mut replacement)) = replacement.or(unpresented) {
            let mut start = index;
            if let Some(run) = &previous
                && run.end == index
                && run.font.id == fallback.id
                && (run.source.end == source.start || source.end == run.source.start)
            {
                let combined = run.source.start.min(source.start)..run.source.end.max(source.end);
                let merged = shape(fontdb, &fallback, combined.clone());
                if let Some(merged) = merged.filter(|glyphs| {
                    !glyphs
                        .iter()
                        .any(|glyph| glyph.is_missing() || (complete && glyph.wants_fallback()))
                }) {
                    start = run.start;
                    source = combined;
                    replacement = merged;
                }
            }
            let count = replacement.len();
            glyphs.splice(start..end, replacement);
            index = start + count;
            previous = Some(FallbackRun {
                start,
                end: index,
                source,
                font: fallback,
            });
        } else {
            previous = None;
            index = end;
        }
    }

    // Warn about missing glyphs.
    for glyph in &glyphs {
        if glyph.is_missing() {
            let c = glyph.byte_idx.char_from(text);
            // TODO: print a full grapheme
            log::warn!(
                "No fonts with a {}/U+{:X} character were found.",
                c,
                c as u32
            );
        }
    }

    glyphs
}

/// Converts a text into a list of glyph IDs.
///
/// This function will do the BIDI reordering and text shaping.
fn shape_text_with_font(
    text: &str,
    bidi: &crate::text::bidi::ResolvedParagraph<'_>,
    source: std::ops::Range<usize>,
    orientations: OrientationSpans<'_>,
    font: Arc<ResolvedFont>,
    span: SpanShaping<'_>,
    settings: &crate::svg::usvg::FaceSettings,
    variations: &[crate::svg::usvg::FontVariation],
    fontdb: &Database,
) -> Option<Vec<Glyph>> {
    use crate::text::{features, shaping};
    let style = span.style;
    fontdb.with_face_data(font.id, |font_data, face_index| -> Option<Vec<Glyph>> {
        let mut rb_font = rustybuzz::Face::from_slice(font_data, face_index)?;

        // CSS Fonts 5 §2.5: the used size gives the face the aspect value `font-size-adjust`
        // specifies, which `size-adjust` does not change (§4.4).
        let size_scale = style
            .font_adjustments
            .size_adjust
            .map_or(settings.size_adjust, |adjust| {
                adjust.scale(&crate::fonts::FaceMetrics::of(&rb_font))
            });
        let used_size = span.font_size * size_scale;
        let final_variations =
            super::font_variations(&rb_font, variations, used_size, span.optical_sizing);

        // Apply font variations for variable fonts
        if !final_variations.is_empty() {
            rb_font.set_variations(&final_variations);
        }
        let language = crate::text::shaping_language(&rb_font, style);
        let face_features = features::FaceFeatures {
            descriptor: &settings.descriptor,
            alternates: &settings.alternates,
            ..features::FaceFeatures::default()
        };
        // §6.5's synthesis sizes are absolute; user units are CSS pixels.
        let used_points = used_size * crate::style::POINTS_PER_PIXEL;
        let units_per_em = f32::from(font.units_per_em.get());

        let mut glyphs = Vec::new();
        let rtl = |offset: usize| bidi.level(offset).is_rtl();
        let vertical_forms = crate::fonts::has_feature(&rb_font, *b"vert")
            || crate::fonts::has_feature(&rb_font, *b"vrt2");
        // CSS Fonts 4 §6.6: the face's own capitals, or capitals scaled from uppercase forms,
        // as for the document's text.
        let plan = features::caps_plan(
            style.font_variant_caps,
            |tag| crate::fonts::has_feature(&rb_font, tag),
            style.font_synthesis.small_caps,
        );
        for (run, _) in bidi.visual_runs(source.clone()) {
            if run.is_empty() {
                continue;
            }
            let mut pieces: Vec<(std::ops::Range<usize>, bool)> = Vec::new();
            if let Some(orientations) = orientations {
                for (range, orientation) in orientations {
                    let range = range.start.max(run.start)..range.end.min(run.end);
                    if range.is_empty() {
                        continue;
                    }
                    for (range, vertical) in orientation.ranges(text, range, vertical_forms) {
                        if let Some((previous, prior_vertical)) = pieces.last_mut()
                            && previous.end == range.start
                            && *prior_vertical == vertical
                        {
                            previous.end = range.end;
                        } else {
                            pieces.push((range, vertical));
                        }
                    }
                }
            } else {
                pieces.push((run.clone(), false));
            }
            if rtl(run.start) {
                pieces.reverse();
            }
            for (run, vertical) in pieces {
                // CSS Fonts 4 §6.5: the face's position variants, else reduced glyphs shifted
                // from the baseline.
                let (position_feature, position_scale, rise) = match shaping::position_form(
                    &rb_font,
                    face_features,
                    used_points,
                    &text[run.clone()],
                    rtl(run.start),
                    style,
                    plan,
                ) {
                    shaping::Position::Normal => (None, 1.0, 0.0),
                    shaping::Position::Native(feature) => (Some(feature), 1.0, 0.0),
                    shaping::Position::Synthetic { scale, offset } => {
                        (None, scale, -offset / used_points * used_size)
                    }
                };
                let shaper_features = shaping::shaper_features(&features::run_features(
                    style,
                    features::FaceFeatures {
                        caps: plan.features,
                        position: position_feature,
                        ..face_features
                    },
                    span.tracked,
                ));
                let mut caps_pieces = if plan.synthetic() {
                    plan.pieces(text, run)
                } else {
                    vec![(run, false)]
                };
                if rtl(caps_pieces[0].0.start) {
                    caps_pieces.reverse();
                }
                for (run, small) in caps_pieces {
                    let sub_text = &text[run.clone()];
                    if sub_text.is_empty() {
                        continue;
                    }

                    let ltr = !rtl(run.start);
                    let hb_direction = if vertical {
                        rustybuzz::Direction::TopToBottom
                    } else if ltr {
                        rustybuzz::Direction::LeftToRight
                    } else {
                        rustybuzz::Direction::RightToLeft
                    };

                    let mut buffer = rustybuzz::UnicodeBuffer::new();
                    // CSS Fonts 4 §9.3: `font-variant-emoji` appends a presentation selector
                    // to each emoji its text does not select one for.
                    let emoji = style.font_features.emoji;
                    let add = |buffer: &mut rustybuzz::UnicodeBuffer,
                               characters: &mut dyn Iterator<Item = (usize, char)>| {
                        let mut characters = characters.peekable();
                        while let Some((cluster, ch)) = characters.next() {
                            buffer.add(ch, cluster as u32);
                            let next = characters.peek().map(|(_, next)| *next);
                            if let Some(selector) = shaping::implied_presentation(emoji, ch, next) {
                                buffer.add(selector, cluster as u32);
                            }
                        }
                    };
                    let presenting = crate::text::implies_presentation(style, sub_text);
                    if small {
                        for (start, upper) in
                            features::uppercase_clusters(text, run.clone(), &style.language)
                        {
                            add(&mut buffer, &mut upper.chars().map(|ch| (start, ch)));
                        }
                    } else if presenting {
                        add(&mut buffer, &mut sub_text.char_indices());
                    } else {
                        buffer.push_str(sub_text);
                    }
                    let sequences = presenting || sub_text.chars().any(shaping::variation_selector);
                    // A selector no sequence of the face takes leaves no glyph, as in the
                    // document's text.
                    buffer.set_flags(rustybuzz::BufferFlags::REMOVE_DEFAULT_IGNORABLES);
                    buffer.set_pre_context(&text[..run.start]);
                    buffer.set_post_context(&text[run.end..]);
                    buffer.set_direction(hb_direction);
                    if let Some(language) = &language {
                        buffer.set_language(language.clone());
                    }
                    buffer.guess_segment_properties();

                    work::count(Work::ShapedByte, sub_text.len());
                    let output = rustybuzz::shape(&rb_font, &shaper_features, buffer);

                    let positions = output.glyph_positions();
                    let infos = output.glyph_infos();
                    let boundaries = crate::fonts::cluster_boundaries(
                        infos.iter().map(|glyph| glyph.cluster as usize),
                        sub_text.len(),
                    );
                    let scale = size_scale
                        * position_scale
                        * if small {
                            features::SMALL_CAPS_SCALE
                        } else {
                            1.0
                        };
                    // Synthesized positions shift horizontal glyphs only, as in the document.
                    let rise = if vertical {
                        0
                    } else {
                        (rise / (span.font_size * scale) * units_per_em).round() as i32
                    };

                    for i in 0..output.len() {
                        let pos = positions[i];
                        let info = infos[i];
                        let idx = run.start + info.cluster as usize;

                        let start = info.cluster as usize;

                        let end = boundaries[boundaries.binary_search(&start).unwrap() + 1];

                        glyphs.push(Glyph {
                            byte_idx: ByteIndex::new(idx),
                            cluster_len: end - start,
                            text: sub_text[start..end].to_string(),
                            id: GlyphId(info.glyph_id as u16),
                            dx: pos.x_offset,
                            dy: pos.y_offset + rise,
                            width: if vertical {
                                -pos.y_advance
                            } else {
                                pos.x_advance
                            },
                            vertical,
                            scale,
                            font: font.clone(),
                            paint: settings.paint.clone(),
                            compression: 1.0,
                            text_offset: 0,
                            rtl: !ltr,
                            unpresented: sequences
                                && !shaping::supports_variations(
                                    &rb_font,
                                    &sub_text[start..end],
                                    Some(emoji),
                                ),
                        });
                    }
                }
            }
        }

        Some(glyphs)
    })?
}

/// An iterator over glyph clusters.
///
/// Input:  0 2 2 2 3 4 4 5 5
/// Result: 0 1     4 5   7
pub(crate) struct GlyphClusters<'a> {
    data: &'a [Glyph],
    idx: usize,
}

impl<'a> GlyphClusters<'a> {
    pub(crate) fn new(data: &'a [Glyph]) -> Self {
        GlyphClusters { data, idx: 0 }
    }
}

impl Iterator for GlyphClusters<'_> {
    type Item = (std::ops::Range<usize>, ByteIndex);

    fn next(&mut self) -> Option<Self::Item> {
        if self.idx == self.data.len() {
            return None;
        }

        let start = self.idx;
        let cluster = self.data[self.idx].byte_idx;
        for g in &self.data[self.idx..] {
            if g.byte_idx != cluster {
                break;
            }

            self.idx += 1;
        }

        Some((start..self.idx, cluster))
    }
}

/// Checks that selected script supports letter spacing.
///
/// [In the CSS spec](https://www.w3.org/TR/css-text-3/#cursive-tracking).
///
/// The list itself is from: https://github.com/harfbuzz/harfbuzz/issues/64
pub(crate) fn script_supports_letter_spacing(script: unicode_script::Script) -> bool {
    use unicode_script::Script;

    !matches!(
        script,
        Script::Arabic
            | Script::Syriac
            | Script::Nko
            | Script::Manichaean
            | Script::Psalter_Pahlavi
            | Script::Mandaic
            | Script::Mongolian
            | Script::Phags_Pa
            | Script::Devanagari
            | Script::Bengali
            | Script::Gurmukhi
            | Script::Modi
            | Script::Sharada
            | Script::Syloti_Nagri
            | Script::Tirhuta
            | Script::Ogham
    )
}

/// A glyph.
///
/// Basically, a glyph ID and it's metrics.
#[derive(Clone)]
pub(crate) struct Glyph {
    /// The glyph ID in the font.
    pub(crate) id: GlyphId,

    /// Position in bytes in the original string.
    ///
    /// We use it to match a glyph with a character in the text chunk and therefore with the style.
    pub(crate) byte_idx: ByteIndex,

    // The length of the cluster in bytes.
    pub(crate) cluster_len: usize,

    /// The text from the original string that corresponds to that glyph.
    pub(crate) text: String,

    /// The glyph offset in font units.
    pub(crate) dx: i32,

    /// The glyph offset in font units.
    pub(crate) dy: i32,

    /// The glyph width / X-advance in font units.
    pub(crate) width: i32,
    pub(crate) vertical: bool,
    /// The glyph's size relative to its span's, below one for synthesized capitals.
    pub(crate) scale: f32,
    /// The horizontal scale of a combination's glyph (CSS Writing Modes 4 §9.1.3).
    pub(crate) compression: f32,
    /// Where `text` starts in the glyph's cluster: within a combination, its own characters.
    pub(crate) text_offset: usize,
    /// The glyph's resolved direction is right-to-left.
    pub(crate) rtl: bool,

    /// Reference to the source font.
    ///
    /// Each glyph can have it's own source font.
    pub(crate) font: Arc<ResolvedFont>,
    pub(crate) paint: Arc<crate::svg::usvg::GlyphPaint>,
    /// The face lacks a variation sequence of the glyph's cluster, including a presentation
    /// `font-variant-emoji` implies, which another face may have (CSS Fonts 4 §9.3).
    pub(crate) unpresented: bool,
}

impl Glyph {
    fn is_missing(&self) -> bool {
        self.id.0 == 0
    }
    /// Whether another face may show the glyph's cluster better.
    fn wants_fallback(&self) -> bool {
        self.is_missing() || self.unpresented
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ResolvedFont {
    pub(crate) id: ID,
    /// The face's metrics in em, and those of its upright vertical text.
    pub(crate) metrics: crate::fonts::FaceMetrics,
    upright_metrics: crate::fonts::FaceMetrics,
    /// The advances of a space and of "0" in em, where the face has them.
    space_advance: Option<f32>,
    zero_advance: Option<f32>,

    units_per_em: NonZeroU16,

    // All values below are in font units.
    ascent: i16,
    descent: i16,
    x_height: NonZeroU16,

    underline_position: i16,
    underline_thickness: NonZeroU16,

    // line-through thickness should be the the same as underline thickness
    // according to the TrueType spec:
    // https://docs.microsoft.com/en-us/typography/opentype/spec/os2#ystrikeoutsize
    line_through_position: i16,

    subscript_offset: i16,
    superscript_offset: i16,
}

pub(crate) fn chunk_span_at(chunk: &TextChunk, byte_offset: ByteIndex) -> Option<&TextSpan> {
    chunk_span_index(chunk, byte_offset).map(|index| &chunk.spans[index])
}

/// The index of the span of `chunk`, whose spans follow each other in its text, that contains
/// `byte_offset`.
fn chunk_span_index(chunk: &TextChunk, byte_offset: ByteIndex) -> Option<usize> {
    let index = chunk
        .spans
        .partition_point(|span| span.end <= byte_offset.value());
    chunk
        .spans
        .get(index)
        .filter(|span| span_contains(span, byte_offset))
        .map(|_| index)
}

pub(crate) fn span_contains(span: &TextSpan, byte_offset: ByteIndex) -> bool {
    byte_offset.value() >= span.start && byte_offset.value() < span.end
}

/// Checks that the selected character is a word separator.
///
/// According to: https://www.w3.org/TR/css-text-3/#word-separator
pub(crate) fn is_word_separator_characters(c: char) -> bool {
    matches!(
        c as u32,
        0x0020 | 0x00A0 | 0x1361 | 0x010100 | 0x010101 | 0x01039F | 0x01091F
    )
}

impl ResolvedFont {
    #[inline]
    pub(crate) fn scale(&self, font_size: f32) -> f32 {
        font_size / self.units_per_em.get() as f32
    }

    #[inline]
    pub(crate) fn ascent(&self, font_size: f32) -> f32 {
        self.ascent as f32 * self.scale(font_size)
    }

    #[inline]
    pub(crate) fn descent(&self, font_size: f32) -> f32 {
        self.descent as f32 * self.scale(font_size)
    }

    #[inline]
    pub(crate) fn height(&self, font_size: f32) -> f32 {
        self.ascent(font_size) - self.descent(font_size)
    }

    #[inline]
    pub(crate) fn x_height(&self, font_size: f32) -> f32 {
        self.x_height.get() as f32 * self.scale(font_size)
    }

    #[inline]
    pub(crate) fn underline_position(&self, font_size: f32) -> f32 {
        self.underline_position as f32 * self.scale(font_size)
    }

    #[inline]
    fn underline_thickness(&self, font_size: f32) -> f32 {
        self.underline_thickness.get() as f32 * self.scale(font_size)
    }

    #[inline]
    pub(crate) fn line_through_position(&self, font_size: f32) -> f32 {
        self.line_through_position as f32 * self.scale(font_size)
    }

    #[inline]
    fn subscript_offset(&self, font_size: f32) -> f32 {
        self.subscript_offset as f32 * self.scale(font_size)
    }

    #[inline]
    fn superscript_offset(&self, font_size: f32) -> f32 {
        self.superscript_offset as f32 * self.scale(font_size)
    }

    pub(crate) fn dominant_baseline_shift(
        &self,
        baseline: DominantBaseline,
        font_size: f32,
    ) -> f32 {
        let alignment = match baseline {
            DominantBaseline::Auto => AlignmentBaseline::Auto,
            DominantBaseline::UseScript => AlignmentBaseline::Auto, // unsupported
            DominantBaseline::NoChange => AlignmentBaseline::Auto,  // already resolved
            DominantBaseline::ResetSize => AlignmentBaseline::Auto, // unsupported
            DominantBaseline::Ideographic => AlignmentBaseline::Ideographic,
            DominantBaseline::Alphabetic => AlignmentBaseline::Alphabetic,
            DominantBaseline::Hanging => AlignmentBaseline::Hanging,
            DominantBaseline::Mathematical => AlignmentBaseline::Mathematical,
            DominantBaseline::Central => AlignmentBaseline::Central,
            DominantBaseline::Middle => AlignmentBaseline::Middle,
            DominantBaseline::TextAfterEdge => AlignmentBaseline::TextAfterEdge,
            DominantBaseline::TextBeforeEdge => AlignmentBaseline::TextBeforeEdge,
        };

        self.alignment_baseline_shift(alignment, font_size)
    }

    // The `alignment-baseline` property is a mess.
    //
    // The SVG 1.1 spec (https://www.w3.org/TR/SVG11/text.html#BaselineAlignmentProperties)
    // goes on and on about what this property suppose to do, but doesn't actually explain
    // how it should be implemented. It's just a very verbose overview.
    //
    // As of Nov 2022, only Chrome and Safari support `alignment-baseline`. Firefox isn't.
    // Same goes for basically every SVG library in existence.
    // Meaning we have no idea how exactly it should be implemented.
    //
    // And even Chrome and Safari cannot agree on how to handle `baseline`, `after-edge`,
    // `text-after-edge` and `ideographic` variants. Producing vastly different output.
    //
    // As per spec, a proper implementation should get baseline values from the font itself,
    // using `BASE` and `bsln` TrueType tables. If those tables are not present,
    // we have to synthesize them (https://drafts.csswg.org/css-inline/#baseline-synthesis-fonts).
    // And in the worst case scenario simply fallback to hardcoded values.
    //
    // Also, most fonts do not provide `BASE` and `bsln` tables to begin with.
    //
    // Again, as of Nov 2022, Chrome does only the latter:
    // https://github.com/chromium/chromium/blob/main/third_party/blink/renderer/platform/fonts/font_metrics.cc#L153
    //
    // Since baseline TrueType tables parsing and baseline synthesis are pretty hard,
    // we do what Chrome does - use hardcoded values. And it seems like Safari does the same.
    //
    //
    // But that's not all! SVG 2 and CSS Inline Layout 3 did a baseline handling overhaul,
    // and it's far more complex now. Not sure if anyone actually supports it.
    pub(crate) fn alignment_baseline_shift(
        &self,
        alignment: AlignmentBaseline,
        font_size: f32,
    ) -> f32 {
        match alignment {
            AlignmentBaseline::Auto => 0.0,
            AlignmentBaseline::Baseline => 0.0,
            AlignmentBaseline::BeforeEdge | AlignmentBaseline::TextBeforeEdge => {
                self.ascent(font_size)
            }
            AlignmentBaseline::Middle => self.x_height(font_size) * 0.5,
            AlignmentBaseline::Central => self.ascent(font_size) - self.height(font_size) * 0.5,
            AlignmentBaseline::AfterEdge | AlignmentBaseline::TextAfterEdge => {
                self.descent(font_size)
            }
            AlignmentBaseline::Ideographic => self.descent(font_size),
            AlignmentBaseline::Alphabetic => 0.0,
            AlignmentBaseline::Hanging => self.ascent(font_size) * 0.8,
            AlignmentBaseline::Mathematical => self.ascent(font_size) * 0.5,
        }
    }
}

pub(crate) type FontsCache = HashMap<Font, Arc<ResolvedFont>>;

/// The code point positions of a text's byte positions: the starts of its characters, which
/// a search finds a position's among.
struct CodePoints(Vec<usize>);

impl CodePoints {
    fn of(text: &str) -> Self {
        let starts: Vec<usize> = text.char_indices().map(|(start, _)| start).collect();
        work::count(Work::CodePoint, starts.len());
        Self(starts)
    }

    /// The code point position of `index`, a character boundary.
    fn at(&self, index: ByteIndex) -> usize {
        work::count(Work::CodePoint, 1);
        self.0.partition_point(|&start| start < index.value())
    }
}

/// A read-only text index in bytes.
///
/// Guarantee to be on a char boundary and in text bounds.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) struct ByteIndex(usize);

impl ByteIndex {
    fn new(i: usize) -> Self {
        ByteIndex(i)
    }

    pub(crate) fn value(&self) -> usize {
        self.0
    }

    /// Converts byte position into a character.
    pub(crate) fn char_from(&self, text: &str) -> char {
        text[self.0..].chars().next().unwrap()
    }
}
