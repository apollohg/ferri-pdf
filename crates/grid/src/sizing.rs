//! The grid track sizing algorithm for one axis (CSS Grid 1 §11.3–§11.8).
use crate::GridError;
use std::ops::Range;

/// Distributable space below this is spent; it absorbs `f64` rounding in the freezing loops.
const EPSILON: f64 = 1e-6;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MinSizing {
    Fixed(f32),
    MinContent,
    MaxContent,
    Auto,
}
impl MinSizing {
    fn intrinsic(self) -> bool {
        !matches!(self, Self::Fixed(_))
    }
    fn content(self) -> bool {
        matches!(self, Self::MinContent | Self::MaxContent)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MaxSizing {
    Fixed(f32),
    MinContent,
    MaxContent,
    Auto,
    FitContent(f32),
    /// `fr`.
    Flex(f32),
}
impl MaxSizing {
    fn intrinsic(self) -> bool {
        !matches!(self, Self::Fixed(_) | Self::Flex(_))
    }
    /// `auto` and `fit-content()` maximums size like `max-content`.
    fn max_content(self) -> bool {
        matches!(self, Self::MaxContent | Self::Auto | Self::FitContent(_))
    }
    /// A fixed maximum, which limits contributions.
    fn fixed_limit(self) -> Option<f64> {
        match self {
            Self::Fixed(limit) | Self::FitContent(limit) => Some(f64::from(limit)),
            _ => None,
        }
    }
    fn flex(self) -> Option<f64> {
        match self {
            Self::Flex(factor) => Some(f64::from(factor)),
            _ => None,
        }
    }
}

/// A track's resolved sizing functions; percentages are resolved or treated as `auto`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TrackSizing {
    pub min: MinSizing,
    pub max: MaxSizing,
}

/// The available grid space (CSS Grid 1 §11.1).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AvailableSpace {
    Definite(f32),
    /// Sizing under a min-content constraint: no free space.
    MinContent,
    /// Sizing under a max-content constraint, or an indefinite size: infinite free space.
    MaxContent,
}

/// The grid container's inner minimum and maximum size along the axis.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SizeBounds {
    pub min: f32,
    pub max: Option<f32>,
}

/// An item's outer size contributions to the tracks it spans.
#[derive(Clone, Debug, PartialEq)]
pub struct SizingItem {
    pub tracks: Range<usize>,
    pub min_content: f32,
    pub max_content: f32,
    /// Outer size with the item's specified minimum size as its preferred size.
    pub minimum: f32,
    /// With `min-*: auto`, the outer content-based minimum size (CSS Grid 1 §6.6), which
    /// applies to an `auto` minimum track unless a multi-track span crosses a flexible track.
    pub content_minimum: Option<f32>,
}

#[derive(Clone, Copy, Debug)]
struct Track {
    base: f64,
    limit: f64,
    min: MinSizing,
    max: MaxSizing,
    infinitely_growable: bool,
}
impl Track {
    fn limit_or_base(&self) -> f64 {
        if self.limit.is_finite() {
            self.limit
        } else {
            self.base
        }
    }
}

/// Which size of the affected tracks a distribution increases.
#[derive(Clone, Copy, PartialEq)]
enum Affected {
    Base,
    Limit,
}

/// The contributions being accommodated, which choose the tracks that grow beyond limits.
#[derive(Clone, Copy, PartialEq)]
enum Accommodating {
    MinContent,
    MaxContent,
    Limits,
}

struct Prepared<'a> {
    item: &'a SizingItem,
    minimum: f64,
    crosses_flex: bool,
    /// The sum of the spanned tracks' fixed maximums when every one has one.
    fixed_limit: Option<f64>,
}
impl Prepared<'_> {
    /// A contribution limited by fixed maximums and floored by the minimum contribution.
    fn limited(&self, contribution: f32) -> f64 {
        let contribution = f64::from(contribution);
        self.fixed_limit
            .map_or(contribution, |limit| contribution.min(limit))
            .max(self.minimum)
    }
}

