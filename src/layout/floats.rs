use crate::graphics::Rect;
use crate::layout::{LayoutError, work::Budget};
use crate::style::{Clear, FloatSide};

#[derive(Clone)]
struct Exclusion {
    page: usize,
    side: FloatSide,
    margin: Rect,
    /// An initial letter's, which is in-flow content rather than a float (CSS Inline 3
    /// §7.9.3).
    letter: bool,
}

#[derive(Clone, Default)]
pub(super) struct FloatContext {
    exclusions: Vec<Exclusion>,
    last_start: Option<(usize, f32)>,
}

impl FloatContext {
    pub(crate) fn insert(&mut self, side: FloatSide, fragments: Vec<(usize, Rect)>) {
        self.last_start = fragments.first().map(|(page, rect)| (*page, rect.y));
        self.exclusions
            .extend(fragments.into_iter().map(|(page, margin)| Exclusion {
                page,
                side,
                margin,
                letter: false,
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
                || rect.y > y + height
                || (height > 0.0 && rect.y == y + height)
                || rect.y + rect.height <= y
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
