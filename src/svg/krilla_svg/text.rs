use std::collections::HashMap;
use std::collections::hash_map::Entry;

use crate::svg::usvg::{FontOpticalSizing, FontVariation, PaintOrder};
use fontdb::Database;
use krilla::color::rgb;
use krilla::geom::Point;
use krilla::num::NormalizedF32;
use krilla::paint::{Fill, Stroke};
use krilla::surface::Surface;
use krilla::text::{Font, Glyph, GlyphId, KrillaGlyph};
use skrifa::{FontRef, MetadataProvider};
use smallvec::SmallVec;
use tiny_skia_path::Transform;

use crate::svg::krilla_svg::util::{UsvgTransformExt, convert_fill, convert_stroke};
use crate::svg::krilla_svg::{ProcessContext, path};

/// Render a text into a surface.
pub(crate) fn render(
    text: &crate::svg::usvg::Text,
    surface: &mut Surface,
    process_context: &mut ProcessContext,
) {
    // Glyph runs stroke in their own space; the outlines stroke in the host space.
    if text.non_scaling_stroke() {
        crate::svg::krilla_svg::group::render(text.flattened(), surface, process_context);
        return;
    }
    for span in text.layouted() {
        if !span.visible {
            continue;
        }

        if let Some(overline) = &span.overline {
            path::render(overline, surface, process_context);
        }

        if let Some(underline) = &span.underline {
            path::render(underline, surface, process_context);
        }

        for cluster in span
            .positioned_glyphs
            .chunk_by(|left, right| left.cluster == right.cluster)
        {
            if cluster.len() > 1 && render_cluster_batch(cluster, span, surface, process_context) {
                continue;
            }
            if cluster.len() > 1 && process_context.svg_settings.embed_text {
                surface.with_actual_text(&cluster[0].text, |surface| {
                    render_cluster(cluster, span, surface, process_context)
                });
            } else {
                render_cluster(cluster, span, surface, process_context);
            }
        }

        if let Some(line_through) = &span.line_through {
            path::render(line_through, surface, process_context);
        }
    }
}

fn render_cluster(
    glyphs: &[crate::svg::usvg::layout::PositionedGlyph],
    span: &crate::svg::usvg::layout::Span,
    surface: &mut Surface,
    process_context: &mut ProcessContext,
) {
    for glyph in glyphs {
        // Ignore glyph if font can't be fetched.
        let Some(font) = process_context.fonts.retrieve(span, glyph.font) else {
            continue;
        };

        let upem = font.units_per_em();

        // The text transform contains the scale transform `font_size / upem`, we need to invert that
        // so we only get the raw transform to account for the glyph position, and the font size
        // is being taken care of by krilla.
        let transform = glyph.transform().pre_concat(Transform::from_scale(
            upem / span.font_size.get(),
            upem / span.font_size.get(),
        ));

        let Some(inverted) = transform.invert() else {
            continue;
        };

        let positioned = [KrillaGlyph::new(
            GlyphId::new(glyph.id.0 as u32),
            0.0,
            0.0,
            0.0,
            0.0,
            0..glyph.text.len(),
            None,
        )];
        paint_glyphs(
            &positioned,
            font,
            &glyph.text,
            transform,
            inverted,
            span,
            surface,
            process_context,
        );
    }
}

fn paint_glyphs(
    glyphs: &[impl Glyph],
    font: Font,
    text: &str,
    transform: Transform,
    inverted: Transform,
    span: &crate::svg::usvg::layout::Span,
    surface: &mut Surface,
    process_context: &mut ProcessContext,
) {
    // Keep text origins in SVG coordinates across replacement-text font runs.
    let origin = Point::from_xy(transform.tx, transform.ty);
    let transform = transform.pre_translate(-origin.x, -origin.y);
    let inverted = Transform::from_translate(origin.x, origin.y).pre_concat(inverted);
    // Paint remains in the enclosing SVG coordinate system.
    let fill = span
        .fill
        .as_ref()
        .map(|f| convert_fill(f, surface.stream_builder(), process_context, inverted));

    let stroke = span
        .stroke
        .as_ref()
        .map(|s| convert_stroke(s, surface.stream_builder(), process_context, inverted));

    let draw_op = |s: &mut Surface,
                   fill: Option<Fill>,
                   stroke: Option<Stroke>,
                   font: Font,
                   embed_text: bool| {
        s.set_fill(fill);
        s.set_stroke(stroke);

        s.draw_glyphs(
            origin,
            glyphs,
            font,
            text,
            span.font_size.get(),
            !embed_text,
        );
    };

    surface.push_transform(&transform.to_krilla());

    if fill.is_none() && stroke.is_none() {
        // Emulate invisible glyph by drawing it with an opacity of zero.
        draw_op(
            surface,
            Some(Fill {
                paint: rgb::Color::new(0, 0, 0).into(),
                opacity: NormalizedF32::ZERO,
                rule: Default::default(),
            }),
            None,
            font,
            process_context.svg_settings.embed_text,
        )
    } else if matches!(span.paint_order, PaintOrder::FillAndStroke)
        || fill.is_none()
        || stroke.is_none()
    {
        draw_op(
            surface,
            fill,
            stroke,
            font.clone(),
            process_context.svg_settings.embed_text,
        );
    } else {
        // Paint order stroke and fill, and we have BOTH, a fill and
        // stroke.

        // We always draw the text outlined in this case, so that
        // text won't be embedded twice.
        draw_op(surface, None, stroke, font.clone(), false);

        draw_op(
            surface,
            fill,
            None,
            font.clone(),
            process_context.svg_settings.embed_text,
        );
    }

    surface.pop();
}

