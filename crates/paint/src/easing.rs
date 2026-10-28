use serde::Serialize;

// Cover f32 subnormal inputs as well as f64 parameter precision.
const BEZIER_ITERATIONS: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct LinearPoint {
    pub input: f32,
    pub output: f32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum StepPosition {
    Start,
    End,
    None,
    Both,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub enum Easing {
    Linear,
    CubicBezier([f32; 4]),
    Steps { count: u32, position: StepPosition },
    Piecewise(Vec<LinearPoint>),
}
impl Easing {
    pub fn sample(&self, input: f32, before: bool) -> f32 {
        if input.is_nan() {
            return f32::NAN;
        }
        let input = f64::from(input);
        match self {
            Self::Linear => input as f32,
            Self::CubicBezier(points) => cubic(input, points.map(f64::from)) as f32,
            Self::Steps { count, position } => {
                let scaled = input * f64::from(*count);
                let mut step = scaled.floor();
                if matches!(position, StepPosition::Start | StepPosition::Both) {
                    step += 1.0;
                }
                if before && scaled.fract() == 0.0 {
                    step -= 1.0;
                }
                if input >= 0.0 {
                    step = step.max(0.0);
                }
                let jumps = f64::from(*count)
                    + match position {
                        StepPosition::None => -1.0,
                        StepPosition::Both => 1.0,
                        _ => 0.0,
                    };
                if input <= 1.0 {
                    step = step.min(jumps);
                }
                (step / jumps) as f32
            }
            Self::Piecewise(points) => {
                let Some(last_segment) = points.len().checked_sub(2) else {
                    return f32::NAN;
                };
                let index = points
                    .partition_point(|point| f64::from(point.input) <= input)
                    .saturating_sub(1)
                    .min(last_segment);
                let (a, b) = (points[index], points[index + 1]);
                if a.input == b.input {
                    return b.output;
                }
                let slope = (f64::from(b.output) - f64::from(a.output))
                    / (f64::from(b.input) - f64::from(a.input));
                line(input - f64::from(a.input), slope, f64::from(a.output)) as f32
            }
        }
    }
}

fn line(distance: f64, slope: f64, origin: f64) -> f64 {
    if slope == 0.0 {
        origin
    } else {
        origin + distance * slope
    }
}

fn coordinate(t: f64, first: f64, second: f64) -> f64 {
    let remaining = 1.0 - t;
    3.0 * remaining * remaining * t * first + 3.0 * remaining * t * t * second + t * t * t
}

fn cubic(input: f64, [x1, y1, x2, y2]: [f64; 4]) -> f64 {
    if input < 0.0 {
        return if x1 > 0.0 {
            line(input, y1 / x1, 0.0)
        } else if x2 > 0.0 {
            line(input, y2 / x2, 0.0)
        } else {
            0.0
        };
    }
    if input > 1.0 {
        return if x2 < 1.0 {
            line(input - 1.0, (1.0 - y2) / (1.0 - x2), 1.0)
        } else if x1 < 1.0 {
            line(input - 1.0, (1.0 - y1) / (1.0 - x1), 1.0)
        } else {
            1.0
        };
    }
    if input == 0.0 || input == 1.0 {
        return input;
    }
    let (mut lower, mut upper) = (0.0, 1.0);
    for _ in 0..BEZIER_ITERATIONS {
        let middle = (lower + upper) * 0.5;
        let value = coordinate(middle, x1, x2);
        if value == input {
            return coordinate(middle, y1, y2);
        }
        if value < input {
            lower = middle;
        } else {
            upper = middle;
        }
    }
    coordinate((lower + upper) * 0.5, y1, y2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn step_boundaries_apply_before_flag_without_clamping_extrapolation() {
        for (position, input, before, expected) in [
            (StepPosition::End, 0.25, false, 0.25),
            (StepPosition::End, 0.25, true, 0.0),
            (StepPosition::Start, 0.0, false, 0.25),
            (StepPosition::Start, 0.0, true, 0.0),
            (StepPosition::Both, 1.0, false, 1.0),
            (StepPosition::Both, 1.0, true, 0.8),
            (StepPosition::End, -0.25, false, -0.25),
            (StepPosition::End, 1.25, false, 1.25),
        ] {
            let actual = Easing::Steps { count: 4, position }.sample(input, before);
            assert_eq!(
                actual, expected,
                "position {position:?}, input {input}, before {before}"
            );
        }
    }

    #[test]
    fn cubic_tangents_and_degenerate_controls_stay_defined() {
        for (controls, input, expected) in [
            ([0.0, 0.0, 0.5, 1.0], -0.5, -1.0),
            ([0.5, 0.0, 1.0, 1.0], 1.5, 2.0),
            ([0.0, 1.0, 0.0, 1.0], -0.5, 0.0),
            ([1.0, 0.0, 1.0, 0.0], 1.5, 1.0),
            ([0.5, 0.0, 1.0, 1.0], f32::NEG_INFINITY, 0.0),
        ] {
            assert_eq!(
                Easing::CubicBezier(controls).sample(input, false),
                expected,
                "controls {controls:?}, input {input}"
            );
        }
        assert_eq!(
            Easing::CubicBezier([1.0, 0.0, 1.0, 1.0]).sample(f32::from_bits(1), false),
            0.0,
            "subnormal input must not stop with a much larger parameter"
        );
        assert!(
            Easing::CubicBezier([0.0, 0.0, 1.0, 1.0])
                .sample(f32::NAN, false)
                .is_nan()
        );
    }

    #[test]
    fn piecewise_discontinuities_use_last_point_and_constant_extensions() {
        let jump = Easing::Piecewise(vec![
            LinearPoint {
                input: 0.0,
                output: 0.0,
            },
            LinearPoint {
                input: 0.5,
                output: 0.25,
            },
            LinearPoint {
                input: 0.5,
                output: 0.75,
            },
            LinearPoint {
                input: 1.0,
                output: 1.0,
            },
        ]);
        assert_eq!(jump.sample(0.5, false), 0.75);
        let constant = Easing::Piecewise(vec![
            LinearPoint {
                input: 0.0,
                output: 2.0,
            },
            LinearPoint {
                input: 1.0,
                output: 2.0,
            },
        ]);
        assert_eq!(constant.sample(f32::INFINITY, false), 2.0);
        assert_eq!(constant.sample(f32::NEG_INFINITY, false), 2.0);
        assert!(Easing::Piecewise(Vec::new()).sample(0.5, false).is_nan());
    }
}
