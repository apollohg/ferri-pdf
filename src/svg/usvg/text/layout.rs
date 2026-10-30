// Copyright 2022 the Resvg Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use std::collections::{HashMap, HashSet};
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
}

impl PositionedGlyph {
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
}

impl GlyphCluster {
    pub(crate) fn height(&self) -> f32 {
        self.ascent - self.descent
    }

    pub(crate) fn transform(&self) -> Transform {
        self.transform
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

    for chunk in &text_node.chunks {
        for span in &chunk.spans {
            let edge = span.line_edge.as_ref().map(|edge| &edge.strut.font);
            for font in std::iter::once(&span.font).chain(edge) {
                if !fonts_cache.contains_key(font) {
                    if let Some(loaded) = resolver
                        .select_font(font, fontdb)
                        .and_then(|id| fontdb.load_font(id))
                    {
                        fonts_cache.insert(font.clone(), Arc::new(loaded));
                    }
                }
            }
        }
    }

    let strut = resolver
        .select_font(&text_node.strut.font, fontdb)
        .and_then(|id| fontdb.load_font(id));
    let wrapped = match text_node.wrapping {
        Some(wrapping) => Some(super::wrap::wrap(
            text_node,
            wrapping,
            &fonts_cache,
            strut.as_ref(),
            resolver,
            fontdb,
        )?),
        None => None,
    };
    let text_node = wrapped.as_ref().unwrap_or(text_node);
    let lines = LineBoxes::new(text_node, strut.as_ref(), &fonts_cache);

    let mut spans = vec![];
    let mut char_offset = 0;
    let mut last_x = 0.0;
    let mut last_y = 0.0;
    let mut line_origin = None;
    let mut line = 0;
    let mut block_offset = 0.0;
    let mut bbox = BBox::default();
    for chunk in &text_node.chunks {
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

        if let (Some(strut), TextFlow::Linear) = (&strut, &chunk.text_flow) {
            let (start_x, start_y) = line_origin.unwrap_or((x, y));
            let inline_start = match text_node.writing_mode {
                WritingMode::LeftToRight => x - start_x,
                WritingMode::TopToBottom => y - start_y,
            };
            apply_tab_stops(
                chunk,
                &text_node.strut,
                strut,
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

        for span in &chunk.spans {
            let font = match fonts_cache.get(&span.font) {
                Some(v) => v,
                None => continue,
            };

            let decoration_spans = collect_decoration_spans(span, &clusters);

            let mut span_ts = text_ts;
            span_ts = span_ts.pre_translate(x, y);
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
                let path = convert_decoration(
                    line,
                    span,
                    font,
                    decoration.as_ref()?,
                    &decoration_spans,
                    text_node.writing_mode,
                    span_ts,
                )?;
                bbox = bbox.expand(path.data.bounds());
                Some(path)
            });

            let mut emphasis = emphasis_marks(
                span,
                &clusters,
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

            if let Some((span_fragments, span_bbox)) = convert_span(span, &clusters, span_ts) {
                bbox = bbox.expand(span_bbox);

                let positioned_glyphs: Vec<PositionedGlyph> = span_fragments
                    .into_iter()
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
fn emphasis_marks(
    span: &TextSpan,
    clusters: &[GlyphCluster],
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
        .iter()
        .filter(|cluster| {
            cluster.visible
                && span_contains(span, cluster.byte_idx)
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
        hanging: 0,
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
                if let Some(path) = fontdb
                    .outline(glyph.font, glyph.id)
                    .and_then(|path| path.transform(positioned.outline_transform()))
                {
                    builder.push_path(&path);
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
    Some(clusters)
}

fn convert_span(
    span: &TextSpan,
    clusters: &[GlyphCluster],
    text_ts: Transform,
) -> Option<(Vec<GlyphCluster>, NonZeroRect)> {
    let mut span_clusters = vec![];
    let mut bboxes_builder = tiny_skia_path::PathBuilder::new();

    for cluster in clusters {
        if !cluster.visible {
            continue;
        }

        if span_contains(span, cluster.byte_idx) {
            span_clusters.push(cluster.clone());
        }

        let mut advance = cluster.advance;
        if advance <= 0.0 {
            advance = 1.0;
        }

        // We have to calculate text bbox using font metrics and not glyph shape.
        if let Some(r) = NonZeroRect::from_xywh(0.0, -cluster.ascent, advance, cluster.height()) {
            if let Some(r) = r.transform(cluster.transform()) {
                bboxes_builder.push_rect(r.to_rect());
            }
        }
    }

    let mut bboxes = bboxes_builder.finish()?;
    bboxes = bboxes.transform(text_ts)?;
    let bbox = bboxes.compute_tight_bounds()?.to_non_zero_rect()?;

    Some((span_clusters, bbox))
}

fn collect_decoration_spans(span: &TextSpan, clusters: &[GlyphCluster]) -> Vec<DecorationSpan> {
    let mut spans = Vec::new();

    let mut started = false;
    let mut width = 0.0;
    let mut transform = Transform::default();

    for cluster in clusters {
        if span_contains(span, cluster.byte_idx) {
            if started && cluster.has_relative_shift {
                started = false;
                spans.push(DecorationSpan { width, transform });
            }

            if !started {
                width = cluster.advance;
                started = true;
                transform = cluster.transform;
            } else {
                width += cluster.advance;
            }
        } else if started {
            spans.push(DecorationSpan { width, transform });
            started = false;
        }
    }

    if started {
        spans.push(DecorationSpan { width, transform });
    }

    spans
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
    decoration_spans: &[DecorationSpan],
    writing_mode: WritingMode,
    transform: Transform,
) -> Option<Path> {
    use crate::style::POINTS_PER_PIXEL;
    debug_assert!(!decoration_spans.is_empty());

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
    let [top, thickness] = metrics
        .place(line, &decoration.declared, &decoration.style)
        .map(|length| length / POINTS_PER_PIXEL);
    let (style, color) = decoration.declared.paint();

    let mut builder = tiny_skia_path::PathBuilder::new();
    for dec_span in decoration_spans {
        let paint = crate::paint::DecorationPaint {
            line,
            style,
            color: crate::paint::Color::BLACK,
            rect: crate::paint::Rect {
                x: 0.0,
                y: top,
                width: dec_span.width,
                height: thickness,
            },
            owner: None,
            positioned: None,
        };
        for fill in paint.fills() {
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
#[derive(Clone, Copy)]
pub(crate) struct DecorationSpan {
    pub(crate) width: f32,
    pub(crate) transform: Transform,
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
    // Spaces hanging at a wrapped line's end do not count for its alignment.
    let hanging = chunk.text.len() - chunk.hanging;
    let aligned = clusters
        .iter()
        .filter(|cluster| cluster.byte_idx.value() < hanging)
        .fold(0.0, |width, cluster| width + cluster.advance);
    let mut x = process_anchor(chunk.anchor, aligned);
    let mut y = 0.0;

    for cluster in clusters {
        let cp = offset + cluster.byte_idx.code_point_at(&chunk.text);
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

    let normals = collect_normals(text, chunk, clusters, &path.path, char_offset, start_offset);
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

        let cp = char_offset + cluster.byte_idx.code_point_at(&chunk.text);
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
    chunk: &TextChunk,
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
            let cp = char_offset + cluster.byte_idx.code_point_at(&chunk.text);
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

/// Converts a text chunk into a list of outlined clusters.
///
/// This function will do the BIDI reordering, text shaping and glyphs outlining,
/// but not the text layouting. So all clusters are in the 0x0 position.
fn directional_text(chunk: &TextChunk, writing_mode: WritingMode) -> std::borrow::Cow<'_, str> {
    use crate::style::logical::TextOrientation;
    if writing_mode != WritingMode::TopToBottom
        || !chunk
            .spans
            .iter()
            .any(|span| span.text_orientation == TextOrientation::Upright)
    {
        return std::borrow::Cow::Borrowed(&chunk.text);
    }
    // Strong-LTR representatives preserve source UTF-8 offsets for the shared bidi analysis.
    const LTR_BY_WIDTH: [char; 4] = ['A', '\u{0100}', '\u{4e00}', '\u{20000}'];
    let mut text = String::with_capacity(chunk.text.len());
    let mut from = 0;
    for span in &chunk.spans {
        text.push_str(&chunk.text[from..span.start]);
        let source = &chunk.text[span.start..span.end];
        if span.text_orientation == TextOrientation::Upright {
            text.extend(source.chars().map(|ch| LTR_BY_WIDTH[ch.len_utf8() - 1]));
        } else {
            text.push_str(source);
        }
        from = span.end;
    }
    text.push_str(&chunk.text[from..]);
    std::borrow::Cow::Owned(text)
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
        None => shape_chunk(chunk, writing_mode, fonts_cache, resolver, fontdb),
        Some(cased) => {
            let cased = cased?;
            let mut glyphs = shape_chunk(&cased.chunk, writing_mode, fonts_cache, resolver, fontdb);
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
            chunk: TextChunk {
                spans,
                text,
                ..chunk.clone()
            },
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

/// Shapes a chunk's spans with their fonts into glyphs in visual order.
fn shape_chunk(
    chunk: &TextChunk,
    writing_mode: WritingMode,
    fonts_cache: &FontsCache,
    resolver: &dyn FontProvider,
    fontdb: &mut Arc<Database>,
) -> Vec<Glyph> {
    // The way this function works is a bit tricky.
    //
    // The first problem is BIDI reordering.
    // We cannot shape text span-by-span, because glyph clusters are not guarantee to be continuous.
    //
    // For example:
    // <text>Hel<tspan fill="url(#lg1)">lo של</tspan>ום.</text>
    //
    // Would be shaped as:
    // H e l l o   ש ל  ו  ם .   (characters)
    // 0 1 2 3 4 5 12 10 8 6 14  (cluster indices in UTF-8)
    //       ---         ---     (green span)
    //
    // As you can see, our continuous `lo של` span was split into two separated one.
    // So our 3 spans: black - green - black, become 5 spans: black - green - black - green - black.
    // If we shape `Hel`, then `lo של` an then `ום` separately - we would get an incorrect output.
    // To properly handle this we simply shape the whole chunk.
    //
    // But this introduces another issue - what to do when we have multiple fonts?
    // The easy solution would be to simply shape text with each font,
    // where the first font output is used as a base one and all others overwrite it.
    // This way in case of:
    // <text font-family="Arial">Hello <tspan font-family="Helvetica">world</tspan></text>
    // we would replace Arial glyphs for `world` with Helvetica one. Pretty simple.
    //
    // Well, it would work most of the time, but not always.
    // This is because different fonts can produce different amount of glyphs for the same text.
    // The most common example are ligatures. Some fonts can shape `fi` as two glyphs `f` and `i`,
    // but some can use `ﬁ` (U+FB01) instead.
    // Meaning that during merging we have to overwrite not individual glyphs, but clusters.

    // Glyph splitting assigns distinct glyphs to the same index in the original text, we need to
    // store previously used indices to make sure we do not re-use the same index while overwriting
    // span glyphs.
    let mut positions = HashSet::new();

    let mut glyphs = Vec::new();
    let directional = directional_text(chunk, writing_mode);
    let orientation_spans = (writing_mode == WritingMode::TopToBottom).then(|| {
        chunk
            .spans
            .iter()
            .map(|span| (span.start..span.end, span.text_orientation))
            .collect::<Vec<_>>()
    });
    let orientations = orientation_spans.as_deref();
    for span in &chunk.spans {
        let font = match fonts_cache.get(&span.font) {
            Some(v) => v.clone(),
            None => continue,
        };

        let tmp_glyphs = shape_text(
            &chunk.text,
            &directional,
            chunk.rtl,
            orientations,
            span.start..span.end,
            font,
            SpanShaping {
                request: &span.font,
                style: &span.style,
                font_size: span.font_size.get(),
                optical_sizing: span.font_optical_sizing,
                tracked: crate::text::shaping::tracked(span.letter_spacing, span.font_size.get()),
            },
            resolver,
            fontdb,
        );

        // Do nothing with the first run.
        if glyphs.is_empty() {
            glyphs = tmp_glyphs;
            continue;
        }

        positions.clear();

        // Overwrite span's glyphs.
        let mut iter = tmp_glyphs.into_iter();
        while let Some(new_glyph) = iter.next() {
            if !span_contains(span, new_glyph.byte_idx) {
                continue;
            }

            let Some(idx) = glyphs
                .iter()
                .position(|g| g.byte_idx == new_glyph.byte_idx)
                .filter(|pos| !positions.contains(pos))
            else {
                continue;
            };

            positions.insert(idx);

            let prev_cluster_len = glyphs[idx].cluster_len;
            if prev_cluster_len < new_glyph.cluster_len {
                // If the new font represents the same cluster with fewer glyphs
                // then remove remaining glyphs.
                for _ in 1..new_glyph.cluster_len {
                    glyphs.remove(idx + 1);
                }
            } else if prev_cluster_len > new_glyph.cluster_len {
                // If the new font represents the same cluster with more glyphs
                // then insert them after the current one.
                for j in 1..prev_cluster_len {
                    if let Some(g) = iter.next() {
                        glyphs.insert(idx + j, g);
                    }
                }
            }

            glyphs[idx] = new_glyph;
        }
    }

    glyphs
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
    fn new(text: &Text, strut: Option<&ResolvedFont>, fonts: &FontsCache) -> Self {
        let vertical = text.writing_mode == WritingMode::TopToBottom;
        let extents_of = |font: Option<&ResolvedFont>, strut: &crate::svg::usvg::Strut| {
            font.map_or([0.0; 2], |font| {
                line_extents(font, strut.font_size, &strut.style, vertical)
            })
        };
        let strut = extents_of(strut, &text.strut);
        let mut extents = vec![strut];
        // Each line's subtrees aligned with an edge: whether the bottom edge, and their
        // extents around their roots' baselines.
        let mut subtrees: Vec<HashMap<usize, (bool, [f32; 2])>> = vec![HashMap::new()];
        for chunk in &text.chunks {
            for _ in 0..chunk.line_breaks {
                extents.push(strut);
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
        } else {
            Transform::from_translate(x + glyph.dx as f32, -glyph.dy as f32)
        };

        positioned_glyphs.push(PositionedGlyph {
            glyph_ts: ts,
            // Will be set later.
            cluster_ts: Transform::default(),
            // Will be set later.
            span_ts: Transform::default(),
            units_per_em: glyph.font.units_per_em.get(),
            font_size,
            font: glyph.font.id,
            cluster: glyph.byte_idx.value(),
            x_advance: glyph.width as f32 / f32::from(glyph.font.units_per_em.get()),
            text: glyph.text.clone(),
            id: glyph.id,
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
    }
}

pub(crate) trait DatabaseExt {
    fn load_font(&self, id: ID) -> Option<ResolvedFont>;
    fn has_char(&self, id: ID, c: char) -> bool;
}

impl DatabaseExt for Database {
    #[inline(never)]
    fn load_font(&self, id: ID) -> Option<ResolvedFont> {
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
            Some(ResolvedFont {
                id,
                metrics,
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
    directional: &str,
    rtl: bool,
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
            directional,
            source,
            rtl,
            orientations,
            font.clone(),
            span,
            &resolver.face_settings(span.request, font.id, span.style),
            &resolver.variations(span.request, font.id),
            fontdb,
        )
    };
    let mut glyphs = shape(fontdb, &font, 0..text.len()).unwrap_or_default();
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
            .find(|glyph| glyph.is_missing() && active.contains(&glyph.byte_idx.value()))
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
            let Some(fallback) = fontdb.load_font(id) else {
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
                    .any(|glyph| glyph.is_missing())
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
            if let Some(candidate) =
                candidate.filter(|glyphs| glyphs.iter().all(|glyph| !glyph.is_missing()))
            {
                replacement = Some((group_end, group_source, fallback.clone(), candidate));
                break;
            }
            if group_end != end {
                let candidate = shape(fontdb, &fallback, source.value()..source_end);
                if let Some(candidate) =
                    candidate.filter(|glyphs| glyphs.iter().all(|glyph| !glyph.is_missing()))
                {
                    replacement = Some((end, source.value()..source_end, fallback, candidate));
                    break;
                }
            }
        }
        if let Some((end, mut source, fallback, mut replacement)) = replacement {
            let mut start = index;
            if let Some(run) = &previous
                && run.end == index
                && run.font.id == fallback.id
                && (run.source.end == source.start || source.end == run.source.start)
            {
                let combined = run.source.start.min(source.start)..run.source.end.max(source.end);
                let merged = shape(fontdb, &fallback, combined.clone());
                if let Some(merged) =
                    merged.filter(|glyphs| glyphs.iter().all(|glyph| !glyph.is_missing()))
                {
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
    directional: &str,
    source: std::ops::Range<usize>,
    rtl: bool,
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
        // specifies.
        let size_scale = style.font_adjustments.size_adjust.map_or(1.0, |adjust| {
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

        // SVG 2 §11.10.2.4: `direction` is the base direction of the text chunk's bidi paragraph.
        let base = if rtl {
            unicode_bidi::Level::rtl()
        } else {
            unicode_bidi::Level::ltr()
        };
        let bidi_info = unicode_bidi::BidiInfo::new(directional, Some(base));
        let paragraph = &bidi_info.paragraphs[0];
        let line = paragraph.range.clone();

        let mut glyphs = Vec::new();

        let (levels, runs) = bidi_info.visual_runs(paragraph, line);
        let vertical_forms = crate::fonts::has_feature(&rb_font, *b"vert")
            || crate::fonts::has_feature(&rb_font, *b"vrt2");
        // CSS Fonts 4 §6.6: the face's own capitals, or capitals scaled from uppercase forms,
        // as for the document's text.
        let plan = features::caps_plan(
            style.font_variant_caps,
            |tag| crate::fonts::has_feature(&rb_font, tag),
            style.font_synthesis.small_caps,
        );
        for run in &runs {
            let run = run.start.max(source.start)..run.end.min(source.end);
            if run.start >= run.end {
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
            if levels[run.start].is_rtl() {
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
                    levels[run.start].is_rtl(),
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
                if levels[caps_pieces[0].0.start].is_rtl() {
                    caps_pieces.reverse();
                }
                for (run, small) in caps_pieces {
                    let sub_text = &text[run.clone()];
                    if sub_text.is_empty() {
                        continue;
                    }

                    let ltr = levels[run.start].is_ltr();
                    let hb_direction = if vertical {
                        rustybuzz::Direction::TopToBottom
                    } else if ltr {
                        rustybuzz::Direction::LeftToRight
                    } else {
                        rustybuzz::Direction::RightToLeft
                    };

                    let mut buffer = rustybuzz::UnicodeBuffer::new();
                    if small {
                        for (start, upper) in
                            features::uppercase_clusters(text, run.clone(), &style.language)
                        {
                            for ch in upper.chars() {
                                buffer.add(ch, start as u32);
                            }
                        }
                    } else {
                        buffer.push_str(sub_text);
                    }
                    buffer.set_pre_context(&text[..run.start]);
                    buffer.set_post_context(&text[run.end..]);
                    buffer.set_direction(hb_direction);
                    if let Some(language) = &language {
                        buffer.set_language(language.clone());
                    }
                    buffer.guess_segment_properties();

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

    /// Reference to the source font.
    ///
    /// Each glyph can have it's own source font.
    pub(crate) font: Arc<ResolvedFont>,
}

impl Glyph {
    fn is_missing(&self) -> bool {
        self.id.0 == 0
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
    chunk
        .spans
        .iter()
        .find(|&span| span_contains(span, byte_offset))
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

    /// Converts byte position into a code point position.
    pub(crate) fn code_point_at(&self, text: &str) -> usize {
        text.char_indices()
            .take_while(|(i, _)| *i != self.0)
            .count()
    }

    /// Converts byte position into a character.
    pub(crate) fn char_from(&self, text: &str) -> char {
        text[self.0..].chars().next().unwrap()
    }
}
