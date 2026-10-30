//! Shared used-size constraints and CSS normal-flow axis equations.
use crate::style::{
    BoxSizing, CalculatedSize, LengthPercentage, Size, StyleError, box_values::unsupported,
    component::Component, grammar::Keyword, units::Relative,
};
use std::sync::Arc;

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

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct IntrinsicWidths {
    pub min: f32,
    pub max: f32,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub(crate) enum IntrinsicSizing {
    MinContent,
    MaxContent,
    FitContent,
    /// `fit-content(<length-percentage>)`: the fit-content formula with the argument in place
    /// of the available space (CSS Sizing 4 §3.2).
    FitContentLimit(LengthPercentage),
    /// `stretch`: the stretch-fit size, the available space (CSS Sizing 3 §4.2).
    Stretch,
}
impl IntrinsicSizing {
    /// `available` is the stretch-fit content size, when the context has one; a limit
    /// resolves against `basis` in the box `box-sizing` names, whose edges are `sizing_edges`.
    /// `None` where `stretch` has no stretch-fit size: it then behaves as `auto` in a
    /// preferred size, `0` in a minimum and `none` in a maximum (CSS Sizing 3 §4.2).
    pub(crate) fn resolve(
        &self,
        widths: IntrinsicWidths,
        available: Option<f32>,
        basis: Option<f32>,
        sizing_edges: f32,
    ) -> Option<f32> {
        let fit = |space: Option<f32>| widths.max.min(widths.min.max(space.unwrap_or(widths.max)));
        match self {
            Self::MinContent => Some(widths.min),
            Self::MaxContent => Some(widths.max),
            Self::FitContent => Some(fit(available)),
            // An unresolvable percentage behaves as `auto` would standing alone (§3.2).
            Self::FitContentLimit(limit) => Some(fit(Size::Value(limit.clone())
                .resolve_definite(basis)
                .map(|limit| (limit - sizing_edges).max(0.0))
                .or(available))),
            Self::Stretch => available,
        }
    }
}

/// What resolving `fit-content()` and `calc-size()` sizes needs, which most constraints lack.
#[derive(Debug)]
struct Resolution {
    /// The percentage basis, and the edges `box-sizing: border-box` includes.
    basis: Option<f32>,
    sizing_edges: f32,
    /// The `calc-size()` of an automatic preferred and minimum size, which applies once the
    /// context determines that size (CSS Values 5 §10.2).
    automatic: [Option<Arc<CalculatedSize>>; 2],
}

/// A [`Resolution`] shared by copies of one set of constraints; equal when it is the same one.
#[derive(Clone, Debug)]
pub(crate) struct SharedResolution(Arc<Resolution>);
impl PartialEq for SharedResolution {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}
impl Eq for SharedResolution {}
impl std::hash::Hash for SharedResolution {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        Arc::as_ptr(&self.0).hash(state);
    }
}

#[derive(Clone, Default)]
pub(crate) struct SizeConstraints {
    pub specified: Option<f32>,
    pub minimum: f32,
    pub maximum: Option<f32>,
    /// Present only where a size is a `fit-content()` or `calc-size()`, which keeps the
    /// constraints small on deeply recursive layout paths.
    resolution: Option<SharedResolution>,
}

/// [`SizeConstraints`] as a hashable key.
#[derive(Clone, Hash, PartialEq, Eq)]
pub(crate) struct ConstraintsKey {
    specified: Option<u32>,
    minimum: u32,
    maximum: Option<u32>,
    resolution: Option<SharedResolution>,
}
impl ConstraintsKey {
    pub(crate) fn constraints(&self) -> SizeConstraints {
        SizeConstraints {
            specified: self.specified.map(f32::from_bits),
            minimum: f32::from_bits(self.minimum),
            maximum: self.maximum.map(f32::from_bits),
            resolution: self.resolution.clone(),
        }
    }
}

