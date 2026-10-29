//! CSS Text 4 §5.4 `text-wrap-style`: `balance` evens out the remaining space of the lines
//! between forced breaks, and `pretty` avoids an excessively short last line. Both keep the
//! first-fit line count and choose among the same soft wrap opportunities; content that only
//! an emergency break or overflow can place keeps its first-fit lines.
use super::{ParagraphLines, indent_at};
use crate::style::TextWrapStyle;
use crate::text::{TextError, hyphenation};
use std::{cell::RefCell, collections::HashMap, rc::Rc};
use unicode_linebreak::BreakOpportunity;

/// "UAs may treat this value as auto if there are more than ten lines to balance."
const MAX_BALANCED_LINES: usize = 10;
/// The lines at the end of a paragraph that `pretty` may rewrap.
const PRETTY_LINES: usize = 4;
/// Under `pretty`, a line narrower than this fraction of its available width is excessively
/// short: the last line is lengthened to at least this, and no earlier line drops below it.
const SHORT_LINE_FRACTION: f32 = 1.0 / 3.0;

/// A group start and the fitting width, containing width and inline offset bits.
type PlanKey = (usize, u32, u32, u32);

/// Line ends planned for each group of lines; `None` keeps first-fit breaking.
#[derive(Default)]
pub(super) struct Plans(RefCell<HashMap<PlanKey, Option<Rc<[usize]>>>>);

/// A first-fit line: where it ends and how much of its available width it leaves.
#[derive(Clone, Copy)]
struct Fitted {
    end: usize,
    remaining: f32,
    available: f32,
}

/// A candidate line between two soft wrap opportunities.
#[derive(Clone, Copy)]
enum Segment {
    Overflows,
    /// A line holding only a leader cannot end at an unforced break.
    Excluded,
    Fits {
        remaining: f32,
        available: f32,
    },
}

/// What a rewrapping minimises and requires.
#[derive(Clone, Copy)]
struct Objective {
    /// Whether the last line's remaining space counts towards the cost.
    last_counts: bool,
    /// The smallest fraction of its available width each line must fill.
    minimum_fill: Option<f32>,
}

impl ParagraphLines<'_, '_, '_, '_> {
    /// The planned end of the line from `start`, and whether it is the group's last line.
    pub(super) fn planned_end(&self, start: usize) -> Result<Option<(usize, bool)>, TextError> {
        let prepared = self.prepared;
        let style = prepared.paragraph.style();
        let wrap = style.line_breaking.wrap_style;
        if !matches!(wrap, TextWrapStyle::Balance | TextWrapStyle::Pretty)
            || !style.white_space.wraps()
        {
            return Ok(None);
        }
        let opportunities = &prepared.opportunities;
        let group_start = opportunities[..opportunities.partition_point(|&(end, _)| end <= start)]
            .iter()
            .rev()
            .find(|(_, opportunity)| *opportunity == BreakOpportunity::Mandatory)
            .map_or(0, |&(end, _)| end);
        let key = (
            group_start,
            self.width.to_bits(),
            self.containing_width.to_bits(),
            self.inline_start.to_bits(),
        );
        let cached = prepared.plans.0.borrow().get(&key).cloned();
        let plan = if let Some(plan) = cached {
            plan
        } else if start == group_start {
            let plan = self.plan(group_start, wrap)?.map(Rc::from);
            prepared.plans.0.borrow_mut().insert(key, plan.clone());
            plan
        } else {
            return Ok(None);
        };
        let Some(plan) = plan else {
            return Ok(None);
        };
        if start != group_start && plan.binary_search(&start).is_err() {
            return Ok(None);
        }
        Ok(plan.iter().find(|&&end| end > start).map(|&end| {
            let forced = opportunities
                .binary_search_by_key(&end, |&(end, _)| end)
                .is_ok_and(|index| opportunities[index].1 == BreakOpportunity::Mandatory);
            (end, forced)
        }))
    }

