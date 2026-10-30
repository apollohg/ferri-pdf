// Copyright 2022 the Resvg Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use std::mem;
use std::sync::Arc;

use fontdb::{Database, ID};
use rustybuzz::ttf_parser;
use rustybuzz::ttf_parser::{GlyphId, RasterImageFormat};
use tiny_skia_path::{NonZeroRect, Size, Transform};

use crate::svg::usvg::*;

fn resolve_rendering_mode(text: &Text) -> ShapeRendering {
    match text.rendering_mode {
        TextRendering::OptimizeSpeed => ShapeRendering::CrispEdges,
        TextRendering::OptimizeLegibility => ShapeRendering::GeometricPrecision,
        TextRendering::GeometricPrecision => ShapeRendering::GeometricPrecision,
    }
}

fn push_outline_paths(
    span: &layout::Span,
    builder: &mut tiny_skia_path::PathBuilder,
    new_children: &mut Vec<Node>,
    rendering_mode: ShapeRendering,
    host: Option<Transform>,
) {
    let builder = mem::replace(builder, tiny_skia_path::PathBuilder::new());

    if let Some(path) = builder.finish().and_then(|p| {
        Path::new(
            String::new(),
            span.visible,
            span.fill.clone(),
            span.stroke.clone(),
            span.paint_order,
            rendering_mode,
            Arc::new(p),
            Transform::default(),
        )
    }) && let Some(path) = host_stroked(path, host)
    {
        new_children.push(Node::Path(Box::new(path)));
    }
}

/// `path`, stroked in the host coordinate space when `host`, the text's absolute transform,
/// is given for `vector-effect: non-scaling-stroke` (SVG 2 §13.6).
fn host_stroked(path: Path, host: Option<Transform>) -> Option<Path> {
    let Some(host) = host else {
        return Some(path);
    };
    Path::new(
        path.id,
        path.visible,
        path.fill,
        path.stroke,
        path.paint_order,
        path.rendering_mode,
        path.data,
        host,
    )?
    .with_non_scaling_stroke()
}

