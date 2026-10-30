//! Footnote areas (CSS GCPM 3 §2): footnote bodies at the bottom of the page area of the page
//! holding their calls, or continued on the pages after it.
#[allow(
    clippy::wildcard_imports,
    reason = "pagination submodules extend the parent paginator and share its scope"
)]
use super::*;
use crate::style::{BoxSizing, FootnoteDisplay, FootnotePolicy};
use std::{cell::RefCell, collections::BTreeSet};

/// A footnote body laid out alone in its footnote area.
pub(super) struct FootnoteLayout {
    flow: StandaloneFlow,
    /// The width the body takes in the area: the area's, or its max-content width when
    /// `compact` lets it share a line.
    width: f32,
    height: f32,
    compact: bool,
    /// Offsets in the body between which no line box or image lies, ascending.
    breaks: Vec<f32>,
    /// The extent of each footnote of an inline body, in its order: from the top of the line
    /// it starts on to the bottom of the line the next one starts on when they share it, or
    /// else to that line's top.
    members: Vec<[f32; 2]>,
}

/// The slice `[top, bottom)` of a footnote body that one page's footnote area holds.
#[derive(Clone, Copy)]
pub(super) struct FootnotePart {
    body: usize,
    top: f32,
    bottom: f32,
    width: f32,
    /// A whole `compact` body, which shares a line with the next when both fit.
    inline: bool,
}
impl FootnotePart {
    fn height(self) -> f32 {
        self.bottom - self.top
    }
}

/// A line of the footnote area: one part, or `compact` bodies side by side.
struct Row {
    parts: Vec<(FootnotePart, f32)>,
    height: f32,
}

/// Groups parts into the footnote area's lines (CSS GCPM 3 §2.3).
fn rows(parts: &[FootnotePart], width: f32) -> Vec<Row> {
    let mut rows: Vec<Row> = Vec::new();
    for &part in parts {
        if part.inline
            && let Some(row) = rows.last_mut()
            && row.parts.iter().all(|(other, _)| other.inline)
        {
            let used: f32 = row.parts.iter().map(|(other, _)| other.width).sum();
            if !overflows(used + part.width, width) {
                row.parts.push((part, used));
                row.height = row.height.max(part.height());
                continue;
            }
        }
        rows.push(Row {
            parts: vec![(part, 0.0)],
            height: part.height(),
        });
    }
    rows
}

fn rows_height(parts: &[FootnotePart], width: f32) -> Option<f32> {
    (!parts.is_empty()).then(|| rows(parts, width).iter().map(|row| row.height).sum())
}

/// Footnote bodies by body and the bits of their area's width.
type FootnoteLayouts = HashMap<(usize, u32), Rc<FootnoteLayout>>;

/// Pages that the calls of footnotes whose policy keeps them with their calls (CSS GCPM 3
/// §2.8) cannot lie on, by footnote and page: the footnote did not fit there, and the call is
/// in a nested layout, which learns that only once its content is placed.
pub(in crate::layout) type FootnoteDeferrals = BTreeMap<(usize, usize), FootnotePolicy>;

/// The footnotes each page's area holds, the remainders waiting for the next page, and the
/// footnotes already placed or waiting.
#[derive(Clone, Default)]
pub(super) struct Footnotes {
    pages: Vec<Vec<FootnotePart>>,
    carried: VecDeque<FootnotePart>,
    placed: BTreeSet<usize>,
    /// How many recorded calls have been taken up, and those taken up whose pages the flow
    /// has not reached.
    recorded: usize,
    ahead: Vec<(usize, usize)>,
    /// Calls in nested layouts that pagination must move off their pages.
    deferred: FootnoteDeferrals,
    /// How far the content of each inline body is queued.
    queued: HashMap<usize, f32>,
    /// Where this pagination placed the inline formatting contexts of footnote areas.
    inline: crate::document::InlineFootnotePlacement,
    /// Bodies by body and area width; laying a body out does not depend on its page.
    layouts: Rc<RefCell<FootnoteLayouts>>,
}
impl Footnotes {
    fn parts(&self, page: usize) -> &[FootnotePart] {
        self.pages.get(page).map_or(&[], Vec::as_slice)
    }
    /// The last page whose footnote area holds a part.
    pub(super) fn last_page(&self) -> Option<usize> {
        self.pages.iter().rposition(|parts| !parts.is_empty())
    }
    pub(super) fn has_carried(&self) -> bool {
        !self.carried.is_empty()
    }
    /// The calls this pagination found on pages that their footnotes do not fit.
    pub(super) fn take_deferred(&mut self) -> FootnoteDeferrals {
        std::mem::take(&mut self.deferred)
    }
    /// Where this pagination placed the inline formatting contexts of footnote areas.
    pub(super) fn take_inline_placement(&mut self) -> crate::document::InlineFootnotePlacement {
        std::mem::take(&mut self.inline)
    }
}

