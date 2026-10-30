use krilla::blend::BlendMode;
use krilla::color::{luma, rgb};
use krilla::geom::{Path, PathBuilder, Rect};
use krilla::mask::MaskType;
use krilla::num::NormalizedF32;
use krilla::paint::{
    Fill, FillRule, LineCap, LineJoin, LinearGradient, Paint, Pattern, RadialGradient,
    SpreadMethod, Stop, Stroke, StrokeDash,
};
use krilla::stream::StreamBuilder;
use tiny_skia::PathSegment;
use tiny_skia_path::Transform;

use crate::svg::krilla_svg::{ProcessContext, group};

/// Convert a usvg `SpreadMethod` into a krilla `SpreadMethod`.
pub(crate) fn convert_spread_method(
    spread_method: &crate::svg::usvg::SpreadMethod,
) -> SpreadMethod {
    match spread_method {
        crate::svg::usvg::SpreadMethod::Pad => SpreadMethod::Pad,
        crate::svg::usvg::SpreadMethod::Reflect => SpreadMethod::Reflect,
        crate::svg::usvg::SpreadMethod::Repeat => SpreadMethod::Repeat,
    }
}

/// Convert a usvg `Stop` into a krilla `Stop`.
pub(crate) fn convert_stop(stop: &crate::svg::usvg::Stop) -> Stop {
    Stop {
        offset: NormalizedF32::new(stop.offset().get()).unwrap(),
        color: rgb::Color::new(stop.color().red, stop.color().green, stop.color().blue).into(),
        opacity: NormalizedF32::new(stop.opacity().get()).unwrap(),
    }
}

/// Convert a usvg `Paint` into a krilla `Paint`.
pub(crate) fn convert_paint(
    paint: &crate::svg::usvg::Paint,
    mut stream_builder: StreamBuilder,
    process_context: &mut ProcessContext,
    // The additional transform is needed because in krilla, a transform to a shape will also apply
    // the transform to the paint server. However, in the case of SVG glyphs, we don't want the transform
    // to be shifted for each glyph we draw (since we draw them separately instead of in a glyph run),
    // so we need to apply an additional inverse transform to counter that effect.
    additional_transform: Transform,
) -> Paint {
    match paint {
        crate::svg::usvg::Paint::Color(c) => {
            if c.red == c.green && c.green == c.blue {
                // Use gray-scale color if possible.
                luma::Color::new(c.red).into()
            } else {
                rgb::Color::new(c.red, c.green, c.blue).into()
            }
        }
        crate::svg::usvg::Paint::LinearGradient(lg) => LinearGradient {
            x1: lg.x1(),
            y1: lg.y1(),
            x2: lg.x2(),
            y2: lg.y2(),
            transform: additional_transform.pre_concat(lg.transform()).to_krilla(),
            spread_method: convert_spread_method(&lg.spread_method()),
            stops: lg.stops().iter().map(convert_stop).collect::<Vec<_>>(),
            anti_alias: false,
        }
        .into(),
        crate::svg::usvg::Paint::RadialGradient(rg) => RadialGradient {
            cx: rg.cx(),
            cy: rg.cy(),
            cr: rg.r().get(),
            fx: rg.fx(),
            fy: rg.fy(),
            fr: 0.0,
            transform: additional_transform.pre_concat(rg.transform()).to_krilla(),
            spread_method: convert_spread_method(&rg.spread_method()),
            stops: rg.stops().iter().map(convert_stop).collect::<Vec<_>>(),
            anti_alias: false,
        }
        .into(),
        crate::svg::usvg::Paint::Pattern(pat) => {
            let mut surface = stream_builder.surface();
            group::render_root(pat.root(), &mut surface, process_context);
            surface.finish();
            let stream = stream_builder.finish();

            Pattern {
                stream,
                transform: additional_transform
                    .pre_concat(pat.transform())
                    .pre_concat(Transform::from_translate(pat.rect().x(), pat.rect().y()))
                    .to_krilla(),
                width: pat.rect().width(),
                height: pat.rect().height(),
            }
            .into()
        }
    }
}