struct Sizer<'a> {
    tracks: Vec<Track>,
    gap: f64,
    space: AvailableSpace,
    items: Vec<Prepared<'a>>,
}

/// Base sizes of the tracks, which are their used sizes.
pub fn size_tracks(
    sizing: &[TrackSizing],
    gap: f32,
    items: &[SizingItem],
    space: AvailableSpace,
    bounds: SizeBounds,
    stretch: bool,
) -> Result<Vec<f32>, GridError> {
    validate(sizing, gap, items, space, bounds)?;
    let mut sizer = Sizer::new(sizing, gap, items, space);
    sizer.resolve_intrinsic();
    sizer.maximize(bounds);
    sizer.expand_flexible(bounds);
    if stretch {
        sizer.stretch_auto(bounds);
    }
    let sizes: Vec<f32> = sizer.tracks.iter().map(|track| track.base as f32).collect();
    if sizes.iter().any(|size| !size.is_finite()) {
        return Err(GridError::InvalidGeometry);
    }
    Ok(sizes)
}

fn validate(
    sizing: &[TrackSizing],
    gap: f32,
    items: &[SizingItem],
    space: AvailableSpace,
    bounds: SizeBounds,
) -> Result<(), GridError> {
    let length = |value: f32| value.is_finite() && value >= 0.0;
    let track_valid = |track: &TrackSizing| {
        let min = match track.min {
            MinSizing::Fixed(value) => length(value),
            _ => true,
        };
        let max = match track.max {
            MaxSizing::Fixed(value) | MaxSizing::FitContent(value) | MaxSizing::Flex(value) => {
                length(value)
            }
            _ => true,
        };
        min && max
    };
    let space_valid = match space {
        AvailableSpace::Definite(value) => length(value),
        _ => true,
    };
    if !sizing.iter().all(track_valid)
        || !length(gap)
        || !space_valid
        || !length(bounds.min)
        || bounds.max.is_some_and(|max| !length(max))
    {
        return Err(GridError::InvalidGeometry);
    }
    for item in items {
        if item.tracks.is_empty() || item.tracks.end > sizing.len() {
            return Err(GridError::InvalidSpan {
                tracks: sizing.len(),
            });
        }
        let contributions = [item.min_content, item.max_content, item.minimum];
        if !contributions
            .into_iter()
            .chain(item.content_minimum)
            .all(|value| value.is_finite())
        {
            return Err(GridError::InvalidGeometry);
        }
    }
    Ok(())
}

impl<'a> Sizer<'a> {
    /// CSS Grid 1 §11.4: initial base sizes and growth limits.
    fn new(
        sizing: &[TrackSizing],
        gap: f32,
        items: &'a [SizingItem],
        space: AvailableSpace,
    ) -> Self {
        let tracks: Vec<Track> = sizing
            .iter()
            .map(|track| {
                let base = match track.min {
                    MinSizing::Fixed(value) => f64::from(value),
                    _ => 0.0,
                };
                let limit = match track.max {
                    MaxSizing::Fixed(value) => f64::from(value),
                    _ => f64::INFINITY,
                };
                Track {
                    base,
                    limit: limit.max(base),
                    min: track.min,
                    max: track.max,
                    infinitely_growable: false,
                }
            })
            .collect();
        let gap = f64::from(gap);
        let items = items
            .iter()
            .map(|item| {
                let spanned = &tracks[item.tracks.clone()];
                let crosses_flex = spanned.iter().any(|track| track.max.flex().is_some());
                let gaps = gap * (spanned.len() - 1) as f64;
                let fixed_limit = spanned
                    .iter()
                    .map(|track| track.max.fixed_limit())
                    .sum::<Option<f64>>()
                    .map(|sum| sum + gaps);
                let automatic = item
                    .content_minimum
                    .filter(|_| spanned.len() == 1 || !crosses_flex)
                    .filter(|_| spanned.iter().any(|track| track.min == MinSizing::Auto))
                    .map(|content| {
                        let content = f64::from(content);
                        let all_fixed = spanned
                            .iter()
                            .map(|track| match track.max {
                                MaxSizing::Fixed(limit) => Some(f64::from(limit)),
                                _ => None,
                            })
                            .sum::<Option<f64>>();
                        all_fixed.map_or(content, |sum| content.min(sum + gaps))
                    });
                Prepared {
                    item,
                    minimum: automatic.map_or(f64::from(item.minimum), |content| {
                        content.max(f64::from(item.minimum))
                    }),
                    crosses_flex,
                    fixed_limit,
                }
            })
            .collect();
        Self {
            tracks,
            gap,
            space,
            items,
        }
    }

