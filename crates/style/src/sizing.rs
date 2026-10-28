//! Shared used-size constraints and CSS normal-flow axis equations.
use crate::{BoxSizing, Size};

#[derive(Clone, Copy, Debug, Default)]
pub struct IntrinsicWidths {
    pub min: f32,
    pub max: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub enum IntrinsicSizing {
    MinContent,
    MaxContent,
    FitContent,
}
impl IntrinsicSizing {
    pub fn resolve(self, widths: IntrinsicWidths, available: Option<f32>) -> f32 {
        match self {
            Self::MinContent => widths.min,
            Self::MaxContent => widths.max,
            Self::FitContent => widths
                .max
                .min(widths.min.max(available.unwrap_or(widths.max))),
        }
    }
}

pub struct SizeConstraints {
    pub specified: Option<f32>,
    pub minimum: f32,
    pub maximum: Option<f32>,
}
impl SizeConstraints {
    /// Intrinsic sizes describe the content box, regardless of box-sizing.
    pub fn with_intrinsic(
        mut self,
        sizes: [&Size; 3],
        widths: IntrinsicWidths,
        available: Option<f32>,
    ) -> Self {
        for (index, size) in sizes.into_iter().enumerate() {
            if let Size::Intrinsic(value) = size {
                let value = value.resolve(widths, available).max(0.0);
                match index {
                    0 => self.specified = Some(value),
                    1 => self.minimum = value,
                    _ => self.maximum = Some(value),
                }
            }
        }
        self
    }
    pub fn new(sizes: [&Size; 3], basis: Option<f32>, sizing: BoxSizing, edges: f32) -> Self {
        let [specified, minimum, maximum] = sizes.map(|size| {
            size.resolve_definite(basis).map(|value| {
                if sizing == BoxSizing::BorderBox {
                    (value - edges).max(0.0)
                } else {
                    value
                }
            })
        });
        Self {
            specified,
            minimum: minimum.unwrap_or(0.0),
            maximum,
        }
    }
    pub fn constrain(&self, value: f32) -> f32 {
        self.maximum
            .map_or(value, |maximum| value.min(maximum))
            .max(self.minimum)
    }
}

/// How an over-constrained normal-flow equation is satisfied.
#[derive(Clone, Copy, Eq, PartialEq)]
pub enum Overconstraint {
    AdjustEndMargin,
    /// CSS Page 3 §3 retains the page margins and resizes the containing block.
    ResizeContaining,
}

#[derive(Clone, Copy)]
pub struct UsedAxis {
    pub size: f32,
    pub margins: [f32; 2],
    pub containing: f32,
}

impl SizeConstraints {
    pub fn normal_flow(
        &self,
        containing: f32,
        edges: f32,
        margins: [Option<f32>; 2],
        used_size: Option<f32>,
        reverse: bool,
        overconstraint: Overconstraint,
    ) -> UsedAxis {
        let [start, end] = margins;
        let natural = used_size.or(self.specified).unwrap_or_else(|| {
            (containing - edges - start.unwrap_or(0.0) - end.unwrap_or(0.0)).max(0.0)
        });
        let size = used_size.unwrap_or_else(|| self.constrain(natural));
        let specified = used_size.is_some() || self.specified.is_some() || size != natural;
        let remainder = containing - edges - size - start.unwrap_or(0.0) - end.unwrap_or(0.0);
        let mut used = [start.unwrap_or(0.0), end.unwrap_or(0.0)];
        let adjustable = if reverse { 0 } else { 1 };
        let mut resize = false;
        if specified && remainder >= 0.0 {
            match (start, end) {
                (None, None) => used = [remainder / margins.len() as f32; 2],
                (None, Some(_)) => used[0] = remainder,
                (Some(_), None) => used[1] = remainder,
                (Some(_), Some(_)) => resize = true,
            }
        } else {
            resize = true;
        }
        let containing = if resize && overconstraint == Overconstraint::ResizeContaining {
            if specified || remainder < 0.0 {
                size + edges + used[0] + used[1]
            } else {
                containing
            }
        } else {
            if resize {
                used[adjustable] += remainder;
            }
            containing
        };
        UsedAxis {
            size,
            margins: used,
            containing,
        }
    }
}
