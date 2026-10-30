use crate::svg::usvg::{Fill, PaintOrder, Stroke};
use krilla::surface::Surface;

use crate::svg::krilla_svg::ProcessContext;
use crate::svg::krilla_svg::util::{PathExt, UsvgTransformExt, convert_fill, convert_stroke};

/// Render a path into a surface.
pub(crate) fn render(
    path: &crate::svg::usvg::Path,
    surface: &mut Surface,
    process_context: &mut ProcessContext,
) {
    if !path.is_visible() {
        return;
    }

    match path.paint_order() {
        PaintOrder::FillAndStroke => {
            draw_path(path, path.fill(), path.stroke(), surface, process_context);
        }
        PaintOrder::StrokeAndFill => {
            draw_path(path, None, path.stroke(), surface, process_context);
            draw_path(path, path.fill(), None, surface, process_context);
        }
    }
}

/// Render a filled and/or stroked path into a surface.
pub(crate) fn draw_path(
    path: &crate::svg::usvg::Path,
    fill: Option<&Fill>,
    stroke: Option<&Stroke>,
    surface: &mut Surface,
    process_context: &mut ProcessContext,
) {
    let fill = fill.map(|f| {
        convert_fill(
            f,
            surface.stream_builder(),
            process_context,
            crate::svg::usvg::Transform::identity(),
        )
    });

    if path.non_scaling_stroke()
        && let Some(stroke) = stroke
    {
        if let Some(fill) = fill {
            surface.set_fill(Some(fill));
            surface.set_stroke(None);
            surface.draw_path(&path.to_krilla());
        }
        draw_host_stroke(path, stroke, surface, process_context);
        return;
    }

    let stroke = stroke.map(|s| {
        convert_stroke(
            s,
            surface.stream_builder(),
            process_context,
            crate::svg::usvg::Transform::identity(),
        )
    });

    // Otherwise krilla will fill with black by default.
    if fill.is_some() || stroke.is_some() {
        surface.set_fill(fill);
        surface.set_stroke(stroke);

        surface.draw_path(&path.to_krilla());
    }
}

/// SVG 2 §13.6 `non-scaling-stroke`: the outline is stroked in the host coordinate space, and
/// filled with the stroke paint as placed in the user coordinate system.
fn draw_host_stroke(
    path: &crate::svg::usvg::Path,
    stroke: &Stroke,
    surface: &mut Surface,
    process_context: &mut ProcessContext,
) {
    let user = path.abs_transform();
    let (Some(host), Some(to_host)) = (path.host_data(), user.invert()) else {
        return;
    };
    let stroke = convert_stroke(stroke, surface.stream_builder(), process_context, user);
    surface.push_transform(&to_host.to_krilla());
    surface.set_fill(None);
    surface.set_stroke(Some(stroke));
    surface.draw_path(&host.to_krilla());
    surface.pop();
}
