//! CSS Transforms 1/2 functions and properties, composed in 3D and projected to 2D affine
//! matrices for print.
use crate::paint::{BoxAreas, Position, PositionComponent, Projection, Rect, projection::corners};
use crate::style::{
    LengthPercentage, POINTS_PER_PIXEL, Style, StyleError,
    box_values::{length_percentage, unsupported},
    calc::Kind,
    component::Component,
    grammar::Keyword,
    units::Relative,
    values::math,
};
use serde::Serialize;

/// Affine matrix `[a, b, c, d, e, f]`: `x' = a·x + c·y + e`, `y' = b·x + d·y + f`.
pub(crate) type Matrix = [f32; 6];
/// Column-major 4×4 matrix in `matrix3d()` argument order.
pub(crate) type Matrix3d = [f32; 16];
/// A box that projects behind the viewer or turns its hidden back face towards them is not
/// rendered: it collapses to a point, which paints nothing.
const NOT_RENDERED: Projection = [0.0; 9];
pub(crate) const IDENTITY: Matrix3d = [
    1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
];
const DIMENSION: usize = 4;
/// Column-major indices of the `w` row, and of the `z` row's term in `z`.
const W_FROM_X: usize = 3;
const W_FROM_Y: usize = 7;
const W_FROM_Z: usize = 11;
const W_CONSTANT: usize = 15;
const Z_FROM_Z: usize = 10;
const TRANSLATION: std::ops::Range<usize> = 12..15;

/// Transforms 2 §8: perspective distances below 1px render as 1px.
const MIN_PERSPECTIVE_PT: f32 = POINTS_PER_PIXEL;
const X_AXIS: [f32; 3] = [1.0, 0.0, 0.0];
const Y_AXIS: [f32; 3] = [0.0, 1.0, 0.0];
const Z_AXIS: [f32; 3] = [0.0, 0.0, 1.0];
const START: LengthPercentage = LengthPercentage::Percentage(0.0);
const CENTRE: LengthPercentage = LengthPercentage::Percentage(0.5);
const END: LengthPercentage = LengthPercentage::Percentage(1.0);
pub(crate) const INITIAL_ORIGIN: [LengthPercentage; 3] = [CENTRE, CENTRE, LengthPercentage::ZERO];
pub(crate) const INITIAL_PERSPECTIVE_ORIGIN: Position = [PositionComponent::CENTER; 2];

/// A `rotate` property value: an angle in radians about an origin-centred axis.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub(crate) struct Rotation {
    pub(crate) axis: [f32; 3],
    pub(crate) angle: f32,
}