    fn constrained(&self) -> bool {
        !matches!(self.space, AvailableSpace::Definite(_))
    }

    /// The minimum contribution, or under a min-/max-content constraint the limited
    /// min-content contribution (CSS Grid 1 §11.5 step 3.1).
    fn intrinsic_minimum(&self, item: &Prepared<'_>) -> f64 {
        if self.constrained() {
            item.limited(item.item.min_content)
        } else {
            item.minimum
        }
    }

    /// CSS Grid 1 §11.5.
    fn resolve_intrinsic(&mut self) {
        self.size_single_span();
        let mut spans: Vec<usize> = self
            .items
            .iter()
            .filter(|item| !item.crosses_flex && item.item.tracks.len() > 1)
            .map(|item| item.item.tracks.len())
            .collect();
        spans.sort_unstable();
        spans.dedup();
        for span in spans {
            let group: Vec<usize> = (0..self.items.len())
                .filter(|&index| {
                    let item = &self.items[index];
                    !item.crosses_flex && item.item.tracks.len() == span
                })
                .collect();
            self.accommodate(&group, false);
        }
        let flexible: Vec<usize> = (0..self.items.len())
            .filter(|&index| self.items[index].crosses_flex)
            .collect();
        self.accommodate(&flexible, true);
        for track in &mut self.tracks {
            if track.limit.is_infinite() {
                track.limit = track.base;
            }
        }
    }

    /// Step 2: tracks sized by the items spanning only them.
    fn size_single_span(&mut self) {
        for index in 0..self.tracks.len() {
            let track = self.tracks[index];
            if track.max.flex().is_some() || !(track.min.intrinsic() || track.max.intrinsic()) {
                continue;
            }
            let items: Vec<&Prepared<'_>> = self
                .items
                .iter()
                .filter(|item| item.item.tracks == (index..index + 1))
                .collect();
            if items.is_empty() {
                continue;
            }
            let largest = |value: &dyn Fn(&Prepared<'_>) -> f64| {
                items.iter().map(|item| value(item)).fold(0.0, f64::max)
            };
            let base = match track.min {
                MinSizing::Fixed(_) => track.base,
                MinSizing::MinContent => largest(&|item| f64::from(item.item.min_content)),
                MinSizing::MaxContent => largest(&|item| f64::from(item.item.max_content)),
                MinSizing::Auto => match self.space {
                    AvailableSpace::MinContent => {
                        largest(&|item| item.limited(item.item.min_content))
                    }
                    AvailableSpace::MaxContent => {
                        largest(&|item| item.limited(item.item.max_content))
                    }
                    AvailableSpace::Definite(_) => largest(&|item| item.minimum),
                },
            };
            let max_content = largest(&|item| f64::from(item.item.max_content));
            let limit = match track.max {
                MaxSizing::MinContent => largest(&|item| f64::from(item.item.min_content)),
                MaxSizing::MaxContent | MaxSizing::Auto => max_content,
                MaxSizing::FitContent(limit) => max_content.min(f64::from(limit)),
                MaxSizing::Fixed(_) | MaxSizing::Flex(_) => track.limit,
            };
            let track = &mut self.tracks[index];
            track.base = track.base.max(base);
            track.limit = limit.max(track.base);
        }
    }