/// The text's outlines, their stroke bounding box, and the layers its spans' `text-shadow`
/// casts beneath them.
pub(crate) fn flatten(
    text: &mut Text,
    cache: &mut Cache,
) -> Option<(Group, NonZeroRect, Vec<Node>)> {
    let mut new_children = vec![];
    let mut shadows = vec![];

    let rendering_mode = resolve_rendering_mode(text);
    let host = text.non_scaling_stroke.then_some(text.abs_transform);

    for span in &text.layouted {
        let span_start = new_children.len();
        if let Some(path) = span.overline.as_ref() {
            let mut path = path.clone();
            path.rendering_mode = rendering_mode;
            if let Some(path) = host_stroked(path, host) {
                new_children.push(Node::Path(Box::new(path)));
            }
        }

        if let Some(path) = span.underline.as_ref() {
            let mut path = path.clone();
            path.rendering_mode = rendering_mode;
            if let Some(path) = host_stroked(path, host) {
                new_children.push(Node::Path(Box::new(path)));
            }
        }

        // Instead of always processing each glyph separately, we always collect
        // as many outline glyphs as possible by pushing them into the span_builder
        // and only if we encounter a different glyph, or we reach the very end of the
        // span to we push the actual outline paths into new_children. This way, we don't need
        // to create a new path for every glyph if we have many consecutive glyphs
        // with just outlines (which is the most common case).
        let mut span_builder = tiny_skia_path::PathBuilder::new();

        // For variable fonts, we need to extract the outline with variations applied.
        // We can't use the cache here since the outline depends on variation values.
        let has_explicit_variations = !span.variations.is_empty();
        let foreground = colr_foreground(span);

        for glyph in span
            .positioned_glyphs
            .iter()
            .flat_map(layout::PositionedGlyph::copies)
        {
            // A (best-effort conversion of a) COLR glyph.
            if let Some(tree) = cache.fontdb_colr(
                glyph.font,
                glyph.id,
                &ColrPaint {
                    variations: &span.variations,
                    font_size: glyph.font_size(),
                    optical_sizing: span.font_optical_sizing,
                    palette: glyph.paint.palette.as_deref(),
                    foreground,
                },
            ) {
                let mut group = Group {
                    transform: glyph.colr_transform(),
                    ..Group::empty()
                };
                // TODO: Probably need to update abs_transform of children?
                group.children.push(Node::Group(Box::new(tree.root)));
                group.calculate_bounding_boxes();

                new_children.push(Node::Group(Box::new(group)));
            }
            // An SVG glyph. Will return the usvg node containing the glyph descriptions.
            else if let Some(node) = cache.fontdb_svg(glyph.font, glyph.id) {
                push_outline_paths(
                    span,
                    &mut span_builder,
                    &mut new_children,
                    rendering_mode,
                    host,
                );

                let mut group = Group {
                    transform: glyph.svg_transform(),
                    ..Group::empty()
                };
                // TODO: Probably need to update abs_transform of children?
                group.children.push(node);
                group.calculate_bounding_boxes();

                new_children.push(Node::Group(Box::new(group)));
            }
            // A bitmap glyph.
            else if let Some(img) = cache.fontdb_raster(glyph.font, glyph.id) {
                push_outline_paths(
                    span,
                    &mut span_builder,
                    &mut new_children,
                    rendering_mode,
                    host,
                );

                let transform = if img.is_sbix {
                    glyph.sbix_transform(
                        img.x as f32,
                        img.y as f32,
                        img.glyph_bbox.map(|bbox| bbox.x_min).unwrap_or(0) as f32,
                        img.glyph_bbox.map(|bbox| bbox.y_min).unwrap_or(0) as f32,
                        img.pixels_per_em as f32,
                        img.image.size.height(),
                    )
                } else {
                    glyph.cbdt_transform(
                        img.x as f32,
                        img.y as f32,
                        img.pixels_per_em as f32,
                        img.image.size.height(),
                    )
                };

                let mut group = Group {
                    transform,
                    ..Group::empty()
                };
                group.children.push(Node::Image(Box::new(img.image)));
                group.calculate_bounding_boxes();

                new_children.push(Node::Group(Box::new(group)));
            } else {
                // Only bypass cache if: explicit variations OR (auto opsz AND font has opsz axis)
                let needs_variations = has_explicit_variations
                    || (span.font_optical_sizing == FontOpticalSizing::Auto
                        && cache.has_opsz_axis(glyph.font));

                let outline = if needs_variations {
                    cache.fontdb.outline_with_variations(
                        glyph.font,
                        glyph.id,
                        &span.variations,
                        glyph.font_size(),
                        span.font_optical_sizing,
                    )
                } else {
                    cache.fontdb_outline(glyph.font, glyph.id)
                };

                if let Some(outline) = outline.and_then(|p| p.transform(glyph.outline_transform()))
                {
                    span_builder.push_path(&outline);
                }
            }
        }

        push_outline_paths(
            span,
            &mut span_builder,
            &mut new_children,
            rendering_mode,
            host,
        );

        for path in [&span.line_through, &span.emphasis].into_iter().flatten() {
            let mut path = path.clone();
            path.rendering_mode = rendering_mode;
            if let Some(path) = host_stroked(path, host) {
                new_children.push(Node::Path(Box::new(path)));
            }
        }

        shadows.extend(shadow_layers(
            &span.shadows,
            &new_children[span_start..],
            cache,
        ));

        // A span composites in its own element's colour space (SVG 2 §13.9).
        if span.linear_light != text.linear_light && new_children.len() > span_start {
            let mut group = Group {
                linear_light: span.linear_light,
                ..Group::empty()
            };
            group.children = new_children.split_off(span_start);
            group.calculate_bounding_boxes();
            new_children.push(Node::Group(Box::new(group)));
        }
    }

    let mut group = Group {
        id: text.id.clone(),
        linear_light: text.linear_light,
        ..Group::empty()
    };

    for child in new_children {
        group.children.push(child);
    }

    group.calculate_bounding_boxes();
    let stroke_bbox = group.stroke_bounding_box().to_non_zero_rect()?;
    Some((group, stroke_bbox, shadows))
}