/// `transform-box`; boxes with a CSS layout box use the content box for `fill-box` and the
/// border box for `stroke-box` and `view-box` (CSS Transforms 1 §3).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[expect(
    clippy::enum_variant_names,
    reason = "variants name the CSS transform-box keywords"
)]
pub(crate) enum TransformBox {
    ContentBox,
    BorderBox,
    FillBox,
    StrokeBox,
    #[default]
    ViewBox,
}
impl TransformBox {
    pub(super) fn from_component(value: &Component) -> Result<Self, StyleError> {
        Ok(match value.keyword() {
            Some(Keyword::ContentBox) => Self::ContentBox,
            Some(Keyword::BorderBox) => Self::BorderBox,
            Some(Keyword::FillBox) => Self::FillBox,
            Some(Keyword::StrokeBox) => Self::StrokeBox,
            Some(Keyword::ViewBox) => Self::ViewBox,
            _ => return Err(unsupported(value)),
        })
    }
    fn reference(self, boxes: &BoxAreas<Rect>) -> Rect {
        match self {
            Self::ContentBox | Self::FillBox => boxes.content,
            Self::BorderBox | Self::StrokeBox | Self::ViewBox => boxes.border,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) enum TransformFunction {
    /// Affine 3D matrix; translation components are points.
    Matrix(Matrix3d),
    /// x and y may be percentages of the border box; z is always a length.
    Translate([LengthPercentage; 3]),
}

impl TransformFunction {
    pub(super) fn zoom(&mut self, factor: f32) {
        match self {
            Self::Matrix(matrix) => {
                for value in &mut matrix[TRANSLATION] {
                    *value *= factor;
                }
                for index in [W_FROM_X, W_FROM_Y, W_FROM_Z] {
                    matrix[index] /= factor;
                }
            }
            Self::Translate(offsets) => {
                for value in offsets {
                    *value = value.zoomed(factor);
                }
            }
        }
    }

    pub(crate) fn css(
        &self,
        length: impl Fn(&LengthPercentage) -> Result<String, StyleError>,
    ) -> Result<String, StyleError> {
        Ok(match self {
            Self::Matrix(matrix) => {
                let mut matrix = *matrix;
                for value in &mut matrix[TRANSLATION] {
                    *value /= POINTS_PER_PIXEL;
                }
                for index in [W_FROM_X, W_FROM_Y, W_FROM_Z] {
                    matrix[index] *= POINTS_PER_PIXEL;
                }
                format!(
                    "matrix3d({})",
                    matrix.map(|value| value.to_string()).join(",")
                )
            }
            Self::Translate(offsets) => format!(
                "translate3d({},{},{})",
                length(&offsets[0])?,
                length(&offsets[1])?,
                length(&offsets[2])?
            ),
        })
    }
}

/// `m` after `n`.
pub(crate) fn multiply(m: &Matrix3d, n: &Matrix3d) -> Matrix3d {
    std::array::from_fn(|index| {
        let (column, row) = (index / DIMENSION, index % DIMENSION);
        (0..DIMENSION)
            .map(|k| m[k * DIMENSION + row] * n[column * DIMENSION + k])
            .sum()
    })
}
/// The 3D matrix of an affine map of the plane `[a, b, c, d, e, f]`, which leaves `z` alone.
fn lift([a, b, c, d, x, y]: Matrix) -> Matrix3d {
    multiply(&translation([x, y, 0.0]), &planar([a, b, c, d]))
}

/// `m` between affine maps of the plane: `inner` first and `outer` last.
pub(crate) fn conjugated(m: &Matrix3d, outer: Matrix, inner: Matrix) -> Matrix3d {
    multiply(&lift(outer), &multiply(m, &lift(inner)))
}

fn translation([x, y, z]: [f32; 3]) -> Matrix3d {
    let mut m = IDENTITY;
    m[TRANSLATION].copy_from_slice(&[x, y, z]);
    m
}
/// `perspective(d)`: depth `d` points in front of the drawing plane.
fn perspective(depth: f32) -> Matrix3d {
    let mut m = IDENTITY;
    m[W_FROM_Z] = -1.0 / depth.max(MIN_PERSPECTIVE_PT);
    m
}
/// Matrix whose linear part has these rows.
fn linear(rows: [[f32; 3]; 3]) -> Matrix3d {
    let mut m = IDENTITY;
    for (row, values) in rows.iter().enumerate() {
        for (column, &value) in values.iter().enumerate() {
            m[column * DIMENSION + row] = value;
        }
    }
    m
}
fn planar([a, b, c, d]: [f32; 4]) -> Matrix3d {
    linear([[a, c, 0.0], [b, d, 0.0], [0.0, 0.0, 1.0]])
}
fn scaling(x: f32, y: f32, z: f32) -> Matrix3d {
    linear([[x, 0.0, 0.0], [0.0, y, 0.0], [0.0, 0.0, z]])
}
/// Rotation about `axis` (CSS Transforms 2 `rotate3d()`); a zero axis is no rotation.
fn rotation(axis: [f32; 3], angle: f32) -> Matrix3d {
    let length = axis.iter().map(|v| v * v).sum::<f32>().sqrt();
    if length == 0.0 {
        return IDENTITY;
    }
    let [x, y, z] = axis.map(|v| v / length);
    let (sin, cos) = angle.sin_cos();
    let t = 1.0 - cos;
    linear([
        [t * x * x + cos, t * x * y - sin * z, t * x * z + sin * y],
        [t * x * y + sin * z, t * y * y + cos, t * y * z - sin * x],
        [t * x * z - sin * y, t * y * z + sin * x, t * z * z + cos],
    ])
}

/// A function's argument values in order, without the commas the grammar requires.
pub(super) fn arguments(function: &Component) -> Vec<&Component> {
    let mut output = Vec::new();
    function
        .arguments()
        .iter()
        .for_each(|item| collect_values(item, &mut output));
    output
}
fn collect_values<'a>(component: &'a Component, output: &mut Vec<&'a Component>) {
    match component {
        Component::Group(items) => items.iter().for_each(|item| collect_values(item, output)),
        Component::Slots(slots) => slots
            .iter()
            .flatten()
            .for_each(|slot| collect_values(slot, output)),
        Component::Reference(_, inner) => collect_values(inner, output),
        Component::Delim(_) => {}
        value => output.push(value),
    }
}
/// A property value's components in order, without grouping.
pub(super) fn values(value: &Component) -> Vec<&Component> {
    let mut output = Vec::new();
    collect_values(value, &mut output);
    output
}
/// `<number>`, or a `<percentage>` as its fraction.
pub(super) fn number(value: &Component, relative: &Relative<'_>) -> Result<f32, StyleError> {
    match value.unwrap() {
        Component::Percentage(fraction) => Ok(*fraction),
        Component::Math(source, Kind::Percentage, range) => {
            Ok(
                math(source, relative, Kind::Percentage, Some(Kind::Percentage))?
                    .resolve(0.0, *range),
            )
        }
        other => crate::style::values::number(other, relative),
    }
}
/// `<angle> | <zero>` in radians.
fn angle(value: &Component, relative: &Relative<'_>) -> Result<f32, StyleError> {
    crate::style::values::angle(value, relative).map(f32::to_radians)
}

fn function(value: &Component, relative: &Relative<'_>) -> Result<TransformFunction, StyleError> {
    let Component::Function(name, _) = value.unwrap() else {
        return Err(unsupported(value));
    };
    let args = arguments(value);
    let numbers = || {
        args.iter()
            .map(|arg| number(arg, relative))
            .collect::<Result<Vec<_>, _>>()
    };
    Ok(TransformFunction::Matrix(match name {
        Keyword::Matrix => {
            let m = numbers()?;
            let mut matrix = planar([m[0], m[1], m[2], m[3]]);
            // Unitless translations are CSS pixels.
            matrix[12] = m[4] * POINTS_PER_PIXEL;
            matrix[13] = m[5] * POINTS_PER_PIXEL;
            matrix
        }
        Keyword::Matrix3d => {
            let mut m: Matrix3d = numbers()?.try_into().map_err(|_| unsupported(value))?;
            // Unitless translations are CSS pixels; the `w` row divides them back out.
            for translation in &mut m[TRANSLATION] {
                *translation *= POINTS_PER_PIXEL;
            }
            for w in [W_FROM_X, W_FROM_Y, W_FROM_Z] {
                m[w] /= POINTS_PER_PIXEL;
            }
            m
        }
        Keyword::Translate | Keyword::Translate3d => {
            let x = length_percentage(args[0], relative)?;
            let y = args
                .get(1)
                .map(|arg| length_percentage(arg, relative))
                .transpose()?
                .unwrap_or(LengthPercentage::ZERO);
            let z = args
                .get(2)
                .map(|arg| length_percentage(arg, relative))
                .transpose()?
                .unwrap_or(LengthPercentage::ZERO);
            return Ok(TransformFunction::Translate([x, y, z]));
        }
        Keyword::Translatex | Keyword::Translatey | Keyword::Translatez => {
            let mut offset = [LengthPercentage::ZERO; 3];
            let axis = match name {
                Keyword::Translatex => 0,
                Keyword::Translatey => 1,
                _ => 2,
            };
            offset[axis] = length_percentage(args[0], relative)?;
            return Ok(TransformFunction::Translate(offset));
        }
        Keyword::Scale => {
            let s = numbers()?;
            scaling(s[0], *s.get(1).unwrap_or(&s[0]), 1.0)
        }
        Keyword::Scale3d => {
            let s = numbers()?;
            scaling(s[0], s[1], s[2])
        }
        Keyword::Scalex => scaling(number(args[0], relative)?, 1.0, 1.0),
        Keyword::Scaley => scaling(1.0, number(args[0], relative)?, 1.0),
        Keyword::Scalez => scaling(1.0, 1.0, number(args[0], relative)?),
        Keyword::Rotate | Keyword::Rotatez => rotation(Z_AXIS, angle(args[0], relative)?),
        Keyword::Rotatex => rotation(X_AXIS, angle(args[0], relative)?),
        Keyword::Rotatey => rotation(Y_AXIS, angle(args[0], relative)?),
        Keyword::Rotate3d => {
            let axis = [
                number(args[0], relative)?,
                number(args[1], relative)?,
                number(args[2], relative)?,
            ];
            rotation(axis, angle(args[3], relative)?)
        }
        Keyword::Skew => {
            let x = angle(args[0], relative)?.tan();
            let y = args
                .get(1)
                .map(|arg| angle(arg, relative))
                .transpose()?
                .unwrap_or(0.0)
                .tan();
            planar([1.0, y, x, 1.0])
        }
        Keyword::Skewx => planar([1.0, 0.0, angle(args[0], relative)?.tan(), 1.0]),
        Keyword::Skewy => planar([1.0, angle(args[0], relative)?.tan(), 0.0, 1.0]),
        Keyword::Perspective => match args.first() {
            Some(depth) if depth.keyword() != Some(Keyword::None) => {
                perspective(crate::style::values::length(depth, relative)?)
            }
            _ => IDENTITY,
        },
        _ => return Err(unsupported(value)),
    }))
}

/// `none | <transform-function>+`; `none` is empty.
pub(crate) fn transform(
    value: &Component,
    relative: &Relative<'_>,
) -> Result<Vec<TransformFunction>, StyleError> {
    if value.keyword() == Some(Keyword::None) {
        return Ok(Vec::new());
    }
    let mut functions = Vec::new();
    for item in value.items() {
        let parsed = function(item, relative)?;
        if let TransformFunction::Matrix(m) = &parsed
            && m.iter().any(|v| !v.is_finite())
        {
            return Err(format!("non-finite transform: {value}").into());
        }
        functions.push(parsed);
    }
    Ok(functions)
}

/// `transform-origin`: one position, an ordered pair, or keywords in either order, then z.
pub(crate) fn origin(
    value: &Component,
    relative: &Relative<'_>,
) -> Result<[LengthPercentage; 3], StyleError> {
    let horizontal = |part: &Component| match part.keyword() {
        Some(Keyword::Left) => Ok(START),
        Some(Keyword::Center) => Ok(CENTRE),
        Some(Keyword::Right) => Ok(END),
        _ => length_percentage(part, relative),
    };
    let vertical = |part: &Component| match part.keyword() {
        Some(Keyword::Top) => Ok(START),
        Some(Keyword::Center) => Ok(CENTRE),
        Some(Keyword::Bottom) => Ok(END),
        _ => length_percentage(part, relative),
    };
    let Component::Group(parts) = value.unwrap() else {
        return Ok(match value.keyword() {
            Some(Keyword::Top | Keyword::Bottom) => {
                [CENTRE, vertical(value)?, LengthPercentage::ZERO]
            }
            _ => [horizontal(value)?, CENTRE, LengthPercentage::ZERO],
        });
    };
    let (x, y, depth) = match parts.as_slice() {
        [Component::Slots(slots), depth] => (
            slots[0].as_ref().ok_or_else(|| unsupported(value))?,
            slots[1].as_ref().ok_or_else(|| unsupported(value))?,
            depth,
        ),
        [x, y, depth] => (x, y, depth),
        _ => return Err(unsupported(value)),
    };
    let z = depth
        .items()
        .first()
        .map(|z| length_percentage(z, relative))
        .transpose()?
        .unwrap_or(LengthPercentage::ZERO);
    Ok([horizontal(x)?, vertical(y)?, z])
}

/// `translate`: `none`, or x, y and z offsets where omitted ones are zero.
pub(crate) fn translate(
    value: &Component,
    relative: &Relative<'_>,
) -> Result<Option<[LengthPercentage; 3]>, StyleError> {
    if value.keyword() == Some(Keyword::None) {
        return Ok(None);
    }
    let parts = values(value);
    let offset = |index: usize| {
        parts.get(index).map_or(Ok(LengthPercentage::ZERO), |part| {
            length_percentage(part, relative)
        })
    };
    Ok(Some([offset(0)?, offset(1)?, offset(2)?]))
}

/// `rotate`: `none`, or an angle about the z axis, a named axis or an axis vector.
pub(crate) fn rotate(
    value: &Component,
    relative: &Relative<'_>,
) -> Result<Option<Rotation>, StyleError> {
    if value.keyword() == Some(Keyword::None) {
        return Ok(None);
    }
    let mut axis = Z_AXIS;
    let mut vector = Vec::new();
    let mut rotation = None;
    for part in values(value) {
        match (part.keyword(), part.unwrap()) {
            (Some(Keyword::X), _) => axis = X_AXIS,
            (Some(Keyword::Y), _) => axis = Y_AXIS,
            (Some(Keyword::Z), _) => axis = Z_AXIS,
            (_, Component::Angle(_) | Component::Math(_, Kind::Angle, _)) => {
                rotation = Some(angle(part, relative)?);
            }
            _ => vector.push(crate::style::values::number(part, relative)?),
        }
    }
    if let Ok(components) = <[f32; 3]>::try_from(vector.as_slice()) {
        axis = components;
    }
    Ok(Some(Rotation {
        axis,
        angle: rotation.ok_or_else(|| unsupported(value))?,
    }))
}

/// `scale`: `none`, or x, y and z factors where y defaults to x and z to one.
pub(crate) fn scale(
    value: &Component,
    relative: &Relative<'_>,
) -> Result<Option<[f32; 3]>, StyleError> {
    if value.keyword() == Some(Keyword::None) {
        return Ok(None);
    }
    let factors = values(value)
        .into_iter()
        .map(|part| number(part, relative))
        .collect::<Result<Vec<_>, _>>()?;
    let x = *factors.first().ok_or_else(|| unsupported(value))?;
    Ok(Some([
        x,
        factors.get(1).copied().unwrap_or(x),
        factors.get(2).copied().unwrap_or(1.0),
    ]))
}

/// `perspective`: `none`, or the distance in points from the viewer to the drawing plane.
pub(crate) fn perspective_distance(
    value: &Component,
    relative: &Relative<'_>,
) -> Result<Option<f32>, StyleError> {
    if value.keyword() == Some(Keyword::None) {
        return Ok(None);
    }
    crate::style::values::length(value, relative).map(Some)
}

/// `transform-style`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub(crate) enum TransformStyle {
    #[default]
    Flat,
    Preserve3d,
}
impl TransformStyle {
    pub(super) fn from_component(value: &Component) -> Result<Self, StyleError> {
        match value.keyword() {
            Some(Keyword::Flat) => Ok(Self::Flat),
            Some(Keyword::Preserve3d) => Ok(Self::Preserve3d),
            _ => Err(unsupported(value)),
        }
    }
}