impl SizeConstraints {
    /// Constraints already resolved to content-box sizes.
    pub(crate) fn fixed(specified: Option<f32>, minimum: f32, maximum: Option<f32>) -> Self {
        Self {
            specified,
            minimum,
            maximum,
            resolution: None,
        }
    }
    pub(crate) fn key(&self) -> ConstraintsKey {
        ConstraintsKey {
            specified: self.specified.map(f32::to_bits),
            minimum: self.minimum.to_bits(),
            maximum: self.maximum.map(f32::to_bits),
            resolution: self.resolution.clone(),
        }
    }
    fn resolution(&self) -> Option<&Resolution> {
        self.resolution.as_ref().map(|shared| &*shared.0)
    }
    /// The percentage basis of `fit-content()` and `calc-size()` sizes.
    fn basis(&self) -> Option<f32> {
        self.resolution().and_then(|resolution| resolution.basis)
    }
    /// The edges `box-sizing: border-box` includes in `fit-content()` and `calc-size()`
    /// sizes.
    pub(crate) fn sizing_edges(&self) -> f32 {
        self.resolution()
            .map_or(0.0, |resolution| resolution.sizing_edges)
    }
    /// The content-box size `calculated` gives a basis whose content-box size is `value`:
    /// the calculation sees and gives sizes in the box `box-sizing` names.
    fn calculate(&self, calculated: &CalculatedSize, value: f32) -> f32 {
        let edges = self.sizing_edges();
        (calculated.resolve(value + edges, self.basis()) - edges).max(0.0)
    }
    /// `index`'s `calc-size()` of an automatic size applied to the automatic size `value`.
    fn automatic(&self, index: usize, value: f32) -> f32 {
        match self
            .resolution()
            .and_then(|resolution| resolution.automatic[index].as_deref())
        {
            Some(calculated) => self.calculate(calculated, value),
            None => value,
        }
    }
    /// The preferred size, given the context's automatic size `automatic` for a box whose
    /// preferred size is automatic.
    pub(crate) fn preferred(&self, automatic: f32) -> f32 {
        self.specified
            .unwrap_or_else(|| self.automatic(0, automatic))
    }
    /// `size`'s content-box value where its basis's is `value`.
    pub(crate) fn adjust(&self, size: &Size, value: f32) -> f32 {
        match size {
            Size::Calculated(calculated) => self.calculate(calculated, value),
            _ => value,
        }
    }
    /// Whether the preferred size is automatic and a `calc-size()` adjusts it.
    pub(crate) fn calculates_automatic(&self) -> bool {
        self.specified.is_none()
            && self
                .resolution()
                .is_some_and(|resolution| resolution.automatic[0].is_some())
    }
    /// The minimum size where the context gives an automatic minimum size `automatic`, such
    /// as a flex item's content-based minimum.
    pub(crate) fn automatic_minimum(&self, automatic: f32) -> f32 {
        self.automatic(1, automatic)
    }
    /// Intrinsic sizes describe the content box, regardless of box-sizing.
    #[must_use]
    pub(crate) fn with_intrinsic(
        mut self,
        sizes: [&Size; 3],
        widths: IntrinsicWidths,
        available: Option<f32>,
    ) -> Self {
        for (index, size) in sizes.into_iter().enumerate() {
            if let Some(value) = size.intrinsic().and_then(|keyword| {
                keyword.resolve(widths, available, self.basis(), self.sizing_edges())
            }) {
                let value = self.adjust(size, value.max(0.0));
                match index {
                    0 => self.specified = Some(value),
                    1 => self.minimum = value,
                    _ => self.maximum = Some(value),
                }
            }
        }
        self
    }
    /// Resolves `stretch` bases against the stretch-fit content size `available` (CSS Sizing 4
    /// §3.2), where the axis does not otherwise resolve intrinsic keywords.
    #[must_use]
    pub(crate) fn with_stretch(self, sizes: [&Size; 3], available: Option<f32>) -> Self {
        let stretched = sizes.map(|size| size.intrinsic() == Some(&IntrinsicSizing::Stretch));
        match available {
            Some(available) if stretched.contains(&true) => self.with_intrinsic(
                sizes.map(|size| {
                    if size.intrinsic() == Some(&IntrinsicSizing::Stretch) {
                        size
                    } else {
                        &Size::Auto
                    }
                }),
                IntrinsicWidths {
                    min: available,
                    max: available,
                },
                Some(available),
            ),
            _ => self,
        }
    }
    pub(crate) fn new(
        sizes: [&Size; 3],
        basis: Option<f32>,
        sizing: BoxSizing,
        edges: f32,
    ) -> Self {
        let sizing_edges = if sizing == BoxSizing::BorderBox {
            edges
        } else {
            0.0
        };
        let [specified, minimum, maximum] = sizes.map(|size| {
            size.resolve_definite(basis)
                .map(|value| (value - sizing_edges).max(0.0))
        });
        let automatic = |size: &Size, value: Option<f32>| match size {
            // An intrinsic basis the axis resolves as automatic adjusts the automatic size.
            Size::Calculated(calculated) if value.is_none() => Some(calculated.clone()),
            _ => None,
        };
        let resolved = sizes.iter().any(|size| {
            matches!(
                size,
                Size::Calculated(_) | Size::Intrinsic(IntrinsicSizing::FitContentLimit(_))
            )
        });
        let mut constraints = Self {
            specified,
            minimum: 0.0,
            maximum,
            resolution: resolved.then(|| {
                SharedResolution(Arc::new(Resolution {
                    basis,
                    sizing_edges,
                    automatic: [automatic(sizes[0], specified), automatic(sizes[1], minimum)],
                }))
            }),
        };
        constraints.minimum = minimum.unwrap_or_else(|| constraints.automatic_minimum(0.0));
        constraints
    }
    pub(crate) fn constrain(&self, value: f32) -> f32 {
        self.maximum
            .map_or(value, |maximum| value.min(maximum))
            .max(self.minimum)
    }
}