    /// Line ends for the group of lines from `group_start`, which the current position starts.
    /// A clamped paragraph balances only the lines before its region break.
    fn plan(
        &self,
        group_start: usize,
        wrap: TextWrapStyle,
    ) -> Result<Option<Vec<usize>>, TextError> {
        let prepared = self.prepared;
        let paragraph = prepared.paragraph;
        let mut fitted: Vec<Fitted> = Vec::new();
        let mut hyphenated = false;
        let mut start = group_start;
        loop {
            let indent = indent_at(
                paragraph,
                &prepared.opportunities,
                start,
                self.containing_width,
            );
            let Some((line, last)) = self.greedy(start, indent)? else {
                return Ok(None);
            };
            let available = self.width - indent;
            let end = line.range.end;
            if line.wrap_width > available
                || prepared
                    .opportunities
                    .binary_search_by_key(&end, |&(end, _)| end)
                    .is_err()
            {
                return Ok(None);
            }
            hyphenated |= hyphenation::is_soft_break(paragraph, &prepared.automatic, end);
            fitted.push(Fitted {
                end,
                remaining: available - line.width,
                available,
            });
            if wrap == TextWrapStyle::Balance && fitted.len() > MAX_BALANCED_LINES {
                return Ok(None);
            }
            let clamped = prepared
                .clamp
                .as_ref()
                .is_some_and(|clamp| self.position.line + fitted.len() >= clamp.lines);
            if last || clamped {
                break;
            }
            start = end;
        }
        let lines = fitted.len();
        let Some(&last) = fitted.last().filter(|_| lines > 1) else {
            return Ok(None);
        };
        if wrap == TextWrapStyle::Balance {
            return self.rewrap(
                group_start,
                last.end,
                lines,
                hyphenated,
                Objective {
                    last_counts: true,
                    minimum_fill: None,
                },
            );
        }
        let short = last.available - last.remaining < SHORT_LINE_FRACTION * last.available;
        if last.end != paragraph.text().len() || !short {
            return Ok(None);
        }
        let window = lines.min(PRETTY_LINES);
        let first = lines - window;
        let from = first
            .checked_sub(1)
            .map_or(group_start, |index| fitted[index].end);
        let Some(rewrapped) = self.rewrap(
            from,
            last.end,
            window,
            hyphenated,
            Objective {
                last_counts: false,
                minimum_fill: Some(SHORT_LINE_FRACTION),
            },
        )?
        else {
            return Ok(None);
        };
        Ok(Some(
            fitted[..first]
                .iter()
                .map(|line| line.end)
                .chain(rewrapped)
                .collect(),
        ))
    }

    /// The ends of exactly `lines` fitting lines from `from` to `to` that minimise the sum of
    /// squared remaining space. Hyphenation opportunities are candidates only where first-fit
    /// breaking already hyphenated.
    fn rewrap(
        &self,
        from: usize,
        to: usize,
        lines: usize,
        hyphenated: bool,
        objective: Objective,
    ) -> Result<Option<Vec<usize>>, TextError> {
        let prepared = self.prepared;
        let paragraph = prepared.paragraph;
        let mut nodes = vec![from];
        nodes.extend(
            prepared
                .opportunities
                .iter()
                .map(|&(end, _)| end)
                .filter(|&end| {
                    end > from
                        && end < to
                        && (hyphenated
                            || !hyphenation::is_soft_break(paragraph, &prepared.automatic, end))
                }),
        );
        nodes.push(to);
        let last = nodes.len() - 1;
        let mut best: Vec<Vec<Option<(f32, usize)>>> = vec![vec![None; nodes.len()]; lines + 1];
        best[0][0] = Some((0.0, 0));
        let mut segments: HashMap<(usize, usize), Segment> = HashMap::new();
        for count in 1..=lines {
            for end in 1..nodes.len() {
                if (count == lines) != (end == last) {
                    continue;
                }
                for start in (0..end).rev() {
                    let segment = if let Some(&segment) = segments.get(&(start, end)) {
                        segment
                    } else {
                        let segment = self.segment(nodes[start], nodes[end])?;
                        segments.insert((start, end), segment);
                        segment
                    };
                    let (remaining, available) = match segment {
                        Segment::Fits {
                            remaining,
                            available,
                        } => (remaining, available),
                        Segment::Overflows if prepared.monotone => break,
                        Segment::Overflows | Segment::Excluded => continue,
                    };
                    let Some((cost, _)) = best[count - 1][start] else {
                        continue;
                    };
                    if objective
                        .minimum_fill
                        .is_some_and(|fill| available - remaining < fill * available)
                    {
                        continue;
                    }
                    let cost = cost
                        + if end == last && !objective.last_counts {
                            0.0
                        } else {
                            remaining * remaining
                        };
                    if best[count][end].is_none_or(|(previous, _)| cost < previous) {
                        best[count][end] = Some((cost, start));
                    }
                }
            }
        }
        let mut ends = Vec::with_capacity(lines);
        let mut node = last;
        for count in (1..=lines).rev() {
            let Some((_, start)) = best[count][node] else {
                return Ok(None);
            };
            ends.push(nodes[node]);
            node = start;
        }
        ends.reverse();
        Ok(Some(ends))
    }

    fn segment(&self, from: usize, to: usize) -> Result<Segment, TextError> {
        let prepared = self.prepared;
        let indent = indent_at(
            prepared.paragraph,
            &prepared.opportunities,
            from,
            self.containing_width,
        );
        let line = self.emit(from, to, indent, Vec::new())?;
        let available = self.width - indent;
        let forced = prepared
            .opportunities
            .binary_search_by_key(&to, |&(end, _)| end)
            .is_ok_and(|index| prepared.opportunities[index].1 == BreakOpportunity::Mandatory);
        Ok(if line.wrap_width > available {
            Segment::Overflows
        } else if !forced && prepared.has_leaders && crate::text::leaders::alone(&line) {
            Segment::Excluded
        } else {
            Segment::Fits {
                remaining: available - line.width,
                available,
            }
        })
    }
}
