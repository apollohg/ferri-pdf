//! Filter Effects 1 §6 filter functions and their §13.1 equivalent filter primitives.
use crate::paint::Color;
use serde::Serialize;

/// Row-major 4×5 `feColorMatrix` values on non-premultiplied sRGB channels in `0..=1`.
pub(crate) type ColorMatrix = [f32; 20];
const MATRIX_COLUMNS: usize = 5;
const CHANNELS: usize = 4;
const ALPHA: usize = 3;
const OFFSET: usize = 4;
pub(crate) const IDENTITY_MATRIX: ColorMatrix = [
    1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    0.0,
];
/// Luminance coefficients of the `saturate` and `hueRotate` matrices (Filter Effects 1 §9.6).
const LUMA: [f32; 3] = [0.213, 0.715, 0.072];
/// `hueRotate` cosine and sine weights, by output row and input column (Filter Effects 1 §9.6).
const HUE_COSINE: [[f32; 3]; 3] = [
    [0.787, -0.715, -0.072],
    [-0.213, 0.285, -0.072],
    [-0.213, -0.715, 0.928],
];
const HUE_SINE: [[f32; 3]; 3] = [
    [-0.213, -0.715, 0.928],
    [0.143, 0.140, -0.283],
    [-0.787, 0.715, 0.072],
];
/// `feGaussianBlur` kernels have negligible weight beyond three standard deviations, the
/// extent the box-blur approximation reaches (Filter Effects 1 §9.12).
pub(crate) const BLUR_EXTENT: f32 = 3.0;

/// One computed `<filter-function>` or `<url>`; lengths are points and angles degrees.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub enum FilterFunction {
    /// Standard deviation.
    Blur(f32),
    Brightness(f32),
    Contrast(f32),
    DropShadow {
        offset: [f32; 2],
        deviation: f32,
        color: Color,
    },
    Grayscale(f32),
    HueRotate(f32),
    Invert(f32),
    Opacity(f32),
    Saturate(f32),
    Sepia(f32),
    /// A reference to an SVG `filter` element, as written.
    Url(Box<str>),
}

impl FilterFunction {
    /// The §13.1 equivalent as one colour matrix, for every function but blur, drop shadows
    /// and references. Amounts above one clamp where §6.1 requires it.
    pub(crate) fn matrix(&self) -> Option<ColorMatrix> {
        Some(match *self {
            Self::Grayscale(amount) => {
                let keep = 1.0 - amount.min(1.0);
                rows([
                    [
                        0.2126 + 0.7874 * keep,
                        0.7152 - 0.7152 * keep,
                        0.0722 - 0.0722 * keep,
                    ],
                    [
                        0.2126 - 0.2126 * keep,
                        0.7152 + 0.2848 * keep,
                        0.0722 - 0.0722 * keep,
                    ],
                    [
                        0.2126 - 0.2126 * keep,
                        0.7152 - 0.7152 * keep,
                        0.0722 + 0.9278 * keep,
                    ],
                ])
            }
            Self::Sepia(amount) => {
                let keep = 1.0 - amount.min(1.0);
                rows([
                    [
                        0.393 + 0.607 * keep,
                        0.769 - 0.769 * keep,
                        0.189 - 0.189 * keep,
                    ],
                    [
                        0.349 - 0.349 * keep,
                        0.686 + 0.314 * keep,
                        0.168 - 0.168 * keep,
                    ],
                    [
                        0.272 - 0.272 * keep,
                        0.534 - 0.534 * keep,
                        0.131 + 0.869 * keep,
                    ],
                ])
            }
            Self::Saturate(amount) => rows(std::array::from_fn(|row| {
                std::array::from_fn(|column| {
                    LUMA[column] * (1.0 - amount) + if row == column { amount } else { 0.0 }
                })
            })),
            Self::HueRotate(degrees) => {
                let (sine, cosine) = degrees.to_radians().sin_cos();
                rows(std::array::from_fn(|row| {
                    std::array::from_fn(|column| {
                        LUMA[column]
                            + cosine * HUE_COSINE[row][column]
                            + sine * HUE_SINE[row][column]
                    })
                }))
            }
            Self::Invert(amount) => {
                let amount = amount.min(1.0);
                transfer(1.0 - 2.0 * amount, amount, 1.0)
            }
            Self::Opacity(amount) => transfer(1.0, 0.0, amount.min(1.0)),
            Self::Brightness(amount) => transfer(amount, 0.0, 1.0),
            Self::Contrast(amount) => transfer(amount, 0.5 - 0.5 * amount, 1.0),
            Self::Blur(_) | Self::DropShadow { .. } | Self::Url(_) => return None,
        })
    }
    /// How far the function moves ink, in points: §13.2 grows the filter region by it.
    pub(crate) fn spread(&self) -> f32 {
        match *self {
            Self::Blur(deviation) => BLUR_EXTENT * deviation,
            Self::DropShadow {
                offset: [x, y],
                deviation,
                ..
            } => x.abs().max(y.abs()) + BLUR_EXTENT * deviation,
            _ => 0.0,
        }
    }
}