/// The layers `shadows` cast from `ink`, a span's flattened glyphs and decorations (CSS Text
/// Decoration 3 §4): each fills them opaquely in its colour, offset and blurred, with the
/// colour's alpha as the layer's opacity, so overlapping shapes composite together. The first
/// shadow is on top.
fn shadow_layers(
    shadows: &[crate::style::TextShadow],
    ink: &[Node],
    cache: &mut Cache,
) -> Vec<Node> {
    let mut layers = Vec::new();
    for shadow in shadows.iter().rev() {
        let color = shadow.color.used;
        let paint = Paint::Color(Color::new_rgb(color.rgb[0], color.rgb[1], color.rgb[2]));
        let [dx, dy] = shadow
            .offset
            .map(|length| length / crate::style::POINTS_PER_PIXEL);
        let transform = Transform::from_translate(dx, dy);
        let mut layer = Group {
            transform,
            abs_transform: transform,
            opacity: Opacity::new_clamped(color.alpha),
            children: ink
                .iter()
                .map(|node| silhouette(node, &paint, cache))
                .collect(),
            ..Group::empty()
        };
        layer.calculate_bounding_boxes();
        if shadow.blur > 0.0 {
            let cast = crate::paint::BoxShadow {
                offset: shadow.offset,
                blur: shadow.blur,
                spread: 0.0,
                color,
                inset: false,
            };
            let extent = cast.extent() / crate::style::POINTS_PER_PIXEL;
            let deviation = PositiveF32::new(cast.sigma() / crate::style::POINTS_PER_PIXEL)
                .unwrap_or(PositiveF32::ZERO);
            // The layer's bounding box is in its own coordinates, before its offset.
            let Some(region) = outset(layer.layer_bounding_box(), extent) else {
                continue;
            };
            layer.filters = vec![Arc::new(filter::Filter {
                id: cache.gen_filter_id(),
                rect: region,
                primitives: vec![filter::Primitive {
                    rect: region,
                    color_interpolation: filter::ColorInterpolation::SRGB,
                    result: SHADOW_RESULT.to_owned(),
                    kind: filter::Kind::GaussianBlur(filter::GaussianBlur {
                        input: filter::Input::SourceGraphic,
                        std_dev_x: deviation,
                        std_dev_y: deviation,
                    }),
                }],
            })];
            layer.calculate_bounding_boxes();
        }
        layers.push(Node::Group(Box::new(layer)));
    }
    layers
}

/// The filter result naming a blurred shadow.
const SHADOW_RESULT: &str = "shadow";

fn outset(rect: NonZeroRect, by: f32) -> Option<NonZeroRect> {
    NonZeroRect::from_ltrb(
        rect.left() - by,
        rect.top() - by,
        rect.right() + by,
        rect.bottom() + by,
    )
}