    /// Steps 3 and 4 for one group of items; `flexible` distributes only to flexible tracks,
    /// in proportion to their flex factors.
    fn accommodate(&mut self, group: &[usize], flexible: bool) {
        if group.is_empty() {
            return;
        }
        let eligible = |track: &Track| !flexible || track.max.flex().is_some();
        let contributions = |sizer: &Self, value: &dyn Fn(&Sizer<'_>, &Prepared<'_>) -> f64| {
            group
                .iter()
                .map(|&index| (index, value(sizer, &sizer.items[index])))
                .collect::<Vec<_>>()
        };
        let minimums = contributions(self, &|sizer, item| sizer.intrinsic_minimum(item));
        self.distribute(
            &minimums,
            &|track| eligible(track) && track.min.intrinsic(),
            Affected::Base,
            Accommodating::MinContent,
            flexible,
        );
        let min_content = contributions(self, &|_, item| f64::from(item.item.min_content));
        self.distribute(
            &min_content,
            &|track| eligible(track) && track.min.content(),
            Affected::Base,
            Accommodating::MinContent,
            flexible,
        );
        if self.space == AvailableSpace::MaxContent {
            let limited = contributions(self, &|_, item| item.limited(item.item.max_content));
            self.distribute(
                &limited,
                &|track| {
                    eligible(track) && matches!(track.min, MinSizing::Auto | MinSizing::MaxContent)
                },
                Affected::Base,
                Accommodating::MaxContent,
                flexible,
            );
        }
        let max_content = contributions(self, &|_, item| f64::from(item.item.max_content));
        self.distribute(
            &max_content,
            &|track| eligible(track) && track.min == MinSizing::MaxContent,
            Affected::Base,
            Accommodating::MaxContent,
            flexible,
        );
        for track in &mut self.tracks {
            track.limit = track.limit.max(track.base);
        }
        if flexible {
            return;
        }
        let infinite: Vec<bool> = self
            .tracks
            .iter()
            .map(|track| track.limit.is_infinite())
            .collect();
        self.distribute(
            &min_content,
            &|track| track.max.intrinsic(),
            Affected::Limit,
            Accommodating::Limits,
            false,
        );
        for (track, was_infinite) in self.tracks.iter_mut().zip(infinite) {
            track.infinitely_growable = was_infinite && track.limit.is_finite();
        }
        self.distribute(
            &max_content,
            &|track| track.max.max_content(),
            Affected::Limit,
            Accommodating::Limits,
            false,
        );
        for track in &mut self.tracks {
            track.infinitely_growable = false;
        }
    }

