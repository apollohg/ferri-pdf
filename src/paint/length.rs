use crate::paint::Calculation;
use serde::Serialize;
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub enum LengthPercentage {
    Length(f32),
    Percentage(f32),
    /// Math function that still depends on the percentage basis.
    Calc(Arc<Calculation>),
}
impl LengthPercentage {
    pub(crate) fn zoomed(&self, factor: f32) -> Self {
        match self {
            Self::Length(value) => Self::Length(value * factor),
            Self::Percentage(_) => self.clone(),
            Self::Calc(value) => Self::Calc(Arc::new((**value).clone().zoomed_length(factor))),
        }
    }
    pub const ZERO: Self = Self::Length(0.0);
    pub fn resolve(&self, basis: f32) -> f32 {
        match self {
            Self::Length(value) => *value,
            Self::Percentage(value) => value * basis,
            Self::Calc(calculation) => calculation.resolve(basis),
        }
    }
    /// Zero for every basis; calculations are never folded down to a constant zero.
    pub fn is_zero(&self) -> bool {
        matches!(self, Self::Length(value) | Self::Percentage(value) if *value == 0.0)
    }
}
impl From<Calculation> for LengthPercentage {
    fn from(calculation: Calculation) -> Self {
        match calculation.simplify() {
            Calculation::Value(value) => Self::Length(crate::paint::calculation::censor(value)),
            Calculation::Percentage(fraction) if fraction.is_finite() => Self::Percentage(fraction),
            calculation => Self::Calc(Arc::new(calculation)),
        }
    }
}
