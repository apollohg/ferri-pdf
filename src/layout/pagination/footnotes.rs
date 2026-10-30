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
}

/// The slice `[top, bottom)` of a footnote body that one page's footnote area holds.
#[derive(Clone, Copy)]
pub(super) struct FootnotePart {
    footnote: usize,
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

/// Footnote bodies by footnote and the bits of their area's width.
type FootnoteLayouts = HashMap<(usize, u32), Rc<FootnoteLayout>>;

/// The footnotes each page's area holds, the remainders waiting for the next page, and the
/// footnotes already placed or waiting.
#[derive(Clone, Default)]
pub(super) struct Footnotes {
    pages: Vec<Vec<FootnotePart>>,
    carried: VecDeque<FootnotePart>,
    placed: BTreeSet<usize>,
    /// Bodies by footnote and area width; laying a body out does not depend on its page.
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
    fn area(&self, page: usize) -> Option<Area> {
        let Fragmentainers::Pages(pages) = &self.fragmentainers else {
            return None;
        };
        let style = &pages.boxes[page].footnote.style;
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
        footnote: usize,
        area_width: f32,
    ) -> Result<Rc<FootnoteLayout>, LayoutError> {
        let key = (footnote, area_width.to_bits());
        if let Some(layout) = self.footnotes.layouts.borrow().get(&key) {
            return Ok(layout.clone());
        }
        let Fragmentainers::Pages(pages) = &self.fragmentainers else {
            unreachable!("footnote areas belong to pages");
        };
        let document = pages.document;
        let body = &document.footnotes()[footnote];
        let compact = body.style().footnote_display == FootnoteDisplay::Compact;
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
        let layout = Rc::new(FootnoteLayout {
            flow,
            width,
            height,
            compact,
            breaks,
        });
        self.footnotes
            .layouts
            .borrow_mut()
            .insert(key, layout.clone());
        Ok(layout)
    }

    /// The whole body of `footnote` as one part of the area of `area_width`.
    fn whole_part(
        &mut self,
        footnote: usize,
        area_width: f32,
    ) -> Result<FootnotePart, LayoutError> {
        let layout = self.footnote_layout(footnote, area_width)?;
        Ok(FootnotePart {
            footnote,
            top: 0.0,
            bottom: layout.height,
            width: layout.width,
            inline: layout.compact,
        })
    }