/// The colour rows of a matrix that keeps alpha.
fn rows(color: [[f32; 3]; 3]) -> ColorMatrix {
    let mut matrix = IDENTITY_MATRIX;
    for (row, values) in color.iter().enumerate() {
        matrix[row * MATRIX_COLUMNS..row * MATRIX_COLUMNS + 3].copy_from_slice(values);
    }
    matrix
}

/// `feComponentTransfer` linear functions: colour `slope·c + intercept`, alpha `alpha·a`.
fn transfer(slope: f32, intercept: f32, alpha: f32) -> ColorMatrix {
    let mut matrix = [0.0; 20];
    for channel in 0..3 {
        matrix[channel * MATRIX_COLUMNS + channel] = slope;
        matrix[channel * MATRIX_COLUMNS + OFFSET] = intercept;
    }
    matrix[ALPHA * MATRIX_COLUMNS + ALPHA] = alpha;
    matrix
}

/// The constant alpha factor of a matrix whose colour rows ignore alpha and whose alpha row
/// only scales it, so that the colour part acts on each paint and the alpha part on the group.
pub(crate) fn alpha_scale(matrix: &ColorMatrix) -> Option<f32> {
    let colour_ignores_alpha = (0..ALPHA).all(|row| matrix[row * MATRIX_COLUMNS + ALPHA] == 0.0);
    let alpha_row = &matrix[ALPHA * MATRIX_COLUMNS..ALPHA * MATRIX_COLUMNS + MATRIX_COLUMNS];
    let scales_alpha = alpha_row
        .iter()
        .enumerate()
        .all(|(column, value)| column == ALPHA || *value == 0.0);
    (colour_ignores_alpha && scales_alpha).then_some(alpha_row[ALPHA])
}

/// `second` applied after `first`.
pub(crate) fn compose(first: &ColorMatrix, second: &ColorMatrix) -> ColorMatrix {
    std::array::from_fn(|index| {
        let (row, column) = (index / MATRIX_COLUMNS, index % MATRIX_COLUMNS);
        let linear: f32 = (0..CHANNELS)
            .map(|k| second[row * MATRIX_COLUMNS + k] * first[k * MATRIX_COLUMNS + column])
            .sum();
        if column == OFFSET {
            linear + second[row * MATRIX_COLUMNS + OFFSET]
        } else {
            linear
        }
    })
}

/// Applies `matrix` to non-premultiplied `rgba`, unclamped.
pub(crate) fn transform(matrix: &ColorMatrix, rgba: [f32; 4]) -> [f32; 4] {
    std::array::from_fn(|row| {
        (0..CHANNELS)
            .map(|k| matrix[row * MATRIX_COLUMNS + k] * rgba[k])
            .sum::<f32>()
            + matrix[row * MATRIX_COLUMNS + OFFSET]
    })
}

/// Whether `matrix` maps every channel combination in `0..=1` into `0..=1`, so that the
/// filter's clamping never applies and the matrix commutes with source-over compositing of
/// the colours it maps: each pixel of a group is a convex combination of its paints' colours.
/// An affine map's image of the unit hypercube is the hull of its corners' images.
pub(crate) fn preserves_unit_range(matrix: &ColorMatrix) -> bool {
    const TOLERANCE: f32 = 1.0e-6;
    (0..1_u8 << CHANNELS).all(|corner| {
        let rgba = std::array::from_fn(|channel| f32::from((corner >> channel) & 1));
        transform(matrix, rgba)
            .iter()
            .all(|value| (-TOLERANCE..=1.0 + TOLERANCE).contains(value))
    })
}
