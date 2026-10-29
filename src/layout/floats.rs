use crate::graphics::Rect;
use crate::layout::{
    LayoutError,
    float_area::{FloatArea, meets},
    work::Budget,
};
use crate::style::{Clear, FloatSide};
use std::sync::Arc;

/// Line box positions found by bisection lie within this distance of the first that fits.
const FIT_TOLERANCE_PT: f32 = 0.01;
/// The golden ratio's reciprocal, which shrinks a golden-section search interval each step.
const GOLDEN_SECTION: f32 = 0.618_034;

#[derive(Clone)]
struct Exclusion {
    page: usize,
    side: FloatSide,
    margin: Rect,
    /// An initial letter's, which is in-flow content rather than a float (CSS Inline 3
    /// §7.9.3).
    letter: bool,
    /// The float area line boxes wrap around; the margin box when `None`.
    area: Option<Arc<FloatArea>>,
}

impl Exclusion {
    fn bottom(&self) -> f32 {
        self.area
            .as_ref()
            .map_or(self.margin.y + self.margin.height, |area| area.bottom())
    }
}

/// One page's part of a float: its margin box and the float area it defines there.
pub(crate) struct FloatFragment {
    pub page: usize,
    pub margin: Rect,
    pub area: Option<FloatArea>,
}

#[derive(Clone, Default)]
pub(super) struct FloatContext {
    exclusions: Vec<Exclusion>,
    last_start: Option<(usize, f32)>,
}

impl FloatContext {
    pub(crate) fn insert(&mut self, side: FloatSide, fragments: Vec<FloatFragment>) {
        self.last_start = fragments
            .first()
            .map(|fragment| (fragment.page, fragment.margin.y));
        self.exclusions
            .extend(fragments.into_iter().map(|fragment| Exclusion {
                page: fragment.page,
                side,
                margin: fragment.margin,
                letter: false,
                area: fragment.area.map(Arc::new),
            }));
    }

    /// Excludes the lines after an initial letter's originating line that its margin box
    /// `margin` reaches on `page` at their `side` start (CSS Inline 3 §7.5).
    pub(crate) fn insert_letter(&mut self, page: usize, side: FloatSide, margin: Rect) {
        self.exclusions.push(Exclusion {
            page,
            side,
            margin,
            letter: true,
            area: None,
        });
    }

    /// The bottom of the initial letters on `page`, which a block that starts with an
    /// initial letter or establishes an independent formatting context clears (§7.9.2).
    pub(crate) fn letter_clearance(
        &self,
        page: usize,
        y: f32,
        work: &Budget,
    ) -> Result<f32, LayoutError> {
        let mut bottom = y;
        for exclusion in &self.exclusions {
            work.spend(1)?;
            if exclusion.letter && exclusion.page == page {
                bottom = bottom.max(exclusion.margin.y + exclusion.margin.height);
            }
        }
        Ok(bottom)
    }

    pub(crate) fn last_start(&self) -> Option<(usize, f32)> {
        self.last_start
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.exclusions.is_empty()
    }