    fn footnote_policy(&self, footnote: usize) -> FootnotePolicy {
        match &self.fragmentainers {
            Fragmentainers::Pages(pages) => {
                pages.document.footnotes()[footnote].style().footnote_policy
            }
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
        for &footnote in footnotes {
            parts.push(self.whole_part(footnote, area.width)?);
        }
        let after = rows_height(&parts, area.width);
        if area
            .maximum
            .is_some_and(|maximum| after.is_some_and(|after| overflows(after, maximum)))
        {
            return Ok(None);
        }
        let start = parts.len() - footnotes.len();
        pending.extend_from_slice(&parts[start..]);
        Ok(Some(
            area.outer_height(after, false) - area.outer_height(before, false),
        ))
    }

    /// The policy that keeps a footnote with its call when it does not fit: `line` or
    /// `block`, whichever moves more.
    pub(super) fn strictest_policy(&self, footnotes: &[usize]) -> FootnotePolicy {
        footnotes
            .iter()
            .map(|&footnote| self.footnote_policy(footnote))
            .max_by_key(|policy| match policy {
                FootnotePolicy::Auto => 0,
                FootnotePolicy::Line => 1,
                FootnotePolicy::Block => 2,
            })
            .unwrap_or_default()
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
            if self.footnotes.placed.insert(footnote) {
                let part = self.whole_part(footnote, area.width)?;
                self.footnotes.carried.push_back(part);
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

    /// Footnotes whose calls the flow recorded without placing them, in unbroken, parallel
    /// or nested fragmentation contexts, wait for the next page.
    pub(super) fn carry_recorded_footnotes(&mut self) -> Result<(), LayoutError> {
        let recorded: Vec<usize> = self.string_locations.footnote_calls().collect();
        self.queue_footnotes(recorded)
    }

    /// Fills the current page's footnote area with waiting footnote content in order, as far
    /// as the space below the flow and the area's maximum height allow; a page without flow
    /// content takes at least one break interval so that every page advances.
    pub(super) fn place_carried_footnotes(&mut self) -> Result<(), LayoutError> {
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
                self.footnotes.pages[page].push(waiting);
                self.footnotes.carried.pop_front();
                continue;
            }
            let layout = self.footnote_layout(waiting.footnote, area.width)?;
            let used = rows_height(&self.footnotes.pages[page], area.width);
            let end = waiting.top + (limit - used.unwrap_or(0.0)).max(0.0);
            let fits = layout
                .breaks
                .iter()
                .copied()
                .filter(|offset| *offset > waiting.top)
                .take_while(|offset| !overflows(*offset, end))
                .last();
            let forced = used.is_none() && !self.has_flow;
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
            self.footnotes.pages[page].push(FootnotePart {
                bottom,
                inline: false,
                ..waiting
            });
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
    /// navigation of the footnote bodies by page.
    pub(super) fn paint_footnotes(
        &mut self,
        page_count: usize,
    ) -> Result<Vec<(usize, StandaloneNavigation)>, LayoutError> {
        let mut navigation = Vec::new();
        for page in 0..page_count.min(self.footnotes.pages.len()) {
            let Some(content) = self.content_height(page) else {
                continue;
            };
            let area = self.area(page).expect("footnote areas belong to pages");
            let Fragmentainers::Pages(pages) = &self.fragmentainers else {
                unreachable!("footnote areas belong to pages");
            };
            let decoration = pages.boxes[page].footnote.decoration.clone();
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
            let mut items = Vec::new();
            if decoration.is_visible() {
                items.push(DisplayItem::Box(BoxPaint {
                    padding: area.padding,
                    ..BoxPaint::whole(border_box, decoration)
                }));
            }
            let x = border_box.x + area.borders[LEFT] + area.padding[LEFT];
            let mut y = border_box.y + area.borders[TOP] + area.padding[TOP];
            for row in rows(self.footnotes.parts(page), area.width) {
                for (part, offset) in row.parts {
                    let layout = self.footnote_layout(part.footnote, area.width)?;
                    let window = Rect {
                        x: 0.0,
                        y: part.top,
                        width: layout.width,
                        height: part.height(),
                    };
                    let mut frame_items = layout.flow.items.clone();
                    if part.top > 0.0 || part.bottom < layout.height {
                        frame_items.insert(
                            0,
                            DisplayItem::PushClip(Clip {
                                rect: window,
                                radii: crate::graphics::Radii::default(),
                                outline: None,
                                axes: [false, true],
                                reference_box: None,
                            }),
                        );
                        frame_items.push(DisplayItem::Pop);
                    }
                    let origin = [x + offset, y - part.top];
                    items.push(DisplayItem::Frame(Box::new(crate::graphics::FlowPaint {
                        matrix: [1.0, 0.0, 0.0, 1.0, origin[0], origin[1]],
                        area: Rect {
                            height: layout.height,
                            y: 0.0,
                            ..window
                        },
                        items: frame_items,
                        lines: layout.flow.lines.clone(),
                    })));
                    navigation.push((page, layout.flow.navigation.translated(origin, window)));
                }
                y += row.height;
            }
            let scope: PaintKey = Rc::from([FLOW_LEVEL]);
            for item in items {
                self.foreground[page].push(ScopedPaint {
                    native: None,
                    visible: true,
                    order: self.orders.take(),
                    scope: scope.clone(),
                    clip: None,
                    group: None,
                    ink: None,
                    item,
                    navigation: BoxNavigation::default(),
                });
            }
        }
        Ok(navigation)
    }
}
