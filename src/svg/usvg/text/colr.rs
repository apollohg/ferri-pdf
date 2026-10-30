// Copyright 2024 the Resvg Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! COLR glyphs as SVG documents for the raster renderer, painted with the palette `font-palette`
//! selects and the text's colour as the foreground (CSS Fonts 4 §9.1).
use skrifa::color::{Brush, ColorPainter, ColorStop, CompositeMode, Extend};
use skrifa::instance::{LocationRef, Size};
use skrifa::outline::{DrawSettings, pen::SvgPen};
use skrifa::raw::types::BoundingBox;
use skrifa::{FontRef, GlyphId, MetadataProvider};
use tiny_skia_path::Transform;
use xmlwriter::XmlWriter;

/// The palette index of the text's foreground colour.
const FOREGROUND: u16 = u16::MAX;
const SVG_NAMESPACE: &str = "http://www.w3.org/2000/svg";

/// The SVG document drawing `glyph` of `font` at `location` in font units with y up, its
/// palette entries `palette` and foreground `foreground`; `None` for a glyph COLR does not
/// define or that names an entry the palette lacks.
pub(crate) fn document(
    font: &FontRef<'_>,
    location: LocationRef<'_>,
    palette: &[crate::paint::Color],
    foreground: crate::paint::Color,
    glyph: GlyphId,
) -> Option<String> {
    let color_glyph = font.color_glyphs().get(glyph)?;
    let mut painter = GlyphPainter {
        font,
        location,
        palette,
        foreground,
        svg: XmlWriter::new(xmlwriter::Options::default()),
        path: String::new(),
        outline_transform: Transform::identity(),
        transform: Transform::identity(),
        transforms: Vec::new(),
        gradient_index: 0,
        clip_path_index: 0,
        failed: false,
    };
    painter.svg.start_element("svg");
    painter.svg.write_attribute("xmlns", SVG_NAMESPACE);
    painter.svg.start_element("g");
    color_glyph.paint(location, &mut painter).ok()?;
    if painter.failed {
        return None;
    }
    Some(painter.svg.end_document())
}

struct GlyphPainter<'a> {
    font: &'a FontRef<'a>,
    location: LocationRef<'a>,
    palette: &'a [crate::paint::Color],
    foreground: crate::paint::Color,
    svg: XmlWriter,
    /// The innermost clip's outline, which a fill paints, and the transform it was drawn in.
    path: String,
    outline_transform: Transform,
    transform: Transform,
    transforms: Vec<Transform>,
    gradient_index: usize,
    clip_path_index: usize,
    /// Whether a paint named a palette entry that does not exist.
    failed: bool,
}

impl GlyphPainter<'_> {
    /// The colour and opacity of palette entry `index` with `alpha` applied.
    fn color(&mut self, index: u16, alpha: f32) -> ([u8; 3], f32) {
        let entry = if index == FOREGROUND {
            Some(self.foreground)
        } else {
            self.palette.get(usize::from(index)).copied()
        };
        let Some(entry) = entry else {
            self.failed = true;
            return ([0; 3], 0.0);
        };
        (entry.rgb, entry.alpha * alpha)
    }

    fn outline(&self, glyph: GlyphId) -> Option<String> {
        let outline = self.font.outline_glyphs().get(glyph)?;
        let mut pen = SvgPen::new();
        outline
            .draw(
                DrawSettings::unhinted(Size::unscaled(), self.location),
                &mut pen,
            )
            .ok()?;
        Some(pen.to_string())
    }

    fn write_transform(&mut self, name: &str, transform: Transform) {
        if transform.is_identity() {
            return;
        }
        let Transform {
            sx,
            ky,
            kx,
            sy,
            tx,
            ty,
        } = transform;
        self.svg
            .write_attribute_fmt(name, format_args!("matrix({sx} {ky} {kx} {sy} {tx} {ty})"));
    }

    fn write_stops(&mut self, stops: &[ColorStop]) {
        for stop in stops {
            let ([red, green, blue], opacity) = self.color(stop.palette_index, stop.alpha);
            self.svg.start_element("stop");
            self.svg.write_attribute("offset", &stop.offset);
            self.svg
                .write_attribute_fmt("stop-color", format_args!("rgb({red}, {green}, {blue})"));
            self.svg.write_attribute("stop-opacity", &opacity);
            self.svg.end_element();
        }
    }

    fn write_extend(&mut self, extend: Extend) {
        self.svg.write_attribute(
            "spreadMethod",
            match extend {
                Extend::Repeat => "repeat",
                Extend::Reflect => "reflect",
                Extend::Pad | Extend::Unknown => "pad",
            },
        );
    }

    /// The innermost clip's outline, filled with `fill`.
    fn write_path(&mut self, fill: &str, opacity: Option<f32>) {
        self.svg.start_element("path");
        self.svg.write_attribute("fill", fill);
        if let Some(opacity) = opacity {
            self.svg.write_attribute("fill-opacity", &opacity);
        }
        let transform = self.outline_transform;
        self.write_transform("transform", transform);
        self.svg.write_attribute("d", &self.path);
        self.svg.end_element();
    }

    /// The current transform in the space of the outline it fills.
    fn gradient_transform(&self) -> Transform {
        self.outline_transform
            .invert()
            .unwrap_or_default()
            .pre_concat(self.transform)
    }

    fn gradient_id(&mut self) -> String {
        self.gradient_index += 1;
        format!("g{}", self.gradient_index)
    }

    fn clip_with(&mut self, path: String) {
        self.clip_path_index += 1;
        let id = format!("c{}", self.clip_path_index);
        self.svg.start_element("clipPath");
        self.svg.write_attribute("id", &id);
        self.svg.start_element("path");
        let transform = self.transform;
        self.write_transform("transform", transform);
        self.svg.write_attribute("d", &path);
        self.svg.end_element();
        self.svg.end_element();
        self.svg.start_element("g");
        self.svg
            .write_attribute_fmt("clip-path", format_args!("url(#{id})"));
        self.path = path;
        self.outline_transform = self.transform;
    }
}

