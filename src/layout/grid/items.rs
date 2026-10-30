use crate::document::BlockBox;
use crate::layout::measure::Failure;
use crate::layout::{
    geometry::{Edges, HorizontalBox, SizeConstraints},
    measure::{ItemMetrics, MeasuredBlock, MeasuredCell, Measurer},
};
use crate::style::{Alignment, COLUMNS, OverflowPosition, ROWS, Size, Style};
use std::ops::Range;

pub(crate) struct Item<'a> {
    pub parent: Option<super::ParentContext>,
    pub source: &'a BlockBox,
    pub inline_definite: bool,
    pub geometry: HorizontalBox,
    pub margins: [Option<f32>; 4],
    pub align: Alignment,
    pub align_overflow: OverflowPosition,
}

#[derive(Clone)]
pub(crate) struct BaselineContribution {
    pub contribution: crate::grid::SizingItem,
    pub baseline: f32,
}

pub(super) fn add_baseline_shim(item: &mut crate::grid::SizingItem, shim: f32) {
    item.min_content += shim;
    item.max_content += shim;
    item.minimum += shim;
    item.content_minimum = item.content_minimum.map(|value| value + shim);
}

pub(super) struct Baselines {
    pub groups: Vec<f32>,
    pub items: Vec<(usize, Option<f32>)>,
}
impl Baselines {
    pub(super) fn new(rows: usize) -> Self {
        Self {
            groups: vec![f32::NEG_INFINITY; rows],
            items: Vec::new(),
        }
    }
    pub(super) fn push(&mut self, row: usize, baseline: Option<f32>) {
        if let Some(baseline) = baseline {
            self.groups[row] = self.groups[row].max(baseline);
        }
        self.items.push((row, baseline));
    }
    pub(super) fn container(
        &self,
        cells: &[MeasuredCell<'_>],
        areas: &[[Range<usize>; 2]],
    ) -> [Option<f32>; 2] {
        [false, true].map(|last| {
            let row = if last {
                areas.iter().map(|area| area[ROWS].end - 1).max()
            } else {
                areas.iter().map(|area| area[ROWS].start).min()
            }?;
            let candidates = areas
                .iter()
                .enumerate()
                .filter(|(_, area)| area[ROWS].contains(&row));
            let shared = candidates
                .clone()
                .find(|(index, area)| area[ROWS].start == row && self.items[*index].1.is_some());
            let (index, _) = shared.or_else(|| {
                let order = |(index, area): &(usize, &[Range<usize>; 2])| {
                    (area[ROWS].start, area[COLUMNS].start, *index)
                };
                if last {
                    candidates.max_by_key(order)
                } else {
                    candidates.min_by_key(order)
                }
            })?;
            let cell = &cells[index];
            let baseline = if last && shared.is_none() {
                cell.block.last_baseline()
            } else {
                cell.block.first_baseline()
            };
            Some(cell.offset + baseline.unwrap_or(cell.block.height))
        })
    }

    pub(super) fn offset(&self, index: usize) -> f32 {
        let (row, baseline) = self.items[index];
        baseline.map_or(0.0, |baseline| self.groups[row] - baseline)
    }
}

pub(super) fn physical_alignment(
    alignment: Alignment,
    reverse: bool,
    own_reverse: bool,
) -> Alignment {
    let alignment = match alignment {
        Alignment::SelfStart => {
            if own_reverse {
                Alignment::Right
            } else {
                Alignment::Left
            }
        }
        Alignment::SelfEnd => {
            if own_reverse {
                Alignment::Left
            } else {
                Alignment::Right
            }
        }
        alignment => alignment,
    };
    match alignment {
        Alignment::Center => Alignment::Center,
        Alignment::Right => Alignment::Right,
        Alignment::Left => Alignment::Left,
        Alignment::End | Alignment::FlexEnd if !reverse => Alignment::Right,
        Alignment::Start
        | Alignment::FlexStart
        | Alignment::Normal
        | Alignment::Stretch
        | Alignment::Auto
            if reverse =>
        {
            Alignment::Right
        }
        _ => Alignment::Left,
    }
}

fn offset(
    (alignment, overflow): (Alignment, OverflowPosition),
    free: f32,
    reverse: bool,
    own_reverse: bool,
) -> f32 {
    let alignment = if overflow.starts(free) {
        Alignment::Start
    } else {
        alignment
    };
    match physical_alignment(alignment, reverse, own_reverse) {
        Alignment::Center => free / 2.0,
        Alignment::Right => free,
        _ => 0.0,
    }
}

impl<'a> Item<'a> {
    /// The item in its grid `area`, self-aligned by default as the grid `container`'s
    /// `align-items` and `justify-items`, without the margins `margin-trim` discards at the
    /// `trimmed` sides.
    pub(crate) fn new(
        source: &'a BlockBox,
        area: Range<f32>,
        parent: &HorizontalBox,
        container: &Style,
        height: Option<f32>,
        trimmed: [bool; 4],
        measurer: &Measurer<'a>,
    ) -> Result<Self, Failure> {
        let style = source.style();
        let available = area.end - area.start;
        let edges = measurer.box_edges(source, available)?;
        let mut margins = style
            .margin
            .each_ref()
            .map(|value| value.resolve(available));
        for (margin, trimmed) in margins.iter_mut().zip(trimmed) {
            if trimmed {
                *margin = Some(0.0);
            }
        }
        let shared = [ROWS, COLUMNS].map(|axis| super::sizing::shared(style, axis));
        for (axis, sides) in [(ROWS, [0, 2]), (COLUMNS, [3, 1])] {
            if shared[axis] {
                for side in sides {
                    margins[side] = Some(margins[side].unwrap_or(0.0));
                }
            }
        }
        let constraints = super::sizing::constraints(
            style,
            COLUMNS,
            Some(available),
            edges.horizontal(),
            shared[COLUMNS],
        );
        let (justify, justify_overflow) = if shared[COLUMNS] {
            (Alignment::Stretch, OverflowPosition::Default)
        } else {
            style
                .grid
                .used_justify_self(container.grid.default_justify())
        };
        let (align, align_overflow) = if shared[ROWS] {
            (Alignment::Stretch, OverflowPosition::Default)
        } else {
            style.flex.used_align_self(&container.flex)
        };
        let auto = usize::from(margins[1].is_none()) + usize::from(margins[3].is_none());
        let space = (available
            - edges.horizontal()
            - margins[1].unwrap_or(0.0)
            - margins[3].unwrap_or(0.0))
        .max(0.0);
        let constraints = if shared[COLUMNS] {
            constraints
        } else {
            let mut constraints = crate::layout::intrinsic::width_constraints(
                source,
                measurer,
                Some(available),
                edges.horizontal(),
                Some(space),
            )?;
            crate::layout::ratio::transfer_to_width(style, &mut constraints, height, &edges);
            constraints
        };
        // CSS Grid 2 §6.2: with a preferred aspect ratio, `normal` sizes the item as a
        // block-level box, which takes its automatic width from a definite height.
        let ratio_width = if shared[COLUMNS]
            || !matches!(justify, Alignment::Normal | Alignment::Auto)
        {
            None
        } else {
            crate::layout::ratio::width_from_height(measurer, source, available, height, &edges)?
        };
        let width = constraints.constrain(if let Some(width) = constraints.specified {
            width
        } else if let Some(width) = ratio_width {
            width
        } else {
            constraints.preferred(
                if auto == 0
                    && (justify == Alignment::Stretch
                        || (matches!(justify, Alignment::Normal | Alignment::Auto)
                            && source.image().is_none()))
                {
                    space
                } else if source.image().is_some() {
                    crate::layout::images::size(source, available, available, height, None, None)[0]
                } else {
                    let natural = crate::layout::intrinsic::measure(source, measurer, available)?;
                    (natural.max.min(available).max(natural.min)
                        - edges.horizontal()
                        - margins[1].unwrap_or(0.0)
                        - margins[3].unwrap_or(0.0))
                    .max(0.0)
                },
            )
        });
        let free = available
            - width
            - edges.horizontal()
            - margins[1].unwrap_or(0.0)
            - margins[3].unwrap_or(0.0);
        let left = margins[3].unwrap_or(if free > 0.0 {
            free / auto.max(1) as f32
        } else {
            0.0
        }) + if auto == 0 {
            offset(
                (justify, justify_overflow),
                free,
                parent.containing_rtl,
                style.used_rtl(),
            )
        } else {
            0.0
        };
        Ok(Self {
            source,
            inline_definite: !measurer.space.indefinite_inline,
            geometry: HorizontalBox {
                containing_x: parent.x + area.start,
                containing_width: available,
                containing_rtl: parent.containing_rtl,
                x: parent.x + area.start + left + edges.padding[3] + edges.border[3],
                width,
                padding: edges.padding,
                border: edges.border,
                margin: [Some(0.0); 4],
            },
            margins,
            align,
            align_overflow,
            parent: None,
        })
    }

