//! CSS Values 4 calculation trees kept until layout supplies the percentage basis.
use crate::paint::Easing;
use serde::Serialize;
use std::cmp::Ordering;

/// Finite boundary shared with the representation of authored numeric values.
pub(crate) const MAX_CALC_MAGNITUDE: f32 = f32::MAX;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum RoundingStrategy {
    Nearest,
    Up,
    Down,
    ToZero,
}

/// Trigonometric functions take and return radians.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum UnaryFunction {
    Abs,
    Sign,
    Sin,
    Cos,
    Tan,
    Asin,
    Acos,
    Atan,
    Sqrt,
    Exp,
    Ln,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum BinaryFunction {
    Round(RoundingStrategy),
    Mod,
    Rem,
    Atan2,
    Pow,
    Log,
}

/// Leaves hold points, plain numbers or degrees; only `Percentage` depends on the basis.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub enum Calculation {
    Value(f32),
    /// Fraction of the percentage basis.
    Percentage(f32),
    Sum(Vec<Self>),
    Product(Vec<Self>),
    Invert(Box<Self>),
    Min(Vec<Self>),
    Max(Vec<Self>),
    Hypot(Vec<Self>),
    Unary(UnaryFunction, Box<Self>),
    Binary(BinaryFunction, Box<[Self; 2]>),
    Progress(Box<[Self; 3]>),
    Mix(Box<[Self; 3]>),
    Eased(Easing, Box<Self>),
}
impl Calculation {
    /// A length under zoom: scale its result while preserving the physical percentage basis.
    pub(crate) fn zoomed_length(mut self, factor: f32) -> Self {
        self.rebase_percentages(factor);
        self.scale(factor)
    }