/// Convert a usvg `LineCap` into a krilla `LineCap`.
pub(crate) fn convert_line_cap(line_cap: &crate::svg::usvg::LineCap) -> LineCap {
    match line_cap {
        crate::svg::usvg::LineCap::Butt => LineCap::Butt,
        crate::svg::usvg::LineCap::Round => LineCap::Round,
        crate::svg::usvg::LineCap::Square => LineCap::Square,
    }
}

/// Convert a usvg `LineJoin` into a krilla `LineJoin`.
pub(crate) fn convert_line_join(line_join: &crate::svg::usvg::LineJoin) -> LineJoin {
    match line_join {
        crate::svg::usvg::LineJoin::Miter => LineJoin::Miter,
        crate::svg::usvg::LineJoin::MiterClip => LineJoin::Miter,
        crate::svg::usvg::LineJoin::Round => LineJoin::Round,
        crate::svg::usvg::LineJoin::Bevel => LineJoin::Bevel,
    }
}

/// Convert a usvg `FillRule` into a krilla `FillRule`.
pub(crate) fn convert_fill_rule(fill_rule: &crate::svg::usvg::FillRule) -> FillRule {
    match fill_rule {
        crate::svg::usvg::FillRule::NonZero => FillRule::NonZero,
        crate::svg::usvg::FillRule::EvenOdd => FillRule::EvenOdd,
    }
}

/// Convert a usvg `Fill` into a krilla `Fill`.
pub(crate) fn convert_fill(
    fill: &crate::svg::usvg::Fill,
    stream_builder: StreamBuilder,
    process_context: &mut ProcessContext,
    additional_transform: Transform,
) -> Fill {
    Fill {
        paint: convert_paint(
            fill.paint(),
            stream_builder,
            process_context,
            additional_transform,
        ),
        opacity: NormalizedF32::new(fill.opacity().get()).unwrap(),
        rule: convert_fill_rule(&fill.rule()),
    }
}

/// Convert a usvg `Stroke` into a krilla `Stroke`.
pub(crate) fn convert_stroke(
    stroke: &crate::svg::usvg::Stroke,
    stream_builder: StreamBuilder,
    process_context: &mut ProcessContext,
    additional_transform: Transform,
) -> Stroke {
    let dash = stroke.dasharray().map(|dash_array| StrokeDash {
        offset: stroke.dashoffset(),
        array: dash_array.to_vec(),
    });

    Stroke {
        paint: convert_paint(
            stroke.paint(),
            stream_builder,
            process_context,
            additional_transform,
        ),
        width: stroke.width().get(),
        miter_limit: stroke.miterlimit().get(),
        line_cap: convert_line_cap(&stroke.linecap()),
        line_join: convert_line_join(&stroke.linejoin()),
        opacity: NormalizedF32::new(stroke.opacity().get()).unwrap(),
        dash,
    }
}

/// Convert a usvg `BlendMode` into a krilla `BlendMode`.
pub(crate) fn convert_blend_mode(blend_mode: &crate::svg::usvg::BlendMode) -> BlendMode {
    match blend_mode {
        crate::svg::usvg::BlendMode::Normal => BlendMode::Normal,
        crate::svg::usvg::BlendMode::Multiply => BlendMode::Multiply,
        crate::svg::usvg::BlendMode::Screen => BlendMode::Screen,
        crate::svg::usvg::BlendMode::Overlay => BlendMode::Overlay,
        crate::svg::usvg::BlendMode::Darken => BlendMode::Darken,
        crate::svg::usvg::BlendMode::Lighten => BlendMode::Lighten,
        crate::svg::usvg::BlendMode::ColorDodge => BlendMode::ColorDodge,
        crate::svg::usvg::BlendMode::ColorBurn => BlendMode::ColorBurn,
        crate::svg::usvg::BlendMode::HardLight => BlendMode::HardLight,
        crate::svg::usvg::BlendMode::SoftLight => BlendMode::SoftLight,
        crate::svg::usvg::BlendMode::Difference => BlendMode::Difference,
        crate::svg::usvg::BlendMode::Exclusion => BlendMode::Exclusion,
        crate::svg::usvg::BlendMode::Hue => BlendMode::Hue,
        crate::svg::usvg::BlendMode::Saturation => BlendMode::Saturation,
        crate::svg::usvg::BlendMode::Color => BlendMode::Color,
        crate::svg::usvg::BlendMode::Luminosity => BlendMode::Luminosity,
        crate::svg::usvg::BlendMode::PlusDarker | crate::svg::usvg::BlendMode::PlusLighter => {
            unreachable!("additive groups composite in pixels")
        }
    }
}

