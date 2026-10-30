//! SVG equivalent shapes reused by CSS Motion Path references.
use super::{converter, shapes, svgtree};
use crate::paint::{Contour, Segment};
use crate::svg::usvg::{NonZeroRect, Options};
use std::sync::Arc;

pub(crate) fn motion_path(
    xml: &str,
    id: &str,
    viewport: [f32; 2],
    options: &Options,
    font_provider: Option<&dyn crate::svg::usvg::FontProvider>,
) -> Option<Vec<Contour>> {
    let xml = roxmltree::Document::parse_with_options(
        xml,
        roxmltree::ParsingOptions {
            allow_dtd: true,
            ..Default::default()
        },
    )
    .ok()?;
    let document = super::ParsedSvg::parse(&xml, None).ok()?;
    let id = percent_encoding::percent_decode_str(id)
        .decode_utf8()
        .ok()?;
    let node = document.0.element_by_id(&id)?;
    let state = converter::State {
        unit_viewport: Some(viewport),
        font_provider,
        ..converter::State::new(
            options,
            NonZeroRect::from_xywh(
                0.0,
                0.0,
                options.default_size.width(),
                options.default_size.height(),
            )?,
        )
    };
    shape_path(node, &state)
}
fn shape_path(
    node: svgtree::SvgNode<'_, '_>,
    state: &converter::State<'_>,
) -> Option<Vec<Contour>> {
    if node.tag_name() == Some(svgtree::EId::Path) {
        if let Some(data) = node.attribute::<&str>(svgtree::AId::D)
            && let Some(contours) = crate::style::shape_values::path_data(
                data,
                crate::style::shape_values::PathErrors::Prefix,
            )
        {
            return Some(contours);
        }
    }
    let path = shapes::convert(node, state, shapes::Purpose::Geometry)?;
    if state.length_error.borrow().is_some() {
        return None;
    }
    let mut output = Vec::new();
    let mut current: Option<Contour> = None;
    let mut from = [0.0; 2];
    let point = |point: tiny_skia_path::Point| [point.x, point.y];
    for segment in path.segments() {
        use tiny_skia_path::PathSegment as S;
        match segment {
            S::MoveTo(to) => {
                output.extend(current.take());
                from = point(to);
                current = Some(Contour {
                    start: from,
                    segments: Vec::new(),
                    closed: false,
                });
            }
            S::Close => {
                if let Some(path) = &mut current {
                    path.closed = true;
                    from = path.start;
                }
                output.extend(current.take());
            }
            segment => {
                let path = current.as_mut()?;
                path.segments.push(match segment {
                    S::LineTo(to) => Segment::Line(point(to)),
                    S::CubicTo(a, b, to) => Segment::Cubic([point(a), point(b), point(to)]),
                    S::QuadTo(control, to) => Segment::quadratic(from, point(control), point(to)),
                    S::MoveTo(_) | S::Close => unreachable!(),
                });
                from = path.segments.last()?.end();
            }
        }
    }
    output.extend(current);
    Some(output)
}