struct ClusterGlyph(KrillaGlyph);
impl Glyph for ClusterGlyph {
    fn glyph_id(&self) -> GlyphId {
        self.0.glyph_id()
    }
    fn requires_codepoint_mapping(&self) -> bool {
        true
    }
    fn text_range(&self) -> std::ops::Range<usize> {
        self.0.text_range()
    }
    fn x_advance(&self, size: f32) -> f32 {
        self.0.x_advance(size)
    }
    fn x_offset(&self, size: f32) -> f32 {
        self.0.x_offset(size)
    }
    fn y_offset(&self, size: f32) -> f32 {
        self.0.y_offset(size)
    }
    fn y_advance(&self, size: f32) -> f32 {
        self.0.y_advance(size)
    }
    fn location(&self) -> Option<krilla::surface::Location> {
        self.0.location()
    }
}

fn render_cluster_batch(
    glyphs: &[crate::svg::usvg::layout::PositionedGlyph],
    span: &crate::svg::usvg::layout::Span,
    surface: &mut Surface,
    context: &mut ProcessContext,
) -> bool {
    let first = &glyphs[0];
    if glyphs.iter().any(|glyph| glyph.font != first.font) {
        return false;
    }
    let Some(font) = context.fonts.retrieve(span, first.font) else {
        return false;
    };
    let size = span.font_size.get();
    let transform = first.transform().pre_concat(Transform::from_scale(
        font.units_per_em() / size,
        font.units_per_em() / size,
    ));
    let Some(inverted) = transform.invert() else {
        return false;
    };
    let mut pen = 0.0;
    let mut positioned = Vec::with_capacity(glyphs.len());
    for glyph in glyphs {
        let current = glyph.transform().pre_concat(Transform::from_scale(
            font.units_per_em() / size,
            font.units_per_em() / size,
        ));
        if [current.sx, current.kx, current.ky, current.sy]
            != [transform.sx, transform.kx, transform.ky, transform.sy]
        {
            return false;
        }
        let mut point = tiny_skia_path::Point::from_xy(current.tx, current.ty);
        inverted.map_point(&mut point);
        positioned.push(ClusterGlyph(KrillaGlyph::new(
            GlyphId::new(glyph.id.0 as u32),
            glyph.x_advance,
            (point.x - pen) / size,
            -point.y / size,
            0.0,
            0..first.text.len(),
            None,
        )));
        pen += glyph.x_advance * size;
    }
    paint_glyphs(
        &positioned,
        font,
        &first.text,
        transform,
        inverted,
        span,
        surface,
        context,
    );
    true
}

/// Manages the krilla fonts used by an SVG.
pub(crate) struct Fonts<'a> {
    db: &'a Database,
    fonts: HashMap<FontInstance, Option<Font>>,
    supported_axes: HashMap<fontdb::ID, SmallVec<[[u8; 4]; 2]>>,
}

/// Identifies a font at specific variation coordinates.
type FontInstance = (fontdb::ID, FontVariations);
type FontVariations = SmallVec<[FontVariation; 2]>;

impl<'a> Fonts<'a> {
    pub(crate) fn new(db: &'a Database) -> Self {
        Self {
            db,
            fonts: HashMap::new(),
            supported_axes: HashMap::new(),
        }
    }

    /// Retrieves the font identified by `id` from the cache or loads it.
    fn retrieve(&mut self, span: &crate::svg::usvg::layout::Span, id: fontdb::ID) -> Option<Font> {
        let variations = self.resolve_variations(span, id);
        match self.fonts.entry((id, variations)) {
            Entry::Occupied(entry) => entry.get().clone(),
            Entry::Vacant(entry) => {
                // Every face is registered from memory, so its shared bytes are borrowed directly.
                let font = self.db.face(id).and_then(|face| {
                    let fontdb::Source::Binary(font_data) = &face.source;
                    let variations = &entry.key().1;
                    let coords: SmallVec<[_; 2]> = variations
                        .iter()
                        .map(|var| (krilla::text::Tag::new(&var.tag), var.value))
                        .collect();
                    Font::new_variable(font_data.clone().into(), face.index, &coords)
                });

                entry.insert(font.clone());
                font
            }
        }
    }

    /// Resolves which variations are applicable for the given font (filtering out undefined axis
    /// values and taking into account optical sizing).
    fn resolve_variations(
        &mut self,
        span: &crate::svg::usvg::layout::Span,
        id: fontdb::ID,
    ) -> FontVariations {
        let supported_axes = self.supported_axes.entry(id).or_insert_with(|| {
            self.db
                .with_face_data(id, |data, index| {
                    FontRef::from_index(data, index)
                        .into_iter()
                        .flat_map(|font| font.axes().iter())
                        .map(|axis| axis.tag().to_be_bytes())
                        .collect()
                })
                .unwrap_or_default()
        });
        let is_supported = |tag| supported_axes.contains(tag);

        let mut variations: FontVariations = span
            .variations
            .iter()
            .filter(|var| is_supported(&var.tag))
            .copied()
            .collect();

        // Set the font size for optical sizing if desired, supported, and not manually set.
        const OPSZ: &[u8; 4] = b"opsz";
        if span.font_optical_sizing == FontOpticalSizing::Auto
            && is_supported(OPSZ)
            && !variations.iter().any(|v| v.tag == *OPSZ)
        {
            variations.push(FontVariation {
                tag: *OPSZ,
                value: span.font_size.get(),
            });
        }

        variations
    }
}