    /// CSS Grid 1 §11.5.1: increases the affected sizes of `affected` tracks to accommodate
    /// each item's contribution.
    fn distribute(
        &mut self,
        contributions: &[(usize, f64)],
        affected: &dyn Fn(&Track) -> bool,
        size: Affected,
        accommodating: Accommodating,
        flexible: bool,
    ) {
        let mut planned: Vec<Option<f64>> = vec![None; self.tracks.len()];
        for &(index, contribution) in contributions {
            let range = self.items[index].item.tracks.clone();
            let current = |track: &Track| match size {
                Affected::Base => track.base,
                Affected::Limit => track.limit_or_base(),
            };
            let spanned: f64 = self.tracks[range.clone()].iter().map(current).sum::<f64>()
                + self.gap * (range.len() - 1) as f64;
            let mut space = (contribution - spanned).max(0.0);
            let targets: Vec<usize> = range
                .filter(|&track| affected(&self.tracks[track]))
                .collect();
            if targets.is_empty() {
                continue;
            }
            let mut incurred = vec![0.0; targets.len()];
            let weights = self.weights(&targets, flexible);
            let limits: Vec<f64> = targets
                .iter()
                .map(|&index| {
                    let track = &self.tracks[index];
                    let limit = match size {
                        Affected::Base => {
                            let fit = match track.max {
                                MaxSizing::FitContent(limit) => f64::from(limit),
                                _ => f64::INFINITY,
                            };
                            track.limit.min(fit)
                        }
                        Affected::Limit if track.infinitely_growable => f64::INFINITY,
                        Affected::Limit => track.limit,
                    };
                    (limit - current(track)).max(0.0)
                })
                .collect();
            space = fill(&mut incurred, &weights, &limits, space);
            if space > EPSILON {
                let preferred: Vec<bool> = targets
                    .iter()
                    .map(|&index| {
                        let max = self.tracks[index].max;
                        match accommodating {
                            Accommodating::MinContent => max.intrinsic(),
                            Accommodating::MaxContent => max.max_content(),
                            Accommodating::Limits => true,
                        }
                    })
                    .collect();
                let any_preferred = preferred.contains(&true);
                // A fit-content() maximum is max-content only up to its argument.
                let beyond: Vec<f64> = targets
                    .iter()
                    .zip(&preferred)
                    .zip(&incurred)
                    .map(|((&index, &preferred), &incurred)| {
                        let track = &self.tracks[index];
                        match track.max {
                            _ if any_preferred && !preferred => 0.0,
                            MaxSizing::FitContent(limit)
                                if accommodating != Accommodating::MinContent =>
                            {
                                (f64::from(limit) - current(track) - incurred).max(0.0)
                            }
                            _ => f64::INFINITY,
                        }
                    })
                    .collect();
                space = fill(&mut incurred, &weights, &beyond, space);
                if space > EPSILON {
                    let unlimited = vec![f64::INFINITY; targets.len()];
                    let chosen: Vec<f64> = weights
                        .iter()
                        .zip(&preferred)
                        .map(|(weight, &preferred)| {
                            if any_preferred && !preferred {
                                0.0
                            } else {
                                *weight
                            }
                        })
                        .collect();
                    fill(&mut incurred, &chosen, &unlimited, space);
                }
            }
            for (&track, increase) in targets.iter().zip(incurred) {
                let planned = planned[track].get_or_insert(0.0);
                *planned = planned.max(increase);
            }
        }
        for (track, planned) in self.tracks.iter_mut().zip(planned) {
            let Some(planned) = planned else { continue };
            match size {
                Affected::Base => track.base += planned,
                Affected::Limit if track.limit.is_infinite() => track.limit = track.base + planned,
                Affected::Limit => track.limit += planned,
            }
        }
    }

    /// Equal shares, or for flexible tracks shares by flex factor: wholly by ratio when the
    /// factors sum to one or more, and otherwise only that fraction of the space.
    fn weights(&self, targets: &[usize], flexible: bool) -> Vec<f64> {
        let count = targets.len() as f64;
        if !flexible {
            return vec![1.0 / count; targets.len()];
        }
        let factors: Vec<f64> = targets
            .iter()
            .map(|&index| self.tracks[index].max.flex().unwrap_or(0.0))
            .collect();
        let sum: f64 = factors.iter().sum();
        if sum >= 1.0 {
            factors.iter().map(|factor| factor / sum).collect()
        } else {
            factors
                .iter()
                .map(|factor| factor + (1.0 - sum) / count)
                .collect()
        }
    }

    fn total(&self) -> f64 {
        self.tracks.iter().map(|track| track.base).sum::<f64>()
            + self.gap * self.tracks.len().saturating_sub(1) as f64
    }

    /// CSS Grid 1 §11.6.
    fn maximize(&mut self, bounds: SizeBounds) {
        let free = match self.space {
            AvailableSpace::Definite(space) => f64::from(space) - self.total(),
            AvailableSpace::MaxContent => f64::INFINITY,
            AvailableSpace::MinContent => 0.0,
        };
        let initial: Vec<f64> = self.tracks.iter().map(|track| track.base).collect();
        self.grow_bases(free);
        if self.space == AvailableSpace::MaxContent
            && let Some(max) = bounds.max
            && self.total() > f64::from(max)
        {
            for (track, base) in self.tracks.iter_mut().zip(initial) {
                track.base = base;
            }
            let free = f64::from(max) - self.total();
            self.grow_bases(free);
        }
    }

