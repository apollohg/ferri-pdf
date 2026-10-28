//! CSS Transforms 1/2 functions, composed in 3D and flattened to 2D affine matrices for print.
use crate::paint::Rect;
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
type Matrix3d = [f32; 16];
const IDENTITY: Matrix3d = [
    1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
];
const DIMENSION: usize = 4;
const X_AXIS: [f32; 3] = [1.0, 0.0, 0.0];
const Y_AXIS: [f32; 3] = [0.0, 1.0, 0.0];
const Z_AXIS: [f32; 3] = [0.0, 0.0, 1.0];
const START: LengthPercentage = LengthPercentage::Percentage(0.0);
const CENTRE: LengthPercentage = LengthPercentage::Percentage(0.5);
const END: LengthPercentage = LengthPercentage::Percentage(1.0);
pub(crate) const INITIAL_ORIGIN: [LengthPercentage; 3] = [CENTRE, CENTRE, LengthPercentage::ZERO];

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) enum TransformFunction {
    /// Affine 3D matrix; translation components are points.
    Matrix(Matrix3d),
    /// x and y may be percentages of the border box; z is always a length.
    Translate([LengthPercentage; 3]),
}

fn multiply(m: &Matrix3d, n: &Matrix3d) -> Matrix3d {
    std::array::from_fn(|index| {
        let (column, row) = (index / DIMENSION, index % DIMENSION);
        (0..DIMENSION)
            .map(|k| m[k * DIMENSION + row] * n[column * DIMENSION + k])
            .sum()
    })
}
fn translation([x, y, z]: [f32; 3]) -> Matrix3d {
    let mut m = IDENTITY;
    m[12..15].copy_from_slice(&[x, y, z]);
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
fn arguments(function: &Component) -> Vec<&Component> {
    fn collect<'a>(component: &'a Component, output: &mut Vec<&'a Component>) {
        match component {
            Component::Group(items) => items.iter().for_each(|item| collect(item, output)),
            Component::Reference(_, inner) => collect(inner, output),
            Component::Delim(_) => {}
            value => output.push(value),
        }
    }
    let mut output = Vec::new();
    function
        .arguments()
        .iter()
        .for_each(|item| collect(item, &mut output));
    output
}
/// `<number>`, or a `<percentage>` where scaling accepts one.
fn number(value: &Component, relative: &Relative<'_>) -> Result<f32, StyleError> {
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
            // PDF content matrices are affine; a perspective row cannot be drawn.
            if m[3] != 0.0 || m[7] != 0.0 || m[11] != 0.0 || m[15] <= 0.0 {
                return Err("projective matrix3d() is unsupported".into());
            }
            let w = m[15];
            m = m.map(|v| v / w);
            for translation in &mut m[12..15] {
                *translation *= POINTS_PER_PIXEL;
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
        Keyword::Perspective => {
            return Err("perspective() is unsupported: PDF matrices are affine".into());
        }
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

impl Style {
    /// Page-space matrix for this box's `border_box`, or `None` without a transform.
    pub(crate) fn transform_matrix(&self, border_box: Rect) -> Option<Matrix> {
        if self.transform.is_empty() {
            return None;
        }
        // z components are lengths, so their percentage basis is irrelevant.
        let resolve = |[x, y, z]: &[LengthPercentage; 3]| {
            [
                x.resolve(border_box.width),
                y.resolve(border_box.height),
                z.resolve(0.0),
            ]
        };
        let [x, y, z] = resolve(&self.transform_origin);
        let origin = [border_box.x + x, border_box.y + y, z];
        let mut result = translation(origin);
        for function in &self.transform {
            let step = match function {
                TransformFunction::Matrix(m) => *m,
                TransformFunction::Translate(offset) => translation(resolve(offset)),
            };
            result = multiply(&result, &step);
        }
        let m = multiply(&result, &translation(origin.map(|v| -v)));
        // Orthographic flattening (CSS Transforms 2 §6) keeps the x/y rows and columns.
        Some([m[0], m[1], m[4], m[5], m[12], m[13]])
    }
}
