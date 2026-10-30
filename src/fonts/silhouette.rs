//! Glyph coverage for blurred text shadows: the area a glyph's outline, its COLR layers or
//! its PNG bitmap paints, rasterized opaquely.
use crate::fonts::FontVariation;
use tiny_skia::{FillRule, Mask, Pixmap, PixmapPaint, Stroke, Transform};
use ttf_parser::{GlyphId, OutlineBuilder, RasterImageFormat, colr};

const DEFAULT_PALETTE: u16 = 0;
const FOREGROUND: ttf_parser::RgbaColor = ttf_parser::RgbaColor {
    red: 0,
    green: 0,
    blue: 0,
    alpha: u8::MAX,
};

/// Outlines in font units, y up.
#[derive(Default)]
struct PathBuilder(tiny_skia::PathBuilder);
impl OutlineBuilder for PathBuilder {
    fn move_to(&mut self, x: f32, y: f32) {
        self.0.move_to(x, y);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.0.line_to(x, y);
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        self.0.quad_to(x1, y1, x, y);
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.0.cubic_to(x1, y1, x2, y2, x, y);
    }
    fn close(&mut self) {
        self.0.close();
    }
}

/// A face instance whose glyphs paint coverage into a pixmap.
pub(crate) struct Silhouettes<'a> {
    face: ttf_parser::Face<'a>,
}
impl<'a> Silhouettes<'a> {
    pub(crate) fn new(data: &'a [u8], variations: &[FontVariation]) -> Option<Self> {
        Self::at(
            data,
            0,
            variations
                .iter()
                .map(|variation| (variation.tag, variation.value)),
        )
    }
    /// The outlines of face `index` of `data` at the axis `variations`, tags and values.
    pub(crate) fn at(
        data: &'a [u8],
        index: u32,
        variations: impl IntoIterator<Item = ([u8; 4], f32)>,
    ) -> Option<Self> {
        let mut face = ttf_parser::Face::parse(data, index).ok()?;
        for (tag, value) in variations {
            face.set_variation(ttf_parser::Tag::from_bytes(&tag), value);
        }
        Some(Self { face })
    }
    /// The box `glyph`'s outline covers, in em with y up, or `None` where colour layers,
    /// bitmaps or SVG documents may paint beyond it. An empty outline covers its origin.
    pub(crate) fn outline_box(&self, glyph: u32) -> Option<[f32; 4]> {
        let tables = self.face.tables();
        if tables.colr.is_some()
            || tables.sbix.is_some()
            || tables.cbdt.is_some()
            || tables.svg.is_some()
        {
            return None;
        }
        let units = self.units_per_em();
        let id = GlyphId(u16::try_from(glyph).ok()?);
        Some(
            self.face
                .outline_glyph(id, &mut PathBuilder::default())
                .map_or([0.0; 4], |bounds| {
                    [bounds.x_min, bounds.y_min, bounds.x_max, bounds.y_max]
                        .map(|value| f32::from(value) / units)
                }),
        )
    }
    pub(crate) fn units_per_em(&self) -> f32 {
        f32::from(self.face.units_per_em())
    }
    fn outline(&self, glyph: GlyphId) -> Option<tiny_skia::Path> {
        let mut builder = PathBuilder::default();
        self.face.outline_glyph(glyph, &mut builder)?;
        builder.0.finish()
    }
    /// Paints `glyph` with `transform` from font units, and strokes its outline `stroke`
    /// font units wide when given.
    pub(crate) fn paint(
        &self,
        glyph: u32,
        transform: Transform,
        stroke: Option<f32>,
        target: &mut Pixmap,
    ) {
        let Ok(id) = u16::try_from(glyph).map(GlyphId) else {
            return;
        };
        let outline = self.outline(id);
        if let (Some(path), Some(width)) = (&outline, stroke) {
            let stroke = Stroke {
                width,
                ..Stroke::default()
            };
            target.stroke_path(path, &crate::paint::opaque(), &stroke, transform, None);
        }
        if self.face.is_color_glyph(id) {
            let mut painter = LayerPainter {
                silhouettes: self,
                target,
                transforms: vec![transform],
                current: None,
                clips: Vec::new(),
            };
            if self
                .face
                .paint_color_glyph(id, DEFAULT_PALETTE, FOREGROUND, &mut painter)
                .is_some()
            {
                return;
            }
        }
        if let Some(path) = &outline {
            target.fill_path(
                path,
                &crate::paint::opaque(),
                FillRule::Winding,
                transform,
                None,
            );
        } else if let Some(image) = self
            .face
            .glyph_raster_image(id, u16::MAX)
            .filter(|image| image.format == RasterImageFormat::PNG)
            && let Ok(bitmap) = Pixmap::decode_png(image.data)
        {
            // The image's bottom-left corner lies at (x, y) pixels from the glyph origin, y up.
            let scale = f32::from(self.face.units_per_em()) / f32::from(image.pixels_per_em);
            let placement = transform
                .pre_translate(
                    f32::from(image.x) * scale,
                    (f32::from(image.y) + bitmap.height() as f32) * scale,
                )
                .pre_scale(scale, -scale);
            target.draw_pixmap(
                0,
                0,
                bitmap.as_ref(),
                &PixmapPaint::default(),
                placement,
                None,
            );
        }
    }
}