/// `node` with every fill and stroke opaque `paint`, and images recoloured to it, keeping their
/// coverage.
fn silhouette(node: &Node, paint: &Paint, cache: &mut Cache) -> Node {
    match node {
        Node::Path(path) => {
            let mut path = path.clone();
            if let Some(fill) = &mut path.fill {
                fill.paint = paint.clone();
                fill.opacity = Opacity::ONE;
                fill.context_element = None;
            }
            if let Some(stroke) = &mut path.stroke {
                stroke.paint = paint.clone();
                stroke.opacity = Opacity::ONE;
                stroke.context_element = None;
            }
            Node::Path(path)
        }
        Node::Group(group) => {
            let mut group = group.clone();
            group.children = group
                .children
                .iter()
                .map(|child| silhouette(child, paint, cache))
                .collect();
            Node::Group(group)
        }
        Node::Image(image) => {
            let Paint::Color(color) = paint else {
                unreachable!("silhouettes are painted in a shadow colour")
            };
            let channel = |value: u8| f32::from(value) / f32::from(u8::MAX);
            let [red, green, blue] = [color.red, color.green, color.blue].map(channel);
            let region = image.abs_bounding_box;
            let mut group = Group {
                filters: vec![Arc::new(filter::Filter {
                    id: cache.gen_filter_id(),
                    rect: region,
                    primitives: vec![filter::Primitive {
                        rect: region,
                        color_interpolation: filter::ColorInterpolation::SRGB,
                        result: SHADOW_RESULT.to_owned(),
                        kind: filter::Kind::ColorMatrix(filter::ColorMatrix {
                            input: filter::Input::SourceGraphic,
                            kind: filter::ColorMatrixKind::Matrix(vec![
                                0.0, 0.0, 0.0, 0.0, red, //
                                0.0, 0.0, 0.0, 0.0, green, //
                                0.0, 0.0, 0.0, 0.0, blue, //
                                0.0, 0.0, 0.0, 1.0, 0.0,
                            ]),
                        }),
                    }],
                })],
                children: vec![Node::Image(image.clone())],
                ..Group::empty()
            };
            group.calculate_bounding_boxes();
            Node::Group(Box::new(group))
        }
        Node::Text(_) => node.clone(),
    }
}

struct PathBuilder {
    builder: tiny_skia_path::PathBuilder,
}

impl ttf_parser::OutlineBuilder for PathBuilder {
    fn move_to(&mut self, x: f32, y: f32) {
        self.builder.move_to(x, y);
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.builder.line_to(x, y);
    }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        self.builder.quad_to(x1, y1, x, y);
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.builder.cubic_to(x1, y1, x2, y2, x, y);
    }

    fn close(&mut self) {
        self.builder.close();
    }
}

pub(crate) trait DatabaseExt {
    fn outline(&self, id: ID, glyph_id: GlyphId) -> Option<tiny_skia_path::Path>;
    fn outline_with_variations(
        &self,
        id: ID,
        glyph_id: GlyphId,
        variations: &[FontVariation],
        font_size: f32,
        font_optical_sizing: FontOpticalSizing,
    ) -> Option<tiny_skia_path::Path>;
    fn has_opsz_axis(&self, id: ID) -> bool;
    fn raster(&self, id: ID, glyph_id: GlyphId) -> Option<BitmapImage>;
    fn svg(&self, id: ID, glyph_id: GlyphId) -> Option<Node>;
    fn colr(&self, id: ID, glyph_id: GlyphId, paint: &ColrPaint<'_>) -> Option<Tree>;
}

/// How a COLR glyph paints: at its span's variation coordinates, with its palette, or the
/// face's default, and its foreground colour.
pub(crate) struct ColrPaint<'a> {
    variations: &'a [FontVariation],
    font_size: f32,
    optical_sizing: FontOpticalSizing,
    palette: Option<&'a [crate::paint::Color]>,
    foreground: crate::paint::Color,
}
impl ColrPaint<'_> {
    /// The bits identifying the painting.
    pub(crate) fn key(&self) -> Vec<u32> {
        let mut key = vec![
            self.font_size.to_bits(),
            u32::from(self.optical_sizing == FontOpticalSizing::Auto),
        ];
        for variation in self.variations {
            key.extend([u32::from_be_bytes(variation.tag), variation.value.to_bits()]);
        }
        for color in self.palette.into_iter().flatten().chain([&self.foreground]) {
            let [red, green, blue] = color.rgb;
            key.extend([
                u32::from_be_bytes([0, red, green, blue]),
                color.alpha.to_bits(),
            ]);
        }
        key
    }
    /// The variation settings, with the font size on an `opsz` axis that `font-optical-sizing:
    /// auto` sets where they do not.
    fn settings(&self) -> Vec<(skrifa::Tag, f32)> {
        const OPSZ: [u8; 4] = *b"opsz";
        let mut settings: Vec<_> = self
            .variations
            .iter()
            .map(|variation| (skrifa::Tag::new(&variation.tag), variation.value))
            .collect();
        if self.optical_sizing == FontOpticalSizing::Auto
            && !self
                .variations
                .iter()
                .any(|variation| variation.tag == OPSZ)
        {
            settings.push((skrifa::Tag::new(&OPSZ), self.font_size));
        }
        settings
    }
}