    pub(crate) fn occupies_page(
        &self,
        page: usize,
        height: f32,
        work: &Budget,
    ) -> Result<bool, LayoutError> {
        for float in &self.exclusions {
            work.spend(1)?;
            if float.page == page
                && float.margin.height > 0.0
                && float.margin.y < height
                && float.margin.y + float.margin.height > 0.0
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub(crate) fn end(&self, work: &Budget) -> Result<Option<(usize, f32)>, LayoutError> {
        self.clearance_end(0, Clear::Both, false, work)
    }

    pub(crate) fn clearance_end(
        &self,
        from_page: usize,
        clear: Clear,
        rtl: bool,
        work: &Budget,
    ) -> Result<Option<(usize, f32)>, LayoutError> {
        if clear == Clear::None {
            return Ok(None);
        }
        let mut end: Option<(usize, f32)> = None;
        for float in &self.exclusions {
            work.spend(1)?;
            if float.page < from_page || !clear.clears(float.side, rtl) {
                continue;
            }
            let bottom = float.margin.y + float.margin.height;
            if end.is_none_or(|(page, y)| float.page > page || (float.page == page && bottom > y)) {
                end = Some((float.page, bottom));
            }
        }
        Ok(end)
    }

    pub(crate) fn band(
        &self,
        page: usize,
        y: f32,
        height: f32,
        bounds: [f32; 2],
        work: &Budget,
    ) -> Result<([f32; 2], Option<f32>), LayoutError> {
        let mut band = bounds;
        let mut next: Option<f32> = None;
        for float in &self.exclusions {
            work.spend(1)?;
            let rect = float.margin;
            if float.page != page
                || rect.height <= 0.0
                || !meets(rect.y, rect.y + rect.height, y, height)
            {
                continue;
            }
            match float.side {
                FloatSide::Left => band[0] = band[0].max(rect.x + rect.width),
                FloatSide::Right => band[1] = band[1].min(rect.x),
            }
            let bottom = rect.y + rect.height;
            next = Some(next.map_or(bottom, |value| value.min(bottom)));
        }
        Ok((band, next))
    }

    /// The edges left to a line box at `y` of `height` within `bounds` beside the float areas
    /// it meets (CSS Shapes 1 §1.3), and whether it meets any.
    pub(crate) fn line_band(
        &self,
        page: usize,
        y: f32,
        height: f32,
        bounds: [f32; 2],
        work: &Budget,
    ) -> Result<([f32; 2], bool), LayoutError> {
        let mut band = bounds;
        let mut met = false;
        for float in &self.exclusions {
            work.spend(1)?;
            if float.page != page {
                continue;
            }
            let rect = float.margin;
            let edge = if let Some(area) = &float.area {
                area.edge(y, height, float.side, work)?
            } else {
                (rect.height > 0.0 && meets(rect.y, rect.y + rect.height, y, height)).then_some(
                    match float.side {
                        FloatSide::Left => rect.x + rect.width,
                        FloatSide::Right => rect.x,
                    },
                )
            };
            let Some(edge) = edge else {
                continue;
            };
            met = true;
            match float.side {
                FloatSide::Left => band[0] = band[0].max(edge),
                FloatSide::Right => band[1] = band[1].min(edge),
            }
        }
        Ok((band, met))
    }

    /// CSS 2.1 §9.5: the first position below `y` where a line box of `height` that needs
    /// `width` fits beside the float areas, or where it meets none. Between the positions
    /// where a float area's vertex enters or leaves the line box, each area's edge is the
    /// outermost of linearly moving points, so the band's width is concave there and its
    /// first fit is found by bisection.
    pub(crate) fn line_fit(
        &self,
        page: usize,
        y: f32,
        height: f32,
        bounds: [f32; 2],
        width: f32,
        work: &Budget,
    ) -> Result<f32, LayoutError> {
        let mut breakpoints = Vec::new();
        let mut end = y;
        for float in self.exclusions.iter().filter(|float| float.page == page) {
            work.spend(1)?;
            if let Some(area) = &float.area {
                area.breakpoints(height, &mut breakpoints);
            } else {
                let rect = float.margin;
                breakpoints.extend([rect.y - height, rect.y + rect.height]);
            }
            end = end.max(float.bottom());
        }
        breakpoints.retain(|position| *position > y && *position < end);
        breakpoints.push(end);
        breakpoints.sort_by(f32::total_cmp);
        breakpoints.dedup();
        work.spend(breakpoints.len())?;
        let slack = |position: f32| -> Result<f32, LayoutError> {
            let (band, _) = self.line_band(page, position, height, bounds, work)?;
            Ok(band[1] - band[0] - width)
        };
        let mut start = y;
        for stop in breakpoints {
            if let Some(fit) = first_fit([start, stop], &slack)? {
                return Ok(fit);
            }
            start = stop;
        }
        Ok(end)
    }

    pub(crate) fn clearance(
        &self,
        page: usize,
        y: f32,
        clear: Clear,
        rtl: bool,
        work: &Budget,
    ) -> Result<f32, LayoutError> {
        if clear == Clear::None {
            return Ok(y);
        }
        let mut bottom = y;
        for float in &self.exclusions {
            work.spend(1)?;
            if float.page == page && clear.clears(float.side, rtl) {
                bottom = bottom.max(float.margin.y + float.margin.height);
            }
        }
        Ok(bottom)
    }

    /// Where a float on `side` styled `clear` may start at or below `y` on `page`. `clear`
    /// does not see initial letters, but a float on their inline-start side placed beside
    /// one clears it (CSS Inline 3 §7.9.3).
    pub(crate) fn float_clearance(
        &self,
        page: usize,
        y: f32,
        clear: Clear,
        side: FloatSide,
        rtl: bool,
        work: &Budget,
    ) -> Result<f32, LayoutError> {
        let mut bottom = y;
        for exclusion in &self.exclusions {
            work.spend(1)?;
            let clears = if exclusion.letter {
                exclusion.side == side
            } else {
                clear.clears(exclusion.side, rtl)
            };
            if exclusion.page == page && clears {
                bottom = bottom.max(exclusion.margin.y + exclusion.margin.height);
            }
        }
        Ok(bottom)
    }

    pub(crate) fn place(
        &self,
        page: usize,
        source_y: f32,
        bounds: [f32; 2],
        size: [f32; 2],
        side: FloatSide,
        work: &Budget,
    ) -> Result<Rect, LayoutError> {
        let mut y = source_y;
        for float in &self.exclusions {
            work.spend(1)?;
            if float.page == page && !float.letter {
                y = y.max(float.margin.y);
            }
        }
        loop {
            work.spend(1)?;
            // A zero-height float still obeys placement constraints at its top edge.
            let probe_height = size[1].max(0.0);
            let (band, next) = self.band(page, y, probe_height, bounds, work)?;
            if size[0] <= band[1] - band[0] || next.is_none() {
                let x = match side {
                    FloatSide::Left => band[0],
                    FloatSide::Right => band[1] - size[0],
                };
                return Ok(Rect {
                    x,
                    y,
                    width: size[0],
                    height: size[1],
                });
            }
            y = next.expect("an obstructing float has a bottom edge");
        }
    }
}

/// The first position in `(start, stop]` where `slack` is not negative, given that it is
/// negative at `start` and concave strictly between the two.
fn first_fit(
    [start, stop]: [f32; 2],
    slack: &impl Fn(f32) -> Result<f32, LayoutError>,
) -> Result<Option<f32>, LayoutError> {
    let step = FIT_TOLERANCE_PT.min((stop - start) / 2.0);
    let inner = stop - step;
    if inner > start && slack(inner)? >= 0.0 {
        return bisect([start, inner], slack).map(Some);
    }
    if slack(stop)? >= 0.0 {
        return Ok(Some(stop));
    }
    let near_start = start + step;
    // A concave slack rising after `start` and falling before `stop` peaks between them.
    if inner - near_start <= step
        || slack(near_start + step)? <= slack(near_start)?
        || slack(inner)? >= slack(inner - step)?
    {
        return Ok(None);
    }
    let [mut low, mut high] = [near_start, inner];
    while high - low > FIT_TOLERANCE_PT {
        let left = high - (high - low) * GOLDEN_SECTION;
        let right = low + (high - low) * GOLDEN_SECTION;
        if slack(left)? < slack(right)? {
            low = left;
        } else {
            high = right;
        }
    }
    let peak = f32::midpoint(low, high);
    if slack(peak)? >= 0.0 {
        bisect([start, peak], slack).map(Some)
    } else {
        Ok(None)
    }
}

/// Narrows `[low, high]`, where `slack` turns from negative to not negative, to its crossing.
fn bisect(
    [mut low, mut high]: [f32; 2],
    slack: &impl Fn(f32) -> Result<f32, LayoutError>,
) -> Result<f32, LayoutError> {
    while high - low > FIT_TOLERANCE_PT {
        let middle = f32::midpoint(low, high);
        if slack(middle)? >= 0.0 {
            high = middle;
        } else {
            low = middle;
        }
    }
    Ok(high)
}