impl ColorPainter for GlyphPainter<'_> {
    fn push_transform(&mut self, transform: skrifa::color::Transform) {
        self.transforms.push(self.transform);
        self.transform = self.transform.pre_concat(Transform::from_row(
            transform.xx,
            transform.yx,
            transform.xy,
            transform.yy,
            transform.dx,
            transform.dy,
        ));
    }

    fn pop_transform(&mut self) {
        if let Some(transform) = self.transforms.pop() {
            self.transform = transform;
        }
    }

    fn push_clip_glyph(&mut self, glyph: GlyphId) {
        let Some(path) = self.outline(glyph) else {
            self.failed = true;
            return;
        };
        self.clip_with(path);
    }

    fn push_clip_box(&mut self, clip: BoundingBox<f32>) {
        let BoundingBox {
            x_min,
            y_min,
            x_max,
            y_max,
        } = clip;
        self.clip_with(format!(
            "M {x_min} {y_min} L {x_max} {y_min} L {x_max} {y_max} L {x_min} {y_max} Z"
        ));
    }

    fn pop_clip(&mut self) {
        self.svg.end_element();
    }

    fn fill(&mut self, brush: Brush<'_>) {
        match brush {
            Brush::Solid {
                palette_index,
                alpha,
            } => {
                let ([red, green, blue], opacity) = self.color(palette_index, alpha);
                self.write_path(&format!("rgb({red}, {green}, {blue})"), Some(opacity));
            }
            Brush::LinearGradient {
                p0,
                p1,
                color_stops,
                extend,
            } => {
                let id = self.gradient_id();
                let transform = self.gradient_transform();
                self.svg.start_element("linearGradient");
                self.svg.write_attribute("id", &id);
                self.svg.write_attribute("x1", &p0.x);
                self.svg.write_attribute("y1", &p0.y);
                self.svg.write_attribute("x2", &p1.x);
                self.svg.write_attribute("y2", &p1.y);
                self.svg.write_attribute("gradientUnits", "userSpaceOnUse");
                self.write_extend(extend);
                self.write_transform("gradientTransform", transform);
                self.write_stops(color_stops);
                self.svg.end_element();
                self.write_path(&format!("url(#{id})"), None);
            }
            Brush::RadialGradient {
                c0,
                r0,
                c1,
                r1,
                color_stops,
                extend,
            } => {
                let id = self.gradient_id();
                let transform = self.gradient_transform();
                self.svg.start_element("radialGradient");
                self.svg.write_attribute("id", &id);
                self.svg.write_attribute("cx", &c1.x);
                self.svg.write_attribute("cy", &c1.y);
                self.svg.write_attribute("r", &r1);
                self.svg.write_attribute("fr", &r0);
                self.svg.write_attribute("fx", &c0.x);
                self.svg.write_attribute("fy", &c0.y);
                self.svg.write_attribute("gradientUnits", "userSpaceOnUse");
                self.write_extend(extend);
                self.write_transform("gradientTransform", transform);
                self.write_stops(color_stops);
                self.svg.end_element();
                self.write_path(&format!("url(#{id})"), None);
            }
            Brush::SweepGradient { .. } => log::warn!("Sweep gradients are not supported."),
        }
    }

    fn push_layer(&mut self, mode: CompositeMode) {
        self.svg.start_element("g");
        let mode = match mode {
            CompositeMode::SrcOver => "normal",
            CompositeMode::Screen => "screen",
            CompositeMode::Overlay => "overlay",
            CompositeMode::Darken => "darken",
            CompositeMode::Lighten => "lighten",
            CompositeMode::ColorDodge => "color-dodge",
            CompositeMode::ColorBurn => "color-burn",
            CompositeMode::HardLight => "hard-light",
            CompositeMode::SoftLight => "soft-light",
            CompositeMode::Difference => "difference",
            CompositeMode::Exclusion => "exclusion",
            CompositeMode::Multiply => "multiply",
            CompositeMode::HslHue => "hue",
            CompositeMode::HslSaturation => "saturation",
            CompositeMode::HslColor => "color",
            CompositeMode::HslLuminosity => "luminosity",
            _ => {
                log::warn!("Unsupported blend mode: {mode:?}");
                "normal"
            }
        };
        self.svg.write_attribute_fmt(
            "style",
            format_args!("mix-blend-mode: {mode}; isolation: isolate"),
        );
    }

    fn pop_layer(&mut self) {
        self.svg.end_element();
    }
}