/// CSS Values 5 §10: a `calc-size()` basis is an intrinsic size keyword its property accepts,
/// and maximum sizes accept `none` rather than `auto`.
pub(super) fn valid(property: crate::style::grammar::Property, value: &Component) -> bool {
    use crate::style::grammar::Property as P;
    !matches!(
        property,
        P::MaxWidth | P::MaxHeight | P::MaxInlineSize | P::MaxBlockSize | P::MaxSize
    ) || !has_auto_basis(value)
}

/// Whether a `calc-size()` in `value` has an `auto` basis, directly or through nesting.
fn has_auto_basis(value: &Component) -> bool {
    match value {
        Component::Function(Keyword::CalcSize, Some(arguments)) => match &**arguments {
            Component::Tokens(arguments) => {
                let mut input = cssparser::ParserInput::new(arguments);
                auto_basis(&mut cssparser::Parser::new(&mut input))
            }
            _ => false,
        },
        Component::Reference(_, inner) | Component::Block(_, inner) => has_auto_basis(inner),
        Component::Group(items) => items.iter().any(has_auto_basis),
        Component::Slots(slots) => slots.iter().flatten().any(has_auto_basis),
        _ => false,
    }
}

fn auto_basis(input: &mut cssparser::Parser<'_, '_>) -> bool {
    match input.next() {
        Ok(cssparser::Token::Ident(name)) => Keyword::from_ident(name) == Some(Keyword::Auto),
        Ok(cssparser::Token::Function(name))
            if Keyword::from_ident(name) == Some(Keyword::CalcSize) =>
        {
            input
                .parse_nested_block(|nested| {
                    Ok::<_, cssparser::ParseError<'_, ()>>(auto_basis(nested))
                })
                .unwrap_or(false)
        }
        _ => false,
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
        let natural = used_size.unwrap_or_else(|| {
            self.preferred(
                (containing - edges - start.unwrap_or(0.0) - end.unwrap_or(0.0)).max(0.0),
            )
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