/// The colour COLR glyphs take as the foreground, as the vector renderer takes it: the span's
/// fill colour, else its stroke's, else black.
fn colr_foreground(span: &layout::Span) -> crate::paint::Color {
    let color = |paint: &Paint| match paint {
        Paint::Color(color) => Some(crate::paint::Color {
            rgb: [color.red, color.green, color.blue],
            alpha: 1.0,
        }),
        _ => None,
    };
    span.fill
        .as_ref()
        .and_then(|fill| color(&fill.paint))
        .or_else(|| span.stroke.as_ref().and_then(|stroke| color(&stroke.paint)))
        .unwrap_or(crate::paint::Color::BLACK)
}

#[derive(Clone)]
pub(crate) struct BitmapImage {
    image: Image,
    x: i16,
    y: i16,
    pixels_per_em: u16,
    glyph_bbox: Option<ttf_parser::Rect>,
    is_sbix: bool,
}

impl DatabaseExt for Database {
    #[inline(never)]
    fn outline(&self, id: ID, glyph_id: GlyphId) -> Option<tiny_skia_path::Path> {
        self.with_face_data(id, |data, face_index| -> Option<tiny_skia_path::Path> {
            let mut font = ttf_parser::Face::parse(data, face_index).ok()?;

            // For variable fonts, we need to set default variation values to get proper outlines
            if font.is_variable() {
                for axis in font.variation_axes() {
                    font.set_variation(axis.tag, axis.def_value);
                }
            }

            let mut builder = PathBuilder {
                builder: tiny_skia_path::PathBuilder::new(),
            };

            font.outline_glyph(glyph_id, &mut builder)?;
            builder.builder.finish()
        })?
    }

    #[inline(never)]
    fn outline_with_variations(
        &self,
        id: ID,
        glyph_id: GlyphId,
        variations: &[FontVariation],
        font_size: f32,
        font_optical_sizing: FontOpticalSizing,
    ) -> Option<tiny_skia_path::Path> {
        self.with_face_data(id, |data, face_index| -> Option<tiny_skia_path::Path> {
            let mut font = ttf_parser::Face::parse(data, face_index).ok()?;

            for v in variations {
                font.set_variation(ttf_parser::Tag::from_bytes(&v.tag), v.value);
            }

            // Auto-set opsz if font-optical-sizing is auto and not explicitly set
            if font_optical_sizing == FontOpticalSizing::Auto {
                let has_explicit_opsz = variations.iter().any(|v| v.tag == *b"opsz");
                if !has_explicit_opsz {
                    // Check if font has opsz axis
                    if let Some(axes) = font.tables().fvar {
                        let has_opsz_axis = axes
                            .axes
                            .into_iter()
                            .any(|axis| axis.tag == ttf_parser::Tag::from_bytes(b"opsz"));
                        if has_opsz_axis {
                            font.set_variation(ttf_parser::Tag::from_bytes(b"opsz"), font_size);
                        }
                    }
                }
            }

            let mut builder = PathBuilder {
                builder: tiny_skia_path::PathBuilder::new(),
            };

            font.outline_glyph(glyph_id, &mut builder)?;
            builder.builder.finish()
        })?
    }