    fn grow_bases(&mut self, free: f64) {
        if free <= 0.0 {
            return;
        }
        let rooms: Vec<f64> = self
            .tracks
            .iter()
            .map(|track| (track.limit - track.base).max(0.0))
            .collect();
        let weights = vec![1.0; self.tracks.len()];
        let mut increases = vec![0.0; self.tracks.len()];
        if free.is_infinite() {
            increases = rooms;
        } else {
            fill(&mut increases, &weights, &rooms, free);
        }
        for (track, increase) in self.tracks.iter_mut().zip(increases) {
            track.base += increase;
        }
    }

    /// CSS Grid 1 §11.7.1: the size of `1fr` filling `space` with the tracks in `range`.
    fn fr_size(&self, range: Range<usize>, space: f64) -> f64 {
        let gaps = self.gap * range.len().saturating_sub(1) as f64;
        let mut inflexible = vec![false; self.tracks.len()];
        loop {
            let mut leftover = space - gaps;
            let mut factors = 0.0;
            for index in range.clone() {
                let track = &self.tracks[index];
                match track.max.flex() {
                    Some(factor) if !inflexible[index] => factors += factor,
                    _ => leftover -= track.base,
                }
            }
            let fraction = leftover / factors.max(1.0);
            let mut changed = false;
            for index in range.clone() {
                let track = &self.tracks[index];
                if let Some(factor) = track.max.flex()
                    && !inflexible[index]
                    && fraction * factor < track.base
                {
                    inflexible[index] = true;
                    changed = true;
                }
            }
            if !changed {
                return fraction.max(0.0);
            }
        }
    }

    /// CSS Grid 1 §11.7.
    fn expand_flexible(&mut self, bounds: SizeBounds) {
        if !self.tracks.iter().any(|track| track.max.flex().is_some()) {
            return;
        }
        let all = 0..self.tracks.len();
        let fraction = match self.space {
            AvailableSpace::MinContent => return,
            AvailableSpace::Definite(space) => {
                if f64::from(space) - self.total() <= 0.0 {
                    return;
                }
                self.fr_size(all, f64::from(space))
            }
            AvailableSpace::MaxContent => {
                let tracks = self.tracks.iter().filter_map(|track| {
                    track.max.flex().map(|factor| {
                        if factor > 1.0 {
                            track.base / factor
                        } else {
                            track.base
                        }
                    })
                });
                let items = self
                    .items
                    .iter()
                    .filter(|item| item.crosses_flex)
                    .map(|item| {
                        self.fr_size(item.item.tracks.clone(), f64::from(item.item.max_content))
                    });
                let fraction = tracks.chain(items).fold(0.0, f64::max);
                let size = self.flexed_total(fraction);
                if size < f64::from(bounds.min) {
                    self.fr_size(all, f64::from(bounds.min))
                } else if let Some(max) = bounds.max.filter(|max| size > f64::from(*max)) {
                    self.fr_size(all, f64::from(max))
                } else {
                    fraction
                }
            }
        };
        for track in &mut self.tracks {
            if let Some(factor) = track.max.flex() {
                track.base = track.base.max(fraction * factor);
            }
        }
    }

    fn flexed_total(&self, fraction: f64) -> f64 {
        self.tracks
            .iter()
            .map(|track| {
                track
                    .max
                    .flex()
                    .map_or(track.base, |factor| track.base.max(fraction * factor))
            })
            .sum::<f64>()
            + self.gap * self.tracks.len().saturating_sub(1) as f64
    }