/// The policy among `policies` that moves most: `block`, then `line`, then `auto`.
fn strictest(policies: impl Iterator<Item = FootnotePolicy>) -> FootnotePolicy {
    policies
        .max_by_key(|policy| match policy {
            FootnotePolicy::Auto => 0,
            FootnotePolicy::Line => 1,
            FootnotePolicy::Block => 2,
        })
        .unwrap_or_default()
}

/// A page's footnote area box in the page's flow coordinates.
struct Area {
    x: f32,
    width: f32,
    padding: [f32; 4],
    borders: [f32; 4],
    margins: [f32; 4],
    /// Content height bounds from `min-height` and from `height` or `max-height`.
    minimum: f32,
    maximum: Option<f32>,
}
impl Area {
    fn edges(&self, side: usize) -> f32 {
        self.padding[side] + self.borders[side] + self.margins[side]
    }
    fn content_height(&self, parts: f32, unbounded: bool) -> f32 {
        let height = parts.max(self.minimum);
        match self.maximum {
            Some(maximum) if !unbounded => height.min(maximum.max(self.minimum)),
            _ => height,
        }
    }
    /// The margin-box height around `parts` of content; an empty area is not generated.
    fn outer_height(&self, parts: Option<f32>, unbounded: bool) -> f32 {
        parts.map_or(0.0, |parts| {
            self.edges(TOP) + self.content_height(parts, unbounded) + self.edges(BOTTOM)
        })
    }
}
const TOP: usize = 0;
const RIGHT: usize = 1;
const BOTTOM: usize = 2;
const LEFT: usize = 3;