    fn rebase_percentages(&mut self, factor: f32) {
        match self {
            Self::Value(_) => {}
            Self::Percentage(value) => *value /= factor,
            Self::Sum(values)
            | Self::Product(values)
            | Self::Min(values)
            | Self::Max(values)
            | Self::Hypot(values) => {
                for value in values {
                    value.rebase_percentages(factor);
                }
            }
            Self::Binary(_, values) => {
                for value in &mut **values {
                    value.rebase_percentages(factor);
                }
            }
            Self::Progress(values) | Self::Mix(values) => {
                for value in &mut **values {
                    value.rebase_percentages(factor);
                }
            }
            Self::Invert(value) | Self::Unary(_, value) | Self::Eased(_, value) => {
                value.rebase_percentages(factor);
            }
        }
    }
    /// Even zero percentages retain an indefinite percentage basis.
    pub fn percentage(fraction: f32) -> Self {
        Self::Percentage(fraction)
    }
    #[must_use]
    pub fn negate(self) -> Self {
        match self {
            Self::Sum(terms)
                if terms
                    .iter()
                    .all(|term| matches!(term, Self::Value(_) | Self::Percentage(_))) =>
            {
                Self::Sum(terms.into_iter().map(Self::negate).collect()).simplify()
            }
            other => other.scale(-1.0),
        }
    }
    #[must_use]
    pub fn scale(self, factor: f32) -> Self {
        Self::Product(vec![Self::Value(factor), self]).simplify()
    }
    pub fn unary(function: UnaryFunction, argument: Self) -> Self {
        Self::Unary(function, Box::new(argument)).simplify()
    }
    pub fn binary(function: BinaryFunction, first: Self, second: Self) -> Self {
        Self::Binary(function, Box::new([first, second])).simplify()
    }
    pub fn progress(value: Self, start: Self, end: Self) -> Self {
        Self::Progress(Box::new([value, start, end])).simplify()
    }
    pub fn mix(progress: Self, start: Self, end: Self) -> Self {
        Self::Mix(Box::new([progress, start, end])).simplify()
    }
    pub fn eased(easing: Easing, input: Self) -> Self {
        Self::Eased(easing, Box::new(input)).simplify()
    }
    /// Folds basis-independent nodes into values and linear percentage terms into one leaf.
    #[must_use]
    pub fn simplify(self) -> Self {
        let constant = match &self {
            Self::Value(_) | Self::Percentage(_) => return self,
            Self::Sum(items)
            | Self::Product(items)
            | Self::Min(items)
            | Self::Max(items)
            | Self::Hypot(items) => items.iter().all(Self::is_value),
            Self::Binary(_, pair) => pair.iter().all(Self::is_value),
            Self::Progress(values) | Self::Mix(values) => values.iter().all(Self::is_value),
            Self::Eased(_, input) => input.is_value(),
            Self::Invert(item) | Self::Unary(_, item) => item.is_value(),
        };
        if constant {
            return Self::Value(self.evaluate(0.0));
        }
        match self {
            Self::Sum(terms) => {
                let (mut value, mut percentage, mut rest) = (-0.0, -0.0, Vec::new());
                let (mut has_value, mut has_percentage) = (false, false);
                for term in terms.into_iter().flat_map(|term| match term {
                    Self::Sum(nested) => nested,
                    other => vec![other],
                }) {
                    match term {
                        Self::Value(v) => {
                            value += v;
                            has_value = true;
                        }
                        Self::Percentage(p) => {
                            percentage += p;
                            has_percentage = true;
                        }
                        other => rest.push(other),
                    }
                }
                if has_percentage {
                    rest.push(Self::Percentage(percentage));
                }
                if has_value {
                    rest.push(Self::Value(value));
                }
                single_or(rest, Self::Sum)
            }
            Self::Product(factors) => {
                let mut scale = 1.0;
                let mut rest = Vec::new();
                for factor in factors {
                    match factor {
                        Self::Value(v) => scale *= v,
                        other => rest.push(other),
                    }
                }
                if let [Self::Percentage(p)] = rest.as_slice() {
                    Self::percentage(p * scale)
                } else {
                    if scale != 1.0 {
                        rest.insert(0, Self::Value(scale));
                    }
                    single_or(rest, Self::Product)
                }
            }
            other => other,
        }
    }
    fn is_value(&self) -> bool {
        matches!(self, Self::Value(_))
    }
    /// IEEE-754 evaluation; NaN and infinities propagate as CSS Values 4 specifies.
    pub fn evaluate(&self, basis: f32) -> f32 {
        match self {
            Self::Value(value) => *value,
            Self::Percentage(fraction) => fraction * basis,
            Self::Sum(terms) => terms.iter().map(|term| term.evaluate(basis)).sum(),
            Self::Product(factors) => factors.iter().map(|f| f.evaluate(basis)).product(),
            Self::Invert(value) => 1.0 / value.evaluate(basis),
            Self::Min(items) => extreme(items, basis, f32::INFINITY, Ordering::Greater),
            Self::Max(items) => extreme(items, basis, f32::NEG_INFINITY, Ordering::Less),
            Self::Hypot(items) => items.iter().fold(0.0, |length, item| {
                let value = item.evaluate(basis);
                if length.is_nan() || value.is_nan() {
                    f32::NAN
                } else {
                    length.hypot(value)
                }
            }),
            Self::Unary(function, argument) => unary(*function, argument.evaluate(basis)),
            Self::Binary(function, operands) => binary(
                *function,
                operands[0].evaluate(basis),
                operands[1].evaluate(basis),
            ),
            Self::Eased(easing, input) => easing.sample(input.evaluate(basis), false),
            Self::Mix(operands) => {
                let progress = operands[0].evaluate(basis);
                operands[1].evaluate(basis) * (1.0 - progress)
                    + operands[2].evaluate(basis) * progress
            }
            Self::Progress(operands) => progress(
                operands[0].evaluate(basis),
                operands[1].evaluate(basis),
                operands[2].evaluate(basis),
            ),
        }
    }
    /// Top-level used value.
    pub fn resolve(&self, basis: f32) -> f32 {
        censor(self.evaluate(basis))
    }
}

/// Top-level NaN becomes zero and infinities clamp to finite magnitudes.
pub(crate) fn censor(value: f32) -> f32 {
    if value.is_nan() || value == 0.0 {
        0.0
    } else {
        value.clamp(-MAX_CALC_MAGNITUDE, MAX_CALC_MAGNITUDE)
    }
}

fn progress(value: f32, start: f32, end: f32) -> f32 {
    if value.is_nan() || start.is_nan() || end.is_nan() {
        f32::NAN
    } else if start == end {
        if value == start {
            0.0
        } else if value < start {
            f32::NEG_INFINITY
        } else {
            f32::INFINITY
        }
    } else {
        (value - start) / (end - start)
    }
}

