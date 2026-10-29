//! Shared used-size constraints and CSS normal-flow axis equations.
use crate::style::{
    BoxSizing, Size, StyleError, box_values::unsupported, component::Component, grammar::Keyword,
    units::Relative,
};

/// `aspect-ratio` (CSS Sizing 4 §4.1).
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize)]
pub(crate) struct AspectRatio {
    pub auto: bool,
    /// Width over height; `None` without a `<ratio>` or for a degenerate one, which behaves
    /// as `auto`.
    pub ratio: Option<f32>,
}
impl Default for AspectRatio {
    fn default() -> Self {
        Self {
            auto: true,
            ratio: None,
        }
    }
}
impl AspectRatio {
    pub(super) fn from_component(
        value: &Component,
        relative: &Relative<'_>,
    ) -> Result<Self, StyleError> {
        if value.keyword() == Some(Keyword::Auto) {
            return Ok(Self::default());
        }
        let ratio = value
            .slot(1)
            .map(|ratio| {
                let mut numbers = Vec::new();
                ratio_numbers(ratio, &mut numbers);
                let [width, height] = match numbers.as_slice() {
                    [width] => [*width, &Component::Number(1.0)],
                    [width, height] => [*width, *height],
                    _ => return Err(unsupported(ratio)),
                };
                Ok([
                    crate::style::values::number(width, relative)?,
                    crate::style::values::number(height, relative)?,
                ])
            })
            .transpose()?;
        // CSS Values 4 §7.4: a ratio with a zero or infinite number is degenerate.
        let ratio = ratio
            .filter(|numbers| numbers.iter().all(|n| n.is_finite() && *n > 0.0))
            .map(|[width, height]| width / height);
        Ok(Self {
            auto: value.slot(0).is_some(),
            ratio,
        })
    }
    /// The preferred aspect ratio of a box with natural aspect ratio `natural`, and whether
    /// it relates the boxes `box-sizing` names rather than the content boxes.
    pub(crate) fn preferred(self, natural: Option<f32>) -> Option<(f32, bool)> {
        match (self.auto, self.ratio) {
            (_, None) => natural.map(|ratio| (ratio, false)),
            (false, Some(ratio)) => Some((ratio, true)),
            (true, Some(ratio)) => Some((natural.unwrap_or(ratio), false)),
        }
    }
}

/// The numbers of a `<ratio>`, in order, without its `/` delimiter.
fn ratio_numbers<'a>(value: &'a Component, output: &mut Vec<&'a Component>) {
    match value.unwrap() {
        Component::Group(items) => items.iter().for_each(|item| ratio_numbers(item, output)),
        Component::Delim(_) => {}
        number => output.push(number),
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct IntrinsicWidths {
    pub min: f32,
    pub max: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[expect(
    clippy::enum_variant_names,
    reason = "variants name the CSS min-content, max-content and fit-content keywords"
)]
pub(crate) enum IntrinsicSizing {
    MinContent,
    MaxContent,
    FitContent,
}
impl IntrinsicSizing {
    pub(crate) fn resolve(self, widths: IntrinsicWidths, available: Option<f32>) -> f32 {
        match self {
            Self::MinContent => widths.min,
            Self::MaxContent => widths.max,
            Self::FitContent => widths
                .max
                .min(widths.min.max(available.unwrap_or(widths.max))),
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) struct SizeConstraints {
    pub specified: Option<f32>,
    pub minimum: f32,
    pub maximum: Option<f32>,
}
impl SizeConstraints {
    /// Intrinsic sizes describe the content box, regardless of box-sizing.
    #[must_use]
    pub(crate) fn with_intrinsic(
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
    pub(crate) fn new(
        sizes: [&Size; 3],
        basis: Option<f32>,
        sizing: BoxSizing,
        edges: f32,
    ) -> Self {
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
    pub(crate) fn constrain(&self, value: f32) -> f32 {
        self.maximum
            .map_or(value, |maximum| value.min(maximum))
            .max(self.minimum)
    }
}

/// How an over-constrained normal-flow equation is satisfied.
#[derive(Clone, Copy, Eq, PartialEq)]
pub(crate) enum Overconstraint {
    AdjustEndMargin,
    /// CSS Page 3 §3 retains the page margins and resizes the containing block.
    ResizeContaining,
}

#[derive(Clone, Copy)]
pub(crate) struct UsedAxis {
    pub size: f32,
    pub margins: [f32; 2],
    pub containing: f32,
}

impl SizeConstraints {
    pub(crate) fn normal_flow(
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
        let adjustable = usize::from(!reverse);
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