    /// CSS Grid 1 §11.8: `auto` maximums share the remaining definite free space.
    fn stretch_auto(&mut self, bounds: SizeBounds) {
        let space = match self.space {
            AvailableSpace::Definite(space) => f64::from(space),
            AvailableSpace::MaxContent if bounds.min > 0.0 => f64::from(bounds.min),
            _ => return,
        };
        let free = space - self.total();
        let count = self
            .tracks
            .iter()
            .filter(|track| track.max == MaxSizing::Auto)
            .count();
        if free <= 0.0 || count == 0 {
            return;
        }
        for track in &mut self.tracks {
            if track.max == MaxSizing::Auto {
                track.base += free / count as f64;
            }
        }
    }
}

/// Distributes `space` to `increases` by `weights`, freezing each as it reaches its `rooms`
/// entry; returns the space left once all are frozen.
fn fill(increases: &mut [f64], weights: &[f64], rooms: &[f64], mut space: f64) -> f64 {
    let mut frozen: Vec<bool> = weights
        .iter()
        .zip(rooms)
        .map(|(weight, room)| *weight <= 0.0 || *room <= EPSILON)
        .collect();
    let mut remaining: Vec<f64> = rooms.to_vec();
    while space > EPSILON && frozen.contains(&false) {
        let total: f64 = weights
            .iter()
            .zip(&frozen)
            .filter(|(_, frozen)| !**frozen)
            .map(|(weight, _)| weight)
            .sum();
        let step = weights
            .iter()
            .zip(&remaining)
            .zip(&frozen)
            .filter(|(_, frozen)| !**frozen)
            .map(|((weight, room), _)| room * total / weight)
            .fold(space, f64::min);
        for index in 0..increases.len() {
            if frozen[index] {
                continue;
            }
            let share = step * weights[index] / total;
            increases[index] += share;
            remaining[index] -= share;
            if remaining[index] <= EPSILON {
                frozen[index] = true;
            }
        }
        space -= step;
    }
    space.max(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(min: MinSizing, max: MaxSizing) -> TrackSizing {
        TrackSizing { min, max }
    }
    fn item(tracks: Range<usize>, min_content: f32, max_content: f32) -> SizingItem {
        SizingItem {
            tracks,
            min_content,
            max_content,
            minimum: 0.0,
            content_minimum: Some(min_content),
        }
    }
    fn size(tracks: &[TrackSizing], items: &[SizingItem], space: AvailableSpace) -> Vec<f32> {
        size_tracks(tracks, 0.0, items, space, SizeBounds::default(), true).unwrap()
    }

    #[test]
    fn spanning_items_fill_fixed_maximums_before_growing_intrinsic_tracks() {
        let tracks = [
            track(MinSizing::Auto, MaxSizing::Fixed(40.0)),
            track(MinSizing::Auto, MaxSizing::Auto),
        ];
        let items = [item(0..1, 10.0, 20.0), item(0..2, 100.0, 120.0)];
        assert_eq!(
            size(&tracks, &items, AvailableSpace::MinContent),
            [40.0, 60.0],
            "the spanning item's minimum fills the first track to its 40pt limit and the \
             rest goes to the auto track"
        );
        assert_eq!(
            size(&tracks, &items, AvailableSpace::MaxContent),
            [40.0, 80.0],
            "the auto track's growth limit absorbs the extra max-content contribution"
        );
        assert_eq!(
            size(&tracks, &items, AvailableSpace::Definite(200.0)),
            [40.0, 160.0],
            "the auto track stretches into the definite free space"
        );
    }

    #[test]
    fn flexible_tracks_respect_min_content_minimums() {
        let fr = |factor| track(MinSizing::Auto, MaxSizing::Flex(factor));
        let items = [item(0..1, 60.0, 60.0), item(1..2, 10.0, 10.0)];
        assert_eq!(
            size(&[fr(1.0), fr(2.0)], &items, AvailableSpace::Definite(150.0)),
            [60.0, 90.0],
            "1fr would be 50pt, below the 60pt minimum, so that track becomes inflexible"
        );
        let zero_minimum = track(MinSizing::Fixed(0.0), MaxSizing::Flex(1.0));
        assert_eq!(
            size(
                &[zero_minimum, fr(2.0)],
                &items,
                AvailableSpace::Definite(150.0)
            ),
            [50.0, 100.0],
            "minmax(0, 1fr) ignores the item's minimum"
        );
        assert_eq!(
            size(&[fr(1.0), fr(2.0)], &items, AvailableSpace::MaxContent),
            [60.0, 120.0],
            "with indefinite space the largest item-implied fraction sizes every track"
        );
        assert_eq!(
            size(&[fr(1.0), fr(2.0)], &items, AvailableSpace::MinContent),
            [60.0, 10.0],
            "under a min-content constraint the flex fraction is zero"
        );
    }

    #[test]
    fn automatic_minimum_is_zero_only_for_multi_track_flexible_spans() {
        let flexible = track(MinSizing::Auto, MaxSizing::Flex(1.0));
        let automatic = track(MinSizing::Auto, MaxSizing::Auto);
        for (tracks, expected) in [
            (vec![flexible], vec![60.0]),
            (vec![flexible, flexible], vec![20.0, 20.0]),
            (vec![automatic, flexible], vec![0.0, 40.0]),
            (vec![automatic, automatic], vec![30.0, 30.0]),
        ] {
            let items = [item(0..tracks.len(), 60.0, 60.0)];
            assert_eq!(
                size(&tracks, &items, AvailableSpace::Definite(40.0)),
                expected,
                "automatic minimum eligibility for {tracks:?}"
            );
        }
        let mut explicit = item(0..2, 60.0, 60.0);
        explicit.minimum = 20.0;
        assert_eq!(
            size(
                &[flexible, flexible],
                &[explicit],
                AvailableSpace::Definite(10.0)
            ),
            [10.0, 10.0],
            "the exception must not discard an explicit minimum contribution"
        );
    }

    #[test]
    fn items_spanning_flexible_tracks_distribute_by_flex_factor() {
        let tracks = [
            track(MinSizing::Auto, MaxSizing::Flex(1.0)),
            track(MinSizing::Auto, MaxSizing::Flex(3.0)),
            track(MinSizing::Auto, MaxSizing::Auto),
        ];
        let items = [item(0..3, 80.0, 80.0)];
        assert_eq!(
            size(&tracks, &items, AvailableSpace::MinContent),
            [20.0, 60.0, 0.0],
            "the item crosses flexible tracks, so only they grow, in a 1:3 ratio"
        );
    }

    #[test]
    fn fit_content_clamps_to_its_argument_but_not_below_min_content() {
        let tracks = [
            track(MinSizing::Auto, MaxSizing::FitContent(40.0)),
            track(MinSizing::Auto, MaxSizing::FitContent(40.0)),
        ];
        let items = [item(0..1, 20.0, 100.0), item(1..2, 84.5, 90.0)];
        assert_eq!(
            size(&tracks, &items, AvailableSpace::Definite(300.0)),
            [40.0, 84.5],
            "fit-content(40pt) caps max-content at 40pt; a wider min-content wins"
        );
    }

    #[test]
    fn maximize_respects_the_containers_maximum_size() {
        let tracks = [track(MinSizing::Fixed(10.0), MaxSizing::Auto); 2];
        let items = [item(0..1, 10.0, 100.0), item(1..2, 10.0, 100.0)];
        let sizes = size_tracks(
            &tracks,
            5.0,
            &items,
            AvailableSpace::MaxContent,
            SizeBounds {
                min: 0.0,
                max: Some(105.0),
            },
            true,
        )
        .unwrap();
        assert_eq!(
            sizes,
            [50.0, 50.0],
            "tracks grow from their 10pt bases only into max-height minus the gap"
        );
    }
}