impl<'a: 't, 't: 'a> Paginator<'a, '_, 't> {
    /// The page's footnote area box and decoration in the paginator's frame: the page flow's,
    /// given as horizontal, in which the area lies at the block end of the page area.
    fn area_box(
        &self,
        page: usize,
    ) -> Option<(
        std::borrow::Cow<'_, crate::style::Style>,
        std::borrow::Cow<'_, BoxDecoration>,
    )> {
        let Fragmentainers::Pages(pages) = &self.fragmentainers else {
            return None;
        };
        let footnote = &pages.boxes[page].footnote;
        let flow = crate::style::logical::Flow::used(pages.document.root().style());
        if !flow.writing_mode.vertical() {
            return Some((
                std::borrow::Cow::Borrowed(&footnote.style),
                std::borrow::Cow::Borrowed(&footnote.decoration),
            ));
        }
        let logical = flow.horizontal();
        Some((
            std::borrow::Cow::Owned(crate::style::logical::reflowed_from(
                &footnote.style,
                flow,
                logical,
            )),
            std::borrow::Cow::Owned(crate::style::logical::decoration(
                &footnote.decoration,
                flow,
                logical,
            )),
        ))
    }

    fn area(&self, page: usize) -> Option<Area> {
        let (style, _) = self.area_box(page)?;
        let content = self.fragmentainers.content_box(page);
        let padding = std::array::from_fn(|side| style.padding[side].resolve(content.width));
        let borders = style
            .decoration
            .borders
            .map(crate::graphics::Border::used_width);
        let margins =
            std::array::from_fn(|side| style.margin[side].resolve(content.width).unwrap_or(0.0));
        let vertical = padding[TOP] + padding[BOTTOM] + borders[TOP] + borders[BOTTOM];
        let horizontal = padding[LEFT] + padding[RIGHT] + borders[LEFT] + borders[RIGHT];
        let content_size = |size: &crate::style::Size, basis: f32, edges: f32| {
            size.resolve_definite(Some(basis)).map(|size| {
                if style.box_sizing == BoxSizing::BorderBox {
                    (size - edges).max(0.0)
                } else {
                    size
                }
            })
        };
        let width = content_size(&style.width, content.width, horizontal)
            .unwrap_or(content.width - horizontal - margins[LEFT] - margins[RIGHT]);
        let height = content_size(&style.height, content.height, vertical);
        let minimum = content_size(&style.min_height, content.height, vertical)
            .unwrap_or(0.0)
            .max(height.unwrap_or(0.0));
        let maximum = match (
            height,
            content_size(&style.max_height, content.height, vertical),
        ) {
            (Some(height), Some(maximum)) => Some(height.min(maximum)),
            (height, maximum) => height.or(maximum),
        };
        Some(Area {
            x: content.x + margins[LEFT],
            width: width.max(0.0),
            padding,
            borders,
            margins,
            minimum,
            maximum,
        })
    }

    /// Whether the page holds only footnotes, which `max-height` then does not limit (CSS
    /// GCPM 3 §2.4.2).
    fn footnotes_only(&self, page: usize) -> bool {
        self.footnote_pages.is_some_and(|first| page >= first)
    }

    fn content_height(&self, page: usize) -> Option<f32> {
        let area = self.area(page)?;
        rows_height(self.footnotes.parts(page), area.width)
    }

    /// The height the page's footnote area takes from the page area.
    pub(super) fn footnote_reserve(&self, page: usize) -> f32 {
        self.area(page).map_or(0.0, |area| {
            area.outer_height(self.content_height(page), self.footnotes_only(page))
        })
    }

    fn footnote_layout(
        &mut self,
        index: usize,
        area_width: f32,
    ) -> Result<Rc<FootnoteLayout>, LayoutError> {
        let key = (index, area_width.to_bits());
        if let Some(layout) = self.footnotes.layouts.borrow().get(&key) {
            return Ok(layout.clone());
        }
        let Fragmentainers::Pages(pages) = &self.fragmentainers else {
            unreachable!("footnote areas belong to pages");
        };
        let document = pages.document;
        let footnotes = document.footnotes();
        let body = &footnotes.bodies[index].block;
        let inline = footnotes.bodies[index].inline;
        let compact = !inline && body.style().footnote_display == FootnoteDisplay::Compact;
        let width = if compact {
            self.measurer
                .run(|measurer| crate::layout::intrinsic::measure(body, measurer, area_width))?
                .max
                .min(area_width)
        } else {
            area_width
        };
        let measured =
            self.measurer
                .block(body, 0.0, width, body.style().used_rtl(), false, None)?;
        let height = if measured.through {
            let mut margins = measured.before.clone();
            margins.merge(measured.after.clone());
            margins.value()
        } else {
            measured.before.value() + measured.height + measured.after.value()
        };
        let flow = Paginator::standalone(
            &mut *self.measurer,
            measured,
            Rect {
                x: 0.0,
                y: 0.0,
                width,
                height,
            },
            document.navigation().destinations,
            // A footnote area's backgrounds may clip to the text of the bodies it holds.
            true,
        )?;
        let monolithic: Vec<[f32; 2]> = flow
            .line_boxes
            .iter()
            .copied()
            .chain(flow.items.iter().filter_map(|item| match item {
                DisplayItem::Image(image) => Some([image.rect.y, image.rect.y + image.rect.height]),
                _ => None,
            }))
            .collect();
        let total = flow.line_boxes.len();
        let lines_before = |offset: f32| {
            flow.line_boxes
                .iter()
                .filter(|[_, bottom]| *bottom <= offset)
                .count()
        };
        let (orphans, widows) = (body.style().orphans, body.style().widows);
        let mut breaks: Vec<f32> = flow
            .line_boxes
            .iter()
            .map(|[_, bottom]| *bottom)
            .filter(|&offset| {
                // A break leaves at least `orphans` lines before it and `widows` after it.
                let before = lines_before(offset);
                before >= orphans
                    && total - before >= widows
                    && monolithic
                        .iter()
                        .all(|[top, bottom]| offset <= *top || offset >= *bottom)
            })
            .chain([height])
            .collect();
        breaks.sort_by(f32::total_cmp);
        breaks.dedup();
        let mut members = Vec::new();
        if inline {
            let elements = &footnotes.bodies[index].members;
            members.resize(elements.len(), [0.0, height]);
            let mut end = height;
            for (position, &member) in elements.iter().enumerate().skip(1).rev() {
                let element = footnotes.elements[member].element;
                // A footnote that starts on no line starts where the one after it does.
                if let Some(start) = flow.event_lines.iter().find(|line| line.element == element) {
                    members[position][0] = start.extent[0];
                    end = if start.shared {
                        start.extent[1]
                    } else {
                        start.extent[0]
                    };
                } else {
                    members[position][0] = end;
                }
                members[position - 1][1] = end;
            }
        }
        let layout = Rc::new(FootnoteLayout {
            flow,
            width,
            height,
            compact,
            breaks,
            members,
        });
        self.footnotes
            .layouts
            .borrow_mut()
            .insert(key, layout.clone());
        Ok(layout)
    }

    fn document_footnotes(&self) -> &'t crate::document::Footnotes {
        let Fragmentainers::Pages(pages) = &self.fragmentainers else {
            unreachable!("footnote areas belong to pages");
        };
        pages.document.footnotes()
    }

    /// The part of its body that `footnote` adds to the area of `area_width`: a block
    /// footnote's whole body, or the lines of an inline body after those queued through
    /// `queued` up to the end of the footnote's content, if any.
    fn footnote_part(
        &mut self,
        footnote: usize,
        area_width: f32,
        queued: f32,
    ) -> Result<Option<FootnotePart>, LayoutError> {
        let footnotes = self.document_footnotes();
        let body = footnotes.elements[footnote].body;
        let layout = self.footnote_layout(body, area_width)?;
        let Some(position) = footnotes.bodies[body].inline.then(|| {
            footnotes.bodies[body]
                .members
                .iter()
                .position(|&member| member == footnote)
                .expect("an inline body holds its footnotes")
        }) else {
            return Ok(Some(FootnotePart {
                body,
                top: 0.0,
                bottom: layout.height,
                width: layout.width,
                inline: layout.compact,
            }));
        };
        let end = layout.members[position][1];
        Ok((end > queued).then_some(FootnotePart {
            body,
            top: queued,
            bottom: end,
            width: layout.width,
            inline: false,
        }))
    }

    /// Whether the content of `footnote` still waits for a later page.
    fn footnote_waits(&mut self, footnote: usize, area_width: f32) -> Result<bool, LayoutError> {
        let footnotes = self.document_footnotes();
        let body = footnotes.elements[footnote].body;
        let end = match footnotes.bodies[body]
            .members
            .iter()
            .position(|&member| member == footnote)
        {
            Some(position) if footnotes.bodies[body].inline => {
                self.footnote_layout(body, area_width)?.members[position][1]
            }
            _ => f32::INFINITY,
        };
        Ok(self
            .footnotes
            .carried
            .iter()
            .any(|part| part.body == body && part.top < end))
    }

    fn footnote_policy(&self, footnote: usize) -> FootnotePolicy {
        match &self.fragmentainers {
            Fragmentainers::Pages(pages) => pages.document.footnotes().elements[footnote].policy,
            Fragmentainers::Columns(_) => FootnotePolicy::Auto,
        }
    }

    /// Whether this paginator places footnotes: the page flow's, outside unbroken and
    /// parallel layout.
    pub(super) fn places_footnotes(&self) -> bool {
        matches!(self.fragmentainers, Fragmentainers::Pages(_))
            && self.unbroken == 0
            && self.parallel_depth == 0
    }

    /// How much more of the page area the footnote area takes with `footnotes` whole after
    /// the `pending` parts of earlier lines; `None` when they do not fit it whole, or must
    /// follow footnotes already waiting for the next page. `pending` gains their parts.
    pub(super) fn footnote_growth(
        &mut self,
        footnotes: &[usize],
        pending: &mut Vec<FootnotePart>,
    ) -> Result<Option<f32>, LayoutError> {
        let Some(area) = self.area(self.page) else {
            return Ok(None);
        };
        if self.footnotes.has_carried() {
            return Ok(None);
        }
        let mut parts: Vec<FootnotePart> = self
            .footnotes
            .parts(self.page)
            .iter()
            .chain(pending.iter())
            .copied()
            .collect();
        let before = rows_height(&parts, area.width);
        let start = parts.len();
        for &footnote in footnotes {
            let body = self.document_footnotes().elements[footnote].body;
            let queued = parts
                .iter()
                .filter(|part| part.body == body)
                .map(|part| part.bottom)
                .fold(
                    self.footnotes.queued.get(&body).copied().unwrap_or(0.0),
                    f32::max,
                );
            parts.extend(self.footnote_part(footnote, area.width, queued)?);
        }
        let after = rows_height(&parts, area.width);
        if area
            .maximum
            .is_some_and(|maximum| after.is_some_and(|after| overflows(after, maximum)))
        {
            return Ok(None);
        }
        pending.extend_from_slice(&parts[start..]);
        Ok(Some(
            area.outer_height(after, false) - area.outer_height(before, false),
        ))
    }

    /// The policy that keeps a footnote with its call when it does not fit: `line` or
    /// `block`, whichever moves more.
    pub(super) fn strictest_policy(&self, footnotes: &[usize]) -> FootnotePolicy {
        strictest(
            footnotes
                .iter()
                .map(|&footnote| self.footnote_policy(footnote)),
        )
    }

    /// The policy that moves `line` of `source` off the physical page it is on, because an
    /// earlier pagination found that a footnote called in a nested layout on the line does
    /// not fit there: the line calls it inside its atomic inlines or, in a nested layout,
    /// among the `footnotes` it calls directly.
    pub(super) fn deferred_policy(
        &self,
        source: &crate::document::Paragraph,
        line: &crate::text::Line,
        footnotes: &[usize],
    ) -> Result<Option<FootnotePolicy>, LayoutError> {
        let deferrals = &self.measurer.footnote_deferrals;
        if deferrals.is_empty() {
            return Ok(None);
        }
        let Some(page) = self.physical_page() else {
            return Ok(None);
        };
        let mut calls = Vec::new();
        for fragment in line.atomics.iter().chain(&line.annotations) {
            let (events, visited) = source.inline_block(fragment.index).block.subtree_events();
            self.measurer.work.spend(visited)?;
            calls.extend(events.into_iter().filter_map(|event| event.footnote));
        }
        if !self.places_footnotes() {
            calls.extend_from_slice(footnotes);
        }
        let policy = strictest(
            calls
                .into_iter()
                .filter_map(|footnote| deferrals.get(&(footnote, page)).copied()),
        );
        Ok((policy != FootnotePolicy::Auto).then_some(policy))
    }

    /// Queues footnotes not yet placed after those waiting, then fills the current page's
    /// area with them.
    fn queue_footnotes(
        &mut self,
        footnotes: impl IntoIterator<Item = usize>,
    ) -> Result<(), LayoutError> {
        let Some(area) = self.area(self.page) else {
            return Ok(());
        };
        for footnote in footnotes {
            if !self.footnotes.placed.insert(footnote) {
                continue;
            }
            let footnotes = self.document_footnotes();
            let body = footnotes.elements[footnote].body;
            let mut queued = self.footnotes.queued.get(&body).copied().unwrap_or(0.0);
            let members = &footnotes.bodies[body].members;
            if let Some(position) = members.iter().position(|&member| member == footnote)
                && position > 0
            {
                // An inline footnote continues the inline formatting context of the one
                // before it only right after its lines, at the end of the area.
                let layout = self.footnote_layout(body, area.width)?;
                let last = self
                    .footnotes
                    .carried
                    .back()
                    .or_else(|| self.footnotes.parts(self.page).last());
                if queued != layout.members[position - 1][1]
                    || last.is_none_or(|part| part.body != body)
                {
                    self.footnotes
                        .inline
                        .starts
                        .insert(footnotes.elements[footnote].element);
                    // The rest of this pagination approximates the footnote starting its own
                    // inline formatting context with its lines from the one it starts on.
                    queued = queued.min(layout.members[position][0]);
                }
            }
            let Some(part) = self.footnote_part(footnote, area.width, queued)? else {
                continue;
            };
            self.footnotes.queued.insert(body, part.bottom);
            match self.footnotes.carried.back_mut() {
                Some(last) if last.body == body && last.bottom == part.top => {
                    last.bottom = part.bottom;
                }
                _ => self.footnotes.carried.push_back(part),
            }
        }
        Ok(())
    }

    /// Records `page` as the page of the inline formatting contexts that start in `part` of an
    /// inline body placed there, those of its first footnote and of the footnotes that start
    /// their own, whose anonymous blocks inherit from its footnote area.
    fn record_inline_pages(
        &mut self,
        part: FootnotePart,
        page: usize,
        area_width: f32,
    ) -> Result<(), LayoutError> {
        let footnotes = self.document_footnotes();
        let body = &footnotes.bodies[part.body];
        if !body.inline {
            return Ok(());
        }
        let layout = self.footnote_layout(part.body, area_width)?;
        let Fragmentainers::Pages(pages) = &self.fragmentainers else {
            unreachable!("footnote areas belong to pages");
        };
        let request = &pages.boxes[page].request;
        // Formatting takes the first page's area until a pagination places a context.
        if *request == pages.document.page_box().request {
            return Ok(());
        }
        for (position, &member) in body.members.iter().enumerate() {
            let element = footnotes.elements[member].element;
            let start = layout.members[position][0];
            if (position == 0 || self.footnotes.inline.starts.contains(&element))
                && part.top <= start
                && (start < part.bottom || part.top == part.bottom)
            {
                self.footnotes
                    .inline
                    .pages
                    .entry(element)
                    .or_insert_with(|| request.clone());
            }
        }
        Ok(())
    }

    /// Places the footnotes of calls on the current page where they fit below the flow, as
    /// much as fits of the first that does not, the rest waiting for the next page.
    pub(super) fn place_footnotes(&mut self, footnotes: &[usize]) -> Result<(), LayoutError> {
        self.queue_footnotes(footnotes.iter().copied())?;
        self.place_carried_footnotes()
    }

    /// Queues the footnotes whose calls the flow recorded on this page or an earlier one
    /// without placing them: calls in unbroken, parallel or nested fragmentation contexts,
    /// such as table cells, columns and positioned boxes. Returns those it queued, with the
    /// pages of their calls.
    fn queue_recorded_footnotes(&mut self) -> Result<Vec<(usize, usize)>, LayoutError> {
        let calls = self
            .string_locations
            .footnote_calls(self.footnotes.recorded)
            .to_vec();
        self.footnotes.recorded += calls.len();
        self.footnotes.ahead.extend(calls);
        let page = self.page;
        let (reached, ahead) = std::mem::take(&mut self.footnotes.ahead)
            .into_iter()
            .partition(|(_, call)| *call <= page);
        self.footnotes.ahead = ahead;
        let queued: Vec<(usize, usize)> = reached
            .into_iter()
            .filter(|(footnote, _)| !self.footnotes.placed.contains(footnote))
            .collect();
        self.queue_footnotes(queued.iter().map(|&(footnote, _)| footnote))?;
        Ok(queued)
    }

    /// Places the footnotes of recorded calls below the content of the current page, where
    /// they fit, before more flow follows them or the page ends. A footnote that must stay
    /// with its call (CSS GCPM 3 §2.8) and does not fit defers its call to a later page,
    /// which a new pagination settles; a call that was already deferred from its page and
    /// stays on it lies where no break can move it, and its footnote continues after it.
    pub(super) fn place_recorded_footnotes(&mut self) -> Result<(), LayoutError> {
        let queued = self.queue_recorded_footnotes()?;
        if queued.is_empty() {
            return Ok(());
        }
        self.fill_footnote_area(false)?;
        let Some(area) = self.area(self.page) else {
            return Ok(());
        };
        for (footnote, page) in queued {
            let policy = self.footnote_policy(footnote);
            if policy != FootnotePolicy::Auto
                && self.footnote_waits(footnote, area.width)?
                && !self
                    .measurer
                    .footnote_deferrals
                    .contains_key(&(footnote, page))
            {
                self.footnotes.deferred.insert((footnote, page), policy);
            }
        }
        Ok(())
    }

    /// Fills the current page's footnote area with waiting footnote content in order, as far
    /// as the space below the flow and the area's maximum height allow; a page without flow
    /// content takes at least one break interval so that every page advances.
    pub(super) fn place_carried_footnotes(&mut self) -> Result<(), LayoutError> {
        self.fill_footnote_area(true)
    }

    /// Fills the current page's footnote area with waiting footnote content; `advance` lets a
    /// page without flow content take a slice that does not fit, so that pagination ends.
    fn fill_footnote_area(&mut self, advance: bool) -> Result<(), LayoutError> {
        let page = self.page;
        let Some(area) = self.area(page) else {
            return Ok(());
        };
        let page_height = self.fragmentainers.height(page);
        let room = page_height - self.y - area.edges(TOP) - area.edges(BOTTOM);
        let limit = match area.maximum {
            Some(maximum) if !self.footnotes_only(page) => room.min(maximum),
            _ => room,
        };
        if self.footnotes.pages.len() <= page {
            self.footnotes.pages.resize_with(page + 1, Vec::new);
        }
        while let Some(&waiting) = self.footnotes.carried.front() {
            let mut parts = self.footnotes.pages[page].clone();
            parts.push(waiting);
            if rows_height(&parts, area.width).is_some_and(|height| !overflows(height, limit)) {
                self.record_inline_pages(waiting, page, area.width)?;
                self.footnotes.pages[page].push(waiting);
                self.footnotes.carried.pop_front();
                continue;
            }
            let layout = self.footnote_layout(waiting.body, area.width)?;
            let used = rows_height(&self.footnotes.pages[page], area.width);
            let end = waiting.top + (limit - used.unwrap_or(0.0)).max(0.0);
            let fits = layout
                .breaks
                .iter()
                .copied()
                .filter(|offset| *offset > waiting.top)
                .take_while(|offset| !overflows(*offset, end))
                .last();
            let forced = advance && used.is_none() && !self.has_flow;
            let Some(bottom) = fits.or_else(|| {
                forced
                    .then(|| {
                        layout
                            .breaks
                            .iter()
                            .copied()
                            .find(|offset| *offset > waiting.top)
                    })
                    .flatten()
            }) else {
                break;
            };
            // A slice of a body takes a line of its own.
            let slice = FootnotePart {
                bottom,
                inline: false,
                ..waiting
            };
            self.record_inline_pages(slice, page, area.width)?;
            self.footnotes.pages[page].push(slice);
            if bottom >= waiting.bottom {
                self.footnotes.carried.pop_front();
            } else {
                self.footnotes
                    .carried
                    .front_mut()
                    .expect("the waiting footnote continues")
                    .top = bottom;
                break;
            }
        }
        self.height = fragmentainer_height(page_height - self.footnote_reserve(page));
        Ok(())
    }

    /// Paints each page's footnote area at the bottom of its page area, returning the
    /// navigation of the footnote bodies by page, and the text of each page's bodies when its
    /// page box's backgrounds clip to text.
    pub(super) fn paint_footnotes(
        &mut self,
        page_count: usize,
    ) -> Result<FootnotePaint, LayoutError> {
        let mut navigation = Vec::new();
        let mut page_text: Box<[Vec<DisplayItem>]> = vec![Vec::new(); page_count].into();
        for page in 0..page_count.min(self.footnotes.pages.len()) {
            let Some(content) = self.content_height(page) else {
                continue;
            };
            let area = self.area(page).expect("footnote areas belong to pages");
            let (style, decoration) = self.area_box(page).expect("footnote areas belong to pages");
            let Fragmentainers::Pages(pages) = &self.fragmentainers else {
                unreachable!("footnote areas belong to pages");
            };
            let flow = crate::style::logical::Flow::used(pages.document.root().style());
            let page_clips = pages.boxes[page].decoration.backgrounds.clips_to_text();
            let decoration = decoration.into_owned();
            let page_area = self.fragmentainers.content_box(page);
            let height = area.content_height(content, self.footnotes_only(page));
            let border_box = Rect {
                x: area.x,
                y: page_area.y + page_area.height
                    - area.margins[BOTTOM]
                    - area.borders[BOTTOM]
                    - area.padding[BOTTOM]
                    - height
                    - area.padding[TOP]
                    - area.borders[TOP],
                width: area.width
                    + area.padding[LEFT]
                    + area.padding[RIGHT]
                    + area.borders[LEFT]
                    + area.borders[RIGHT],
                height: height
                    + area.padding[TOP]
                    + area.padding[BOTTOM]
                    + area.borders[TOP]
                    + area.borders[BOTTOM],
            };
            // CSS Backgrounds 4 §2.8: the area's text-clipped backgrounds paint within the
            // text of the footnote bodies it holds.
            let clips_to_text = decoration.backgrounds.clips_to_text();
            let visible = style.visibility.is_visible();
            let paint = BoxPaint {
                padding: area.padding,
                ..BoxPaint::whole(border_box, decoration)
            };
            // The area transforms about its border box like a block box, and composites as one
            // group, which its bodies' boxes join.
            let transform = self.transform_on(&style, &paint.fragment_areas(), None, page)?;
            if transform.as_ref().is_some_and(Region::collapses) {
                continue;
            }
            let group = Group::of(&style);
            let pushes: Vec<_> = transform
                .clone()
                .map(Region::push)
                .into_iter()
                .chain(group.pushes(std::slice::from_ref(&paint)))
                .collect();
            let backdrop = group.backdrop(&paint);
            let mut items = Vec::new();
            if visible {
                items.extend(backdrop);
                if paint.decoration.is_visible() {
                    items.push(DisplayItem::Box(paint.clone()));
                }
            }
            let mut text = Vec::new();
            let x = border_box.x + area.borders[LEFT] + area.padding[LEFT];
            let mut y = border_box.y + area.borders[TOP] + area.padding[TOP];
            for row in rows(self.footnotes.parts(page), area.width) {
                for (part, offset) in row.parts {
                    let layout = self.footnote_layout(part.body, area.width)?;
                    let window = Rect {
                        x: 0.0,
                        y: part.top,
                        width: layout.width,
                        height: part.height(),
                    };
                    let origin = [x + offset, y - part.top];
                    let sliced = part.top > 0.0 || part.bottom < layout.height;
                    // The part's slice of the body's items, placed in the area.
                    let frame = |mut items: Vec<DisplayItem>, lines: Vec<TextLine>| {
                        if sliced {
                            items.insert(
                                0,
                                DisplayItem::PushClip(Clip {
                                    rect: window,
                                    radii: crate::graphics::Radii::default(),
                                    outline: None,
                                    axes: [false, true],
                                    reference_box: None,
                                }),
                            );
                            items.push(DisplayItem::Pop);
                        }
                        DisplayItem::Frame(Box::new(crate::graphics::FlowPaint {
                            matrix: [1.0, 0.0, 0.0, 1.0, origin[0], origin[1]],
                            area: Rect {
                                height: layout.height,
                                y: 0.0,
                                ..window
                            },
                            items,
                            lines,
                        }))
                    };
                    items.push(frame(layout.flow.items.clone(), layout.flow.lines.clone()));
                    if (clips_to_text || page_clips)
                        && let Some(area) = &layout.flow.text_area
                    {
                        text.push(frame(area.items.clone(), area.lines.clone()));
                    }
                    let placed = layout.flow.navigation.translated(origin, window);
                    navigation.push((
                        page,
                        match &transform {
                            Some(region) => placed.transformed(region),
                            None => placed,
                        },
                    ));
                    // CSS Images 4 §2.7: a body broken across footnote areas is imaged as laid
                    // out whole, which its first part starts.
                    if part.top == 0.0 {
                        self.measurer
                            .element_targets
                            .displayed
                            .borrow_mut()
                            .adopt((*layout.flow.renderings).clone().physical(flow));
                    }
                }
                y += row.height;
            }
            if clips_to_text
                && let Some(DisplayItem::Box(paint)) = items
                    .iter_mut()
                    .find(|item| matches!(item, DisplayItem::Box(_)))
            {
                paint.text_area = Some(std::sync::Arc::new(crate::graphics::FlowPaint {
                    matrix: IDENTITY,
                    area: page_area,
                    items: text.clone(),
                    lines: Vec::new(),
                }));
            }
            if page_clips {
                page_text[page] = match &transform {
                    Some(region) => std::iter::once(region.clone().push())
                        .chain(text)
                        .chain(std::iter::once(DisplayItem::Pop))
                        .collect(),
                    None => text,
                };
            }
            let pops = pushes.len();
            let items = pushes
                .into_iter()
                .chain(items)
                .chain(std::iter::repeat_n(DisplayItem::Pop, pops));
            let scope: PaintKey = Rc::from([FLOW_LEVEL]);
            let (clip, group) = self.root_paint.unwrap_or_default();
            for item in items {
                self.foreground[page].push(ScopedPaint {
                    native: None,
                    visible: true,
                    order: self.orders.take(),
                    scope: scope.clone(),
                    clip,
                    group,
                    ink: None,
                    item,
                    navigation: BoxNavigation::default(),
                });
            }
        }
        Ok((navigation, page_text))
    }
}

/// The navigation of the footnote bodies by page, and the text of each page's bodies.
type FootnotePaint = (Vec<(usize, StandaloneNavigation)>, Box<[Vec<DisplayItem>]>);