    fn has_opsz_axis(&self, id: ID) -> bool {
        self.with_face_data(id, |data, face_index| -> Option<bool> {
            let font = ttf_parser::Face::parse(data, face_index).ok()?;
            let has_opsz = font.tables().fvar.map_or(false, |axes| {
                axes.axes
                    .into_iter()
                    .any(|axis| axis.tag == ttf_parser::Tag::from_bytes(b"opsz"))
            });
            Some(has_opsz)
        })
        .flatten()
        .unwrap_or(false)
    }

    fn raster(&self, id: ID, glyph_id: GlyphId) -> Option<BitmapImage> {
        self.with_face_data(id, |data, face_index| -> Option<BitmapImage> {
            let font = ttf_parser::Face::parse(data, face_index).ok()?;
            let image = font.glyph_raster_image(glyph_id, u16::MAX)?;

            if image.format == RasterImageFormat::PNG {
                let bitmap_image = BitmapImage {
                    image: Image {
                        id: String::new(),
                        visible: true,
                        size: Size::from_wh(image.width as f32, image.height as f32)?,
                        rendering_mode: ImageRendering::OptimizeQuality,
                        kind: ImageKind::PNG(Arc::new(image.data.into())),
                        abs_transform: Transform::default(),
                        abs_bounding_box: NonZeroRect::from_xywh(
                            0.0,
                            0.0,
                            image.width as f32,
                            image.height as f32,
                        )?,
                    },
                    x: image.x,
                    y: image.y,
                    pixels_per_em: image.pixels_per_em,
                    glyph_bbox: font.glyph_bounding_box(glyph_id),
                    // ttf-parser always checks sbix first, so if this table exists, it was used.
                    is_sbix: font.tables().sbix.is_some(),
                };

                return Some(bitmap_image);
            }

            None
        })?
    }

    fn svg(&self, id: ID, glyph_id: GlyphId) -> Option<Node> {
        // TODO: Technically not 100% accurate because the SVG format in a OTF font
        // is actually a subset/superset of a normal SVG, but it seems to work fine
        // for Twitter Color Emoji, so might as well use what we already have.

        // TODO: Glyph records can contain the data for multiple glyphs. We should
        // add a cache so we don't need to reparse the data every time.
        self.with_face_data(id, |data, face_index| -> Option<Node> {
            let font = ttf_parser::Face::parse(data, face_index).ok()?;
            let image = font.glyph_svg_image(glyph_id)?;
            let tree = Tree::from_data(image.data, &Options::default()).ok()?;

            // Twitter Color Emoji seems to always have one SVG record per glyph,
            // while Noto Color Emoji sometimes contains multiple ones. It's kind of hacky,
            // but the best we have for now.
            let node = if image.start_glyph_id == image.end_glyph_id {
                Node::Group(Box::new(tree.root))
            } else {
                tree.node_by_id(&format!("glyph{}", glyph_id.0))
                    .log_none(|| {
                        log::warn!("Failed to find SVG glyph node for glyph {}", glyph_id.0);
                    })
                    .cloned()?
            };

            Some(node)
        })?
    }

    fn colr(&self, id: ID, glyph_id: GlyphId, paint: &ColrPaint<'_>) -> Option<Tree> {
        use skrifa::MetadataProvider;
        self.with_face_data(id, |data, face_index| -> Option<Tree> {
            let font = skrifa::FontRef::from_index(data, face_index).ok()?;
            let location = font.axes().location(paint.settings());
            let default;
            let palette = match paint.palette {
                Some(palette) => palette,
                None => {
                    default = crate::fonts::default_palette(&font);
                    &default
                }
            };
            let document = super::colr::document(
                &font,
                (&location).into(),
                palette,
                paint.foreground,
                skrifa::GlyphId::new(u32::from(glyph_id.0)),
            )?;
            Tree::from_data(document.as_bytes(), &Options::default()).ok()
        })?
    }
}
