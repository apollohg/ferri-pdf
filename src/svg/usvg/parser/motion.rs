//! SVG equivalent shapes reused by CSS Motion Path references.
use super::{converter, shapes, svgtree};
use crate::paint::{Contour, Segment};
use crate::style::transform::Matrix3d;
use crate::svg::usvg::{NonZeroRect, Options};
use std::sync::Arc;

pub(crate) fn motion_path(
    xml: &str,
    id: &str,
    viewport: [f32; 2],
    options: &Options,
    font_provider: Option<&dyn crate::svg::usvg::FontProvider>,
) -> Option<Vec<Contour>> {
    let xml = roxmltree::Document::parse_with_options(xml, super::xml_options()).ok()?;
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

/// How an element's transform places its content in its parent's user space (CSS Transforms 2
/// §4).
pub(crate) enum Placement {
    Affine(crate::svg::usvg::Transform),
    /// A 3D matrix, column-major, in user units: the element
    /// projects through a perspective, or is a plane of a 3D rendering context.
    Matrix(Matrix3d),
    /// The element lies behind the viewer or turns a hidden back face to them (§4.2, §10).
    Hidden,
}

/// Elements whose content is flattened before it applies, which overrides `preserve-3d` in
/// their descendants (§11).
const FLATTENING_ELEMENTS: [svgtree::EId; 3] = [
    svgtree::EId::ClipPath,
    svgtree::EId::Mask,
    svgtree::EId::Pattern,
];
const PRESERVE_3D: &str = "preserve-3d";
const NONE: &str = "none";
const NORMAL: &str = "normal";
const ISOLATE: &str = "isolate";
const HIDDEN: &str = "hidden";
/// `overflow` values that do not group the element's content (§7.1).
const UNGROUPED_OVERFLOW: [&str; 2] = ["visible", "clip"];
/// Elements whose `overflow` clips the viewport they establish.
const VIEWPORT_ELEMENTS: [svgtree::EId; 3] = [
    svgtree::EId::Svg,
    svgtree::EId::Symbol,
    svgtree::EId::Marker,
];

pub(super) fn transform(
    node: svgtree::SvgNode<'_, '_>,
    state: &converter::State<'_>,
    cache: &mut converter::Cache,
    parent: &crate::svg::usvg::Group,
    collect: &dyn Fn(&mut converter::Cache, &mut crate::svg::usvg::Group),
) -> Option<Placement> {
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
    let plane = preserves_3d(node, state) || participates(node, state);
    if node.is_contents() || !(css_transform || css_properties || plane) {
        return None;
    }
    match transform_impl(node, state, cache, parent, collect, plane) {
        Ok(transform) => transform,
        Err(error) => {
            cache.style_error.get_or_insert(Arc::new(error));
            Some(Placement::Affine(Default::default()))
        }
    }
}

/// Whether `node` participates in a 3D rendering context: its parent, which for the instance
/// a `use` element references is that element, establishes or extends one (§4.1.2).
pub(super) fn participates(node: svgtree::SvgNode<'_, '_>, state: &converter::State<'_>) -> bool {
    node.parent_element()
        .is_some_and(|parent| preserves_3d(parent, state))
}

/// Whether the used `transform-style` of `node` is `preserve-3d` (§7, §7.1, §11).
pub(super) fn preserves_3d(node: svgtree::SvgNode<'_, '_>, state: &converter::State<'_>) -> bool {
    use svgtree::AId;
    if node.attribute::<&str>(AId::TransformStyle) != Some(PRESERVE_3D)
        || state.parent_clip_path.is_some()
    {
        return false;
    }
    let grouped = node
        .attribute::<crate::svg::usvg::Opacity>(AId::Opacity)
        .is_some_and(|opacity| opacity.get() < 1.0)
        || [AId::Filter, AId::ClipPath, AId::Mask]
            .iter()
            .any(|&attribute| node.attribute::<&str>(attribute).is_some_and(|v| v != NONE))
        || node.attribute::<&str>(AId::Isolation) == Some(ISOLATE)
        || node
            .attribute::<&str>(AId::MixBlendMode)
            .is_some_and(|mode| mode != NORMAL)
        // Nested viewports clip their overflow unless it is visible.
        || (node
            .tag_name()
            .is_some_and(|tag| VIEWPORT_ELEMENTS.contains(&tag))
            && node.parent_element().is_some()
            && node.attribute::<&str>(AId::Overflow).is_none_or(|overflow| {
                overflow
                    .split_ascii_whitespace()
                    .any(|keyword| !UNGROUPED_OVERFLOW.contains(&keyword))
            }));
    !grouped
        && !node.ancestors().any(|ancestor| {
            ancestor
                .tag_name()
                .is_some_and(|tag| FLATTENING_ELEMENTS.contains(&tag))
        })
}

/// The CSS transform list `source` of the `clipPath` `node`, `None` where it does not parse.
/// §11: a clipping path ignores a list with three-dimensional transform functions.
pub(super) fn clip_path_transform(
    node: svgtree::SvgNode<'_, '_>,
    state: &converter::State<'_>,
    source: &str,
) -> Option<crate::svg::usvg::Transform> {
    use crate::paint::POINTS_PER_PIXEL;
    let style = super::units::with_relative(node, state, |relative| {
        crate::style::motion::svg_style([("transform", source)], relative)
    })
    .ok()?;
    Some(style.planar_transform().map_or_else(
        crate::svg::usvg::Transform::default,
        |[a, b, c, d, e, f]| {
            crate::svg::usvg::Transform::from_row(
                a,
                b,
                c,
                d,
                e / POINTS_PER_PIXEL,
                f / POINTS_PER_PIXEL,
            )
        },
    ))
}

/// Whether `node` hides its back face (§10).
pub(super) fn backface_hidden(node: svgtree::SvgNode<'_, '_>) -> bool {
    node.attribute::<&str>(svgtree::AId::BackfaceVisibility) == Some(HIDDEN)
}

/// The perspective `node` gives its transformed children, in points (§4.1.1, §8, §9).
pub(super) fn perspective(
    node: svgtree::SvgNode<'_, '_>,
    state: &converter::State<'_>,
    cache: &mut converter::Cache,
    parent: &crate::svg::usvg::Group,
    collect: &dyn Fn(&mut converter::Cache, &mut crate::svg::usvg::Group),
) -> Option<Matrix3d> {
    use svgtree::AId;
    let depth = node
        .attribute::<&str>(AId::Perspective)
        .filter(|&depth| depth != NONE)?;
    let mut values = vec![("perspective", depth)];
    for (attribute, property) in [
        (AId::PerspectiveOrigin, "perspective-origin"),
        (AId::TransformBox, "transform-box"),
    ] {
        if let Some(source) = node.attribute(attribute) {
            values.push((property, source));
        }
    }
    let styled = super::units::with_relative(node, state, |relative| {
        crate::style::motion::svg_style(values, relative)
    });
    let mut style = match styled {
        Ok(style) => style,
        Err(error) => {
            cache.style_error.get_or_insert(Arc::new(error));
            return None;
        }
    };
    style.effective_zoom = node.effective_zoom();
    let mut style = style.used();
    let boxes = reference_boxes(&mut style, false, state, cache, parent, collect);
    style.parent_3d(&boxes).map(|parent| parent.perspective)
}

/// The areas whose content box is the reference box `style.transform_box` selects, measuring
/// the element's content with `collect` when it or `measure` needs its bounds (CSS Transforms
/// 1 §3); `style.transform_box` then selects that content box.
fn reference_boxes(
    style: &mut crate::style::Style,
    measure: bool,
    state: &converter::State<'_>,
    cache: &mut converter::Cache,
    parent: &crate::svg::usvg::Group,
    collect: &dyn Fn(&mut converter::Cache, &mut crate::svg::usvg::Group),
) -> crate::paint::BoxAreas<crate::paint::Rect> {
    use crate::style::TransformBox;
    let view = points(state.view_box.to_rect());
    let (fill, stroke) = if measure || style.transform_box != TransformBox::ViewBox {
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
    crate::paint::BoxAreas {
        content: own,
        padding: stroke,
        border: stroke,
    }
}

/// `rect`, in user units, in points.
fn points(rect: crate::svg::usvg::Rect) -> crate::paint::Rect {
    use crate::paint::POINTS_PER_PIXEL;
    crate::paint::Rect {
        x: rect.x() * POINTS_PER_PIXEL,
        y: rect.y() * POINTS_PER_PIXEL,
        width: rect.width() * POINTS_PER_PIXEL,
        height: rect.height() * POINTS_PER_PIXEL,
    }
}

/// `m`, a 3D matrix of points, as one from and to user units in x and y.
fn user_units(m: &Matrix3d) -> Matrix3d {
    use crate::paint::POINTS_PER_PIXEL;
    use crate::style::transform::{multiply, scaling};
    let to_user = 1.0 / POINTS_PER_PIXEL;
    multiply(
        &scaling(to_user, to_user, to_user),
        &multiply(
            m,
            &scaling(POINTS_PER_PIXEL, POINTS_PER_PIXEL, POINTS_PER_PIXEL),
        ),
    )
}

fn transform_impl(
    node: svgtree::SvgNode<'_, '_>,
    state: &converter::State<'_>,
    cache: &mut converter::Cache,
    parent: &crate::svg::usvg::Group,
    collect: &dyn Fn(&mut converter::Cache, &mut crate::svg::usvg::Group),
    plane: bool,
) -> Result<Option<Placement>, crate::style::StyleError> {
    use crate::paint::POINTS_PER_PIXEL;
    use crate::style::motion::{Curve, OffsetPath, OffsetPosition, Reference};
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
        (AId::BackfaceVisibility, "backface-visibility"),
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
    let parent_3d = node
        .parent_element()
        .and_then(|parent| cache.perspectives.get(&parent.key()).copied())
        .map(|perspective| crate::style::transform::Parent3d { perspective });
    if !style.is_transformed() {
        // An untransformed plane lies where its parent's perspective keeps it.
        return Ok(plane.then(|| {
            Placement::Matrix(user_units(
                &parent_3d.map_or(crate::style::transform::IDENTITY, |parent| {
                    parent.perspective
                }),
            ))
        }));
    }
    let measure = matches!(style.motion.position, OffsetPosition::Auto)
        || matches!(
            &*style.motion.path,
            OffsetPath::Path {
                curve: Curve::Ray { contain: true, .. },
                ..
            }
        );
    let boxes = reference_boxes(&mut style, measure, state, cache, parent, collect);
    let mut reference = Reference::rect(points(state.view_box.to_rect()));
    reference.border_radius = Some(style.decoration.radii.clone());
    if let OffsetPath::Path {
        curve: Curve::Reference(url),
        ..
    } = &*style.motion.path
    {
        // Linking rewrote references into other documents to their appended elements.
        if let Some(id) = url.strip_prefix('#') {
            reference.path = percent_encoding::percent_decode_str(id)
                .decode_utf8()
                .ok()
                .and_then(|id| node.document().element_by_id(&id))
                .and_then(|node| shape_path(node, state))
                .map(Arc::new);
        }
    }
    let Some(m) = style.plane(&boxes, parent_3d.as_ref(), &reference) else {
        return Ok(None);
    };
    if plane {
        return Ok(Some(Placement::Matrix(user_units(&m))));
    }
    let projection = style.projected(&m, &boxes);
    if projection == crate::style::transform::NOT_RENDERED {
        return Ok(Some(Placement::Hidden));
    }
    Ok(Some(match crate::paint::projection::affine(projection) {
        // All geometry supplied to the shared transform is in points; SVG uses user units.
        Some([a, b, c, d, e, f]) => Placement::Affine(crate::svg::usvg::Transform::from_row(
            a,
            b,
            c,
            d,
            e / POINTS_PER_PIXEL,
            f / POINTS_PER_PIXEL,
        )),
        None => Placement::Matrix(user_units(&m)),
    }))
}