/// The perspective a transformed box's parent places it under (CSS Transforms 2 §4.1.4).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Parent3d {
    /// The parent's perspective matrix in page space.
    pub(crate) perspective: Matrix3d,
}

impl Parent3d {
    pub(crate) fn mapped(self, affine: Matrix) -> Self {
        let inverse = kurbo::Affine::new(affine.map(f64::from))
            .inverse()
            .as_coeffs()
            .map(|value| value as f32);
        Self {
            perspective: conjugated(&self.perspective, affine, inverse),
        }
    }
}

/// CSS Transforms 2 §4.2: the page-space matrix applied to points `(x, y, 0, 1)` of `reference`,
/// as a projection of the plane. A box with `w ≤ 0` at every corner is behind the viewer and
/// a hidden back face (§10) is not rendered. A `w` constant across the box divides out into
/// an affine map.
fn project(m: &Matrix3d, reference: Rect, backface_hidden: bool) -> Projection {
    if backface_hidden && m[Z_FROM_Z] < 0.0 {
        return NOT_RENDERED;
    }
    let projection = crate::paint::projection::of_plane(*m);
    let corners = corners(reference);
    if corners
        .iter()
        .all(|&corner| crate::paint::projection::weight(projection, corner) <= 0.0)
    {
        return NOT_RENDERED;
    }
    crate::paint::projection::affine_over(projection, &corners)
        .map_or(projection, crate::paint::projection::from_affine)
}