fn single_or(
    mut items: Vec<Calculation>,
    wrap: fn(Vec<Calculation>) -> Calculation,
) -> Calculation {
    if items.len() == 1 {
        items.remove(0)
    } else {
        wrap(items)
    }
}

fn extreme(items: &[Calculation], basis: f32, start: f32, replace_when: Ordering) -> f32 {
    items.iter().fold(start, |best, item| {
        let value = item.evaluate(basis);
        if best.is_nan() || value.is_nan() {
            f32::NAN
        } else if best.total_cmp(&value) == replace_when {
            value
        } else {
            best
        }
    })
}

fn unary(function: UnaryFunction, value: f32) -> f32 {
    match function {
        UnaryFunction::Abs => value.abs(),
        UnaryFunction::Sign if value == 0.0 || value.is_nan() => value,
        UnaryFunction::Sign => value.signum(),
        UnaryFunction::Sin => value.sin(),
        UnaryFunction::Cos => value.cos(),
        UnaryFunction::Tan => value.tan(),
        UnaryFunction::Asin => value.asin(),
        UnaryFunction::Acos => value.acos(),
        UnaryFunction::Atan => value.atan(),
        UnaryFunction::Sqrt => value.sqrt(),
        UnaryFunction::Exp => value.exp(),
        UnaryFunction::Ln => value.ln(),
    }
}

fn binary(function: BinaryFunction, first: f32, second: f32) -> f32 {
    if first.is_nan() || second.is_nan() {
        return f32::NAN;
    }
    match function {
        BinaryFunction::Round(strategy) => round(strategy, first, second),
        BinaryFunction::Mod => {
            if second.is_infinite()
                && first.is_finite()
                && first.is_sign_negative() != second.is_sign_negative()
            {
                return f32::NAN;
            }
            let remainder = first % second;
            if remainder == 0.0 {
                0.0_f32.copysign(second)
            } else if remainder.is_sign_negative() != second.is_sign_negative() {
                remainder + second
            } else {
                remainder
            }
        }
        BinaryFunction::Rem => first % second,
        BinaryFunction::Atan2 => first.atan2(second),
        BinaryFunction::Pow if first.abs() == 1.0 && second.is_infinite() => f32::NAN,
        BinaryFunction::Pow => first.powf(second),
        BinaryFunction::Log => {
            if first < 0.0 || second < 0.0 || second == 1.0 {
                f32::NAN
            } else if first == 0.0 {
                f32::NEG_INFINITY
            } else if first == 1.0 {
                0.0
            } else if first == f32::INFINITY {
                f32::INFINITY
            } else {
                first.ln() / second.ln()
            }
        }
    }
}

fn round(strategy: RoundingStrategy, value: f32, step: f32) -> f32 {
    if step == 0.0 || (value.is_infinite() && step.is_infinite()) {
        return f32::NAN;
    }
    if value.is_infinite() {
        return value;
    }
    if step.is_infinite() {
        return match strategy {
            RoundingStrategy::Up if value > 0.0 => f32::INFINITY,
            RoundingStrategy::Down if value < 0.0 => f32::NEG_INFINITY,
            RoundingStrategy::Up | RoundingStrategy::Down if value == 0.0 => value,
            RoundingStrategy::Up => -0.0,
            RoundingStrategy::Down => 0.0,
            RoundingStrategy::Nearest | RoundingStrategy::ToZero => 0.0_f32.copysign(value),
        };
    }
    // Every finite f32 quotient fits in f64, including ratios of subnormal values.
    let (value, step) = (f64::from(value), f64::from(step).abs());
    let lower = (value / step).floor() * step;
    let upper = (value / step).ceil() * step;
    (match strategy {
        RoundingStrategy::Up => upper,
        RoundingStrategy::Down => lower,
        RoundingStrategy::ToZero if value < 0.0 => upper,
        RoundingStrategy::ToZero => lower,
        RoundingStrategy::Nearest if value - lower < upper - value => lower,
        RoundingStrategy::Nearest => upper,
    }) as f32
}