    pub(super) fn inherit(
        &mut self,
        parent: Option<super::ParentContext>,
        tracks: [Option<std::rc::Rc<super::TrackGeometry>>; 2],
    ) {
        let left = self.geometry.x - self.geometry.containing_x;
        let right = self.geometry.containing_width - self.geometry.width - left;
        let vertical = [0, 2].map(|side| {
            self.geometry.padding[side]
                + self.geometry.border[side]
                + self.margins[side].unwrap_or(0.0)
        });
        self.parent = parent.map(|parent| parent.with_geometry(tracks, [vertical, [left, right]]));
    }

    pub(crate) fn measure(
        &self,
        measurer: &mut Measurer<'a>,
        height: Option<f32>,
        used_height: Option<f32>,
    ) -> Result<MeasuredBlock<'a>, Failure> {
        let mut space = measurer.space;
        space.indefinite_inline &= !self.inline_definite;
        measurer.with_space(space, |measurer| {
            measurer.with_grid_parent(self.parent.clone(), |measurer| {
                measurer.with_sized_item(self.source, |measurer| {
                    self.measure_contents(measurer, height, used_height)
                })
            })
        })
    }

    fn measure_contents(
        &self,
        measurer: &mut Measurer<'a>,
        height: Option<f32>,
        used_height: Option<f32>,
    ) -> Result<MeasuredBlock<'a>, Failure> {
        if self.source.style().display.is_table() {
            crate::layout::tables::measure_wrapper(
                measurer,
                used_height,
                |measurer, used_height| {
                    crate::layout::tables::measure_format(
                        measurer,
                        self.source,
                        &self.geometry,
                        height,
                        used_height,
                    )
                },
            )
        } else {
            measurer.measure_format(
                self.source,
                self.geometry.clone(),
                true,
                height,
                used_height,
            )
        }
    }

    pub(crate) fn measure_area(
        &self,
        measurer: &mut Measurer<'a>,
        available: f32,
    ) -> Result<MeasuredBlock<'a>, Failure> {
        let used = self.stretch().then(|| {
            let heights = self.height(Some(available));
            heights.constrain(
                heights.preferred(
                    (available
                        - self.vertical_edges()
                        - self.margins[0].unwrap_or(0.0)
                        - self.margins[2].unwrap_or(0.0))
                    .max(0.0),
                ),
            )
        });
        self.measure(measurer, Some(available), used)
    }

    pub(crate) fn height(&self, available: Option<f32>) -> SizeConstraints {
        let style = self.source.style();
        super::sizing::constraints(
            style,
            ROWS,
            available,
            self.vertical_edges(),
            super::sizing::shared(style, ROWS),
        )
    }

    pub(super) fn row_edges(&self) -> [f32; 2] {
        [0, 2].map(|side| {
            self.geometry.padding[side]
                + self.geometry.border[side]
                + self.margins[side].unwrap_or(0.0)
        })
    }

    pub(super) fn project_rows(
        &self,
        rows: &[crate::grid::SizingItem],
        parent: Range<usize>,
        parent_gap: f32,
        measurer: &Measurer<'_>,
    ) -> Result<Vec<crate::grid::SizingItem>, Failure> {
        let child_gap = self.source.style().flex.gap[ROWS].resolve_normal(0.0, parent_gap);
        measurer.work.spend(rows.len().saturating_mul(4).max(1))?;
        Ok(crate::grid::subgrid_contributions(
            rows,
            parent,
            false,
            self.row_edges(),
            child_gap - parent_gap,
        )?)
    }

    pub(crate) fn vertical_edges(&self) -> f32 {
        self.geometry.padding[0]
            + self.geometry.padding[2]
            + self.geometry.border[0]
            + self.geometry.border[2]
    }

    pub(crate) fn baseline(&self, measured: &ItemMetrics, intrinsic_tracks: bool) -> Option<f32> {
        let style = self.source.style();
        let cyclic = intrinsic_tracks
            && [&style.height, &style.min_height, &style.max_height]
                .into_iter()
                .any(|size| {
                    matches!(size.basis(), Size::Value(_)) && size.resolve_definite(None).is_none()
                });
        (self.align == Alignment::Baseline
            && self.margins[0].is_some()
            && self.margins[2].is_some()
            && !cyclic)
            .then(|| measured.baseline.unwrap_or(measured.height) + self.margins[0].unwrap_or(0.0))
    }

    pub(crate) fn stretch(&self) -> bool {
        (super::sizing::shared(self.source.style(), ROWS)
            || self
                .source
                .style()
                .height
                .is_auto_block_size(self.source.image().is_some()))
            && self.margins[0].is_some()
            && self.margins[2].is_some()
            && (self.align == Alignment::Stretch
                || (matches!(self.align, Alignment::Normal | Alignment::Auto)
                    && self.source.image().is_none()
                    && crate::layout::ratio::PreferredRatio::of_box(
                        self.source.style(),
                        &Edges {
                            padding: self.geometry.padding,
                            border: self.geometry.border,
                        },
                    )
                    .is_none()))
    }

    pub(crate) fn top(&self, available: f32, height: f32) -> f32 {
        let free =
            available - height - self.margins[0].unwrap_or(0.0) - self.margins[2].unwrap_or(0.0);
        let auto = usize::from(self.margins[0].is_none()) + usize::from(self.margins[2].is_none());
        self.margins[0].unwrap_or(if free > 0.0 {
            free / auto.max(1) as f32
        } else {
            0.0
        }) + if auto == 0 {
            offset((self.align, self.align_overflow), free, false, false)
        } else {
            0.0
        }
    }
}