impl Style {
    /// `transform`, `translate`, `rotate` or `scale` other than `none`.
    pub(crate) fn is_transformed(&self) -> bool {
        !self.transform.is_empty()
            || self.translate.is_some()
            || self.rotate.is_some()
            || self.scale.is_some()
            || self.motion.active()
    }
    /// Transforms, perspective and `preserve-3d` establish a stacking context and a containing
    /// block for all descendants (CSS Transforms 2 §5, §7, §8).
    pub(crate) fn has_transform_effects(&self) -> bool {
        self.is_transformed()
            || self.perspective.is_some()
            || self.transform_style == TransformStyle::Preserve3d
    }
    /// The used `transform-style`: grouping property values force `flat` (§7.1).
    pub(crate) fn preserves_3d(&self) -> bool {
        self.transform_style == TransformStyle::Preserve3d
            && !self.is_scroll_container()
            && !self.is_grouped()
            && self.clip.is_none()
    }
    /// The perspective this box gives its transformed children, if any.
    pub(crate) fn parent_3d(&self, boxes: &BoxAreas<Rect>) -> Option<Parent3d> {
        self.perspective.map(|depth| {
            let reference = self.transform_box.reference(boxes);
            let [x, y] = &self.perspective_origin;
            let origin = [
                reference.x + x.resolve(reference.width),
                reference.y + y.resolve(reference.height),
                0.0,
            ];
            Parent3d {
                perspective: multiply(
                    &multiply(&translation(origin), &perspective(depth)),
                    &translation(origin.map(|v| -v)),
                ),
            }
        })
    }
    /// CSS Transforms 2 §6: the page-space transformation matrix of a box with these areas, or
    /// `None` without a transform.
    fn transformation(
        &self,
        boxes: &BoxAreas<Rect>,
        containing: &crate::style::motion::Reference,
    ) -> Option<Matrix3d> {
        if !self.is_transformed() {
            return None;
        }
        let reference = self.transform_box.reference(boxes);
        // z components are lengths, so their percentage basis is irrelevant.
        let resolve = |[x, y, z]: &[LengthPercentage; 3]| {
            [
                x.resolve(reference.width),
                y.resolve(reference.height),
                z.resolve(0.0),
            ]
        };
        let [x, y, z] = resolve(&self.transform_origin);
        let origin = [reference.x + x, reference.y + y, z];
        let motion = self
            .motion
            .placement(boxes.border, containing)
            .map(|(point, angle)| {
                let anchor = self
                    .motion
                    .anchor
                    .as_ref()
                    .map_or([origin[0], origin[1]], |anchor| {
                        [
                            reference.x + anchor[0].resolve(reference.width),
                            reference.y + anchor[1].resolve(reference.height),
                        ]
                    });
                multiply(
                    &multiply(
                        &translation([point[0] - origin[0], point[1] - origin[1], 0.0]),
                        &rotation(Z_AXIS, angle),
                    ),
                    &translation([origin[0] - anchor[0], origin[1] - anchor[1], 0.0]),
                )
            });
        let Ok(composed) = self.composed(
            |offset| Ok::<_, std::convert::Infallible>(resolve(offset)),
            motion,
        );
        Some(multiply(
            &multiply(&translation(origin), &composed),
            &translation(origin.map(|v| -v)),
        ))
    }
    /// `translate`, `rotate`, `scale` and `transform` composed in that order (§6), with
    /// `resolve` placing translations.
    fn composed<E>(
        &self,
        resolve: impl Fn(&[LengthPercentage; 3]) -> Result<[f32; 3], E>,
        motion: Option<Matrix3d>,
    ) -> Result<Matrix3d, E> {
        let mut result = IDENTITY;
        if let Some(offset) = &self.translate {
            result = multiply(&result, &translation(resolve(offset)?));
        }
        if let Some(Rotation { axis, angle }) = self.rotate {
            result = multiply(&result, &rotation(axis, angle));
        }
        if let Some([x, y, z]) = self.scale {
            result = multiply(&result, &scaling(x, y, z));
        }
        if let Some(motion) = motion {
            result = multiply(&result, &motion);
        }
        for function in &self.transform {
            let step = match function {
                TransformFunction::Matrix(m) => *m,
                TransformFunction::Translate(offset) => translation(resolve(offset)?),
            };
            result = multiply(&result, &step);
        }
        Ok(result)
    }
    /// The planar matrix of the transform properties without the origin, in points, for SVG
    /// content, whose renderer places the origin in its reference box. `None` without a
    /// transform, and for a translation by a percentage of the reference box or a perspective,
    /// which the renderer evaluates in that box. SVG content is flattened into its viewport
    /// plane (CSS Transforms 2 §4).
    pub(crate) fn planar_transform(&self) -> Option<Matrix> {
        if !self.is_transformed() {
            return None;
        }
        let length = |component: &LengthPercentage| match component {
            LengthPercentage::Length(length) => Some(*length),
            _ => None,
        };
        let m = self
            .composed(
                |[x, y, z]| {
                    Ok::<_, ()>([
                        length(x).ok_or(())?,
                        length(y).ok_or(())?,
                        length(z).ok_or(())?,
                    ])
                },
                None,
            )
            .ok()?;
        (m[W_FROM_X] == 0.0 && m[W_FROM_Y] == 0.0 && m[W_CONSTANT] == 1.0)
            .then_some([m[0], m[1], m[4], m[5], m[12], m[13]])
    }
    /// The 3D matrix placing this box in its parent's space (CSS Transforms 2 §4.1.4): its
    /// transformation matrix after the parent's perspective, or `None` without a transform.
    pub(crate) fn plane(
        &self,
        boxes: &BoxAreas<Rect>,
        parent: Option<&Parent3d>,
        containing: &crate::style::motion::Reference,
    ) -> Option<Matrix3d> {
        let own = self.transformation(boxes, containing)?;
        Some(match parent {
            Some(parent) => multiply(&parent.perspective, &own),
            None => own,
        })
    }
    /// The projection drawing this box's plane, which accumulated matrix `m` places (§4.2).
    pub(crate) fn projected(&self, m: &Matrix3d, boxes: &BoxAreas<Rect>) -> Projection {
        project(m, self.transform_box.reference(boxes), self.backface_hidden)
    }
    /// The page-space projection drawing this box in its parent's plane, or `None` without a
    /// transform.
    pub(crate) fn transform_matrix(
        &self,
        boxes: &BoxAreas<Rect>,
        parent: Option<&Parent3d>,
        containing: &crate::style::motion::Reference,
    ) -> Option<Projection> {
        self.plane(boxes, parent, containing)
            .map(|m| self.projected(&m, boxes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mapped_parent_perspective_preserves_depth_and_off_center_projection() {
        const DEPTH: f32 = 100.0;
        const POINT: [f32; 3] = [11.0, 17.0, 25.0];
        const ORIGIN: [f32; 3] = [7.0, 13.0, 0.0];
        const TOLERANCE: f32 = 1e-4;
        let parent = Parent3d {
            perspective: multiply(
                &multiply(&translation(ORIGIN), &perspective(DEPTH)),
                &translation(ORIGIN.map(|value| -value)),
            ),
        };
        for affine @ [a, b, c, d, x, y] in [
            [0.0, 1.0, -1.0, 0.0, 200.0, 30.0],
            [0.0, 1.0, 1.0, 0.0, 20.0, 30.0],
            [0.0, -1.0, 1.0, 0.0, 20.0, 200.0],
        ] {
            let matrix = parent.mapped(affine).perspective;
            let [px, py, pz] = POINT;
            let point = [a * px + c * py + x, b * px + d * py + y, pz, 1.0];
            let homogeneous: [f32; 4] = std::array::from_fn(|row| {
                (0..DIMENSION)
                    .map(|column| matrix[column * DIMENSION + row] * point[column])
                    .sum()
            });
            let w = 1.0 - pz / DEPTH;
            let original = [
                (px - ORIGIN[0] * pz / DEPTH) / w,
                (py - ORIGIN[1] * pz / DEPTH) / w,
                pz / w,
            ];
            let expected = [
                a * original[0] + c * original[1] + x,
                b * original[0] + d * original[1] + y,
                original[2],
            ];
            for axis in 0..3 {
                let actual = homogeneous[axis] / homogeneous[3];
                assert!(
                    (actual - expected[axis]).abs() < TOLERANCE,
                    "{affine:?}, axis {axis}: mapped projection {actual}, expected {}",
                    expected[axis]
                );
            }
        }
    }
}