/// Paints the union of a COLR glyph's painted layers, each inside its clips.
struct LayerPainter<'s, 'a, 'p> {
    silhouettes: &'s Silhouettes<'a>,
    target: &'p mut Pixmap,
    transforms: Vec<Transform>,
    current: Option<(tiny_skia::Path, Transform)>,
    clips: Vec<Mask>,
}
impl LayerPainter<'_, '_, '_> {
    fn transform(&self) -> Transform {
        *self.transforms.last().expect("the base transform stays")
    }
    /// Pushes the clip stack's area within `path`; a missing path clips everything.
    fn clip(&mut self, path: Option<(&tiny_skia::Path, Transform)>) {
        let mut mask = crate::paint::within(self.clips.last(), self.target)
            .expect("masks match the non-empty target");
        match path {
            Some((path, transform)) => {
                mask.intersect_path(path, FillRule::Winding, true, transform);
            }
            None => mask.clear(),
        }
        self.clips.push(mask);
    }
}
impl<'a> colr::Painter<'a> for LayerPainter<'_, 'a, '_> {
    fn outline_glyph(&mut self, glyph: GlyphId) {
        self.current = self
            .silhouettes
            .outline(glyph)
            .map(|path| (path, self.transform()));
    }
    fn paint(&mut self, _: colr::Paint<'a>) {
        if let Some(clip) = self.clips.last() {
            let full = tiny_skia::Rect::from_xywh(
                0.0,
                0.0,
                self.target.width() as f32,
                self.target.height() as f32,
            );
            if let Some(full) = full {
                self.target.fill_rect(
                    full,
                    &crate::paint::opaque(),
                    Transform::identity(),
                    Some(clip),
                );
            }
        } else if let Some((path, transform)) = &self.current {
            self.target.fill_path(
                path,
                &crate::paint::opaque(),
                FillRule::Winding,
                *transform,
                None,
            );
        }
    }
    fn push_clip(&mut self) {
        let current = self.current.take();
        self.clip(current.as_ref().map(|(path, transform)| (path, *transform)));
        self.current = current;
    }
    fn push_clip_box(&mut self, clip: colr::ClipBox) {
        let path = tiny_skia::Rect::from_ltrb(clip.x_min, clip.y_min, clip.x_max, clip.y_max)
            .map(tiny_skia::PathBuilder::from_rect);
        let transform = self.transform();
        self.clip(path.as_ref().map(|path| (path, transform)));
    }
    fn pop_clip(&mut self) {
        self.clips.pop();
    }
    fn push_layer(&mut self, _: colr::CompositeMode) {}
    fn pop_layer(&mut self) {}
    fn push_transform(&mut self, transform: ttf_parser::Transform) {
        let ttf_parser::Transform { a, b, c, d, e, f } = transform;
        let combined = self
            .transform()
            .pre_concat(Transform::from_row(a, b, c, d, e, f));
        self.transforms.push(combined);
    }
    fn pop_transform(&mut self) {
        if self.transforms.len() > 1 {
            self.transforms.pop();
        }
    }
}