/// Convert a usvg `MaskType` into a krilla `MaskType`.
pub(crate) fn convert_mask_type(mask_type: &crate::svg::usvg::MaskType) -> MaskType {
    match mask_type {
        crate::svg::usvg::MaskType::Luminance => MaskType::Luminosity,
        crate::svg::usvg::MaskType::Alpha => MaskType::Alpha,
    }
}

/// Draws the premultiplied `pixmap` stretched over `area`.
pub(crate) fn draw_pixmap(
    pixmap: &tiny_skia::Pixmap,
    area: Rect,
    surface: &mut krilla::surface::Surface,
) {
    let demultiplied = pixmap
        .pixels()
        .iter()
        .flat_map(|p| {
            let c = p.demultiply();
            [c.red(), c.green(), c.blue(), c.alpha()]
        })
        .collect::<Vec<_>>();
    let image = krilla::image::Image::from_rgba8(demultiplied, pixmap.width(), pixmap.height());
    surface.push_transform(&krilla::geom::Transform::from_translate(
        area.left(),
        area.top(),
    ));
    surface.draw_image(
        image,
        krilla::geom::Size::from_wh(area.width(), area.height())
            .expect("a rectangle has a positive size"),
    );
    surface.pop();
}

pub(crate) trait RectExt {
    fn to_clip_path(&self) -> Path;
}

impl RectExt for Rect {
    fn to_clip_path(&self) -> Path {
        let mut path_builder = PathBuilder::new();
        path_builder.move_to(self.left(), self.top());
        path_builder.line_to(self.right(), self.top());
        path_builder.line_to(self.right(), self.bottom());
        path_builder.line_to(self.left(), self.bottom());
        path_builder.close();
        path_builder.finish().unwrap()
    }
}

pub(crate) trait KrillaTransformExt {
    fn to_usvg(&self) -> Transform;
}

impl KrillaTransformExt for krilla::geom::Transform {
    fn to_usvg(&self) -> Transform {
        Transform::from_row(
            self.sx(),
            self.ky(),
            self.kx(),
            self.sy(),
            self.tx(),
            self.ty(),
        )
    }
}

pub(crate) trait UsvgTransformExt {
    fn to_krilla(&self) -> krilla::geom::Transform;
}

impl UsvgTransformExt for Transform {
    fn to_krilla(&self) -> krilla::geom::Transform {
        krilla::geom::Transform::from_row(self.sx, self.ky, self.kx, self.sy, self.tx, self.ty)
    }
}

pub(crate) trait PathExt {
    fn to_krilla(&self) -> Path;
}

impl PathExt for crate::svg::usvg::Path {
    fn to_krilla(&self) -> Path {
        self.data().to_krilla()
    }
}

impl PathExt for tiny_skia_path::Path {
    fn to_krilla(&self) -> Path {
        let mut pb = PathBuilder::new();

        for seg in self.segments() {
            match seg {
                PathSegment::MoveTo(p) => pb.move_to(p.x, p.y),
                PathSegment::LineTo(p) => pb.line_to(p.x, p.y),
                PathSegment::QuadTo(p1, p2) => pb.quad_to(p1.x, p1.y, p2.x, p2.y),
                PathSegment::CubicTo(p1, p2, p3) => pb.cubic_to(p1.x, p1.y, p2.x, p2.y, p3.x, p3.y),
                PathSegment::Close => pb.close(),
            }
        }

        pb.finish().unwrap()
    }
}