pub(super) fn transform(
    node: svgtree::SvgNode<'_, '_>,
    state: &converter::State<'_>,
    cache: &mut converter::Cache,
    parent: &crate::svg::usvg::Group,
    collect: &dyn Fn(&mut converter::Cache, &mut crate::svg::usvg::Group),
) -> Option<crate::svg::usvg::Transform> {
    let css_transform = node
        .attribute::<&str>(svgtree::AId::Transform)
        .is_some_and(|value| value.parse::<svgtypes::Transform>().is_err());
    let css_properties = [
        svgtree::AId::OffsetPath,
        svgtree::AId::CssTranslate,
        svgtree::AId::CssRotate,
        svgtree::AId::CssScale,
    ]
    .iter()
    .any(|&attribute| node.has_attribute(attribute));
    if node.is_contents() || (!css_transform && !css_properties) {
        return None;
    }
    match transform_impl(node, state, cache, parent, collect) {
        Ok(transform) => transform,
        Err(error) => {
            cache.style_error.get_or_insert(Arc::new(error));
            Some(Default::default())
        }
    }
}
fn transform_impl(
    node: svgtree::SvgNode<'_, '_>,
    state: &converter::State<'_>,
    cache: &mut converter::Cache,
    parent: &crate::svg::usvg::Group,
    collect: &dyn Fn(&mut converter::Cache, &mut crate::svg::usvg::Group),
) -> Result<Option<crate::svg::usvg::Transform>, crate::style::StyleError> {
    use crate::paint::{BoxAreas, POINTS_PER_PIXEL, Rect};
    use crate::style::{
        TransformBox,
        motion::{Curve, OffsetPath, OffsetPosition, Reference},
    };
    use svgtree::AId;
    let zoom = node.effective_zoom();
    let mut values = Vec::new();
    let svg_transform = node
        .attribute::<&str>(AId::Transform)
        .and_then(|source| source.parse::<svgtypes::Transform>().ok())
        .map(|matrix| {
            format!(
                "matrix({},{},{},{},{},{})",
                matrix.a, matrix.b, matrix.c, matrix.d, matrix.e, matrix.f
            )
        });
    for (attribute, property) in [
        (AId::OffsetPath, "offset-path"),
        (AId::OffsetDistance, "offset-distance"),
        (AId::OffsetPosition, "offset-position"),
        (AId::OffsetAnchor, "offset-anchor"),
        (AId::OffsetRotate, "offset-rotate"),
        (AId::TransformBox, "transform-box"),
        (AId::TransformOrigin, "transform-origin"),
        (AId::CssTranslate, "translate"),
        (AId::CssRotate, "rotate"),
        (AId::CssScale, "scale"),
        (AId::Transform, "transform"),
    ] {
        let source = if attribute == AId::Transform {
            svg_transform
                .as_deref()
                .or_else(|| node.attribute(attribute))
        } else {
            node.attribute(attribute)
        };
        if let Some(source) = source {
            values.push((property, source));
        }
    }
    if let Some(owner) = node
        .ancestors()
        .find(|node| node.tag_name() == Some(svgtree::EId::Svg))
        && let Some(radius) = owner.attribute::<&str>(AId::CssBorderRadius)
    {
        values.push(("border-radius", radius));
    }
    let mut style = super::units::with_relative(node, state, |relative| {
        crate::style::motion::svg_style(values, relative)
    })?;
    style.effective_zoom = zoom;
    let mut style = style.used();
    if !style.is_transformed() {
        return Ok(None);
    }
    let points = |rect: crate::svg::usvg::Rect| Rect {
        x: rect.x() * POINTS_PER_PIXEL,
        y: rect.y() * POINTS_PER_PIXEL,
        width: rect.width() * POINTS_PER_PIXEL,
        height: rect.height() * POINTS_PER_PIXEL,
    };
    let view = points(state.view_box.to_rect());
    let needs_box = style.transform_box != TransformBox::ViewBox
        || matches!(style.motion.position, OffsetPosition::Auto)
        || matches!(
            &*style.motion.path,
            OffsetPath::Path {
                curve: Curve::Ray { contain: true, .. },
                ..
            }
        );
    let (fill, stroke) = if needs_box {
        let mut probe = crate::svg::usvg::Group::empty();
        probe.abs_transform = parent.abs_transform;
        collect(cache, &mut probe);
        probe.calculate_bounding_boxes();
        (
            points(probe.bounding_box),
            points(probe.stroke_bounding_box),
        )
    } else {
        (view, view)
    };
    let own = match style.transform_box {
        TransformBox::ViewBox => view,
        TransformBox::FillBox | TransformBox::ContentBox => fill,
        TransformBox::BorderBox | TransformBox::StrokeBox => stroke,
    };
    // The CSS-layout mapper aliases view-box to border-box; SVG supplies its selected box.
    style.transform_box = TransformBox::ContentBox;
    let boxes = BoxAreas {
        content: own,
        padding: stroke,
        border: stroke,
    };
    let mut reference = Reference::rect(view);
    reference.border_radius = Some(style.decoration.radii.clone());
    if let OffsetPath::Path {
        curve: Curve::Reference(url),
        ..
    } = &*style.motion.path
    {
        if let Some(id) = url.strip_prefix('#') {
            reference.path = percent_encoding::percent_decode_str(id)
                .decode_utf8()
                .ok()
                .and_then(|id| node.document().element_by_id(&id))
                .and_then(|node| shape_path(node, state))
                .map(Arc::new);
        } else if let Some((document, id)) = url.rsplit_once('#') {
            reference.path = state
                .motion_path_resolver
                .and_then(|resolve| resolve(document))
                .and_then(|xml| {
                    motion_path(
                        &xml,
                        id,
                        [state.view_box.width(), state.view_box.height()],
                        state.opt,
                        state.font_provider,
                    )
                })
                .map(Arc::new);
        }
    }
    let Some([a, b, c, d, e, f]) = style.transform_matrix(&boxes, None, &reference)? else {
        return Ok(None);
    };
    // All geometry supplied to the shared transform is in points; SVG uses user units.
    Ok(Some(crate::svg::usvg::Transform::from_row(
        a,
        b,
        c,
        d,
        e / POINTS_PER_PIXEL,
        f / POINTS_PER_PIXEL,
    )))
}
