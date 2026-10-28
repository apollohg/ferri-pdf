//! CSS Grid 1 computed values: explicit track lists, named areas, auto-placement and the
//! grid-placement lines of items.
use crate::{
    LengthPercentage, StyleError,
    box_values::{length_percentage, unsupported},
    component::Component,
    flex_values::Alignment,
    grammar::{Keyword, Label, Production, Property},
};
use serde::Serialize;
use std::{ops::Range, sync::Arc};

mod subgrid;
pub use subgrid::{NameRepeat, Subgrid};

/// Index of the row (block) axis in per-axis grid arrays, matching `row-gap` in `gap`.
pub const ROWS: usize = 0;
/// Index of the column (inline) axis in per-axis grid arrays.
pub const COLUMNS: usize = 1;
/// CSS Grid 1 §5.4: the implicit grid is clamped to lines in [-10000, 10000].
pub const MAX_GRID_LINE: i32 = 10_000;
/// Explicit tracks beyond the limited grid are dropped while repeating.
const MAX_EXPLICIT_TRACKS: usize = MAX_GRID_LINE as usize - 1;
/// CSS Grid 1 §7.3.1: a sequence of full stops is a null cell token.
const NULL_CELL: char = '.';
const START_SUFFIX: &str = "-start";
const END_SUFFIX: &str = "-end";

#[derive(Clone, Debug, PartialEq, Serialize)]
pub enum TrackBreadth {
    Length(LengthPercentage),
    /// `fr`.
    Flex(f32),
    MinContent,
    MaxContent,
    Auto,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub enum TrackSize {
    Breadth(TrackBreadth),
    MinMax(TrackBreadth, TrackBreadth),
    FitContent(LengthPercentage),
}
impl TrackSize {
    pub const AUTO: Self = Self::Breadth(TrackBreadth::Auto);
}

/// Names of one grid line, in source order.
pub type LineNames = Vec<Arc<str>>;

/// `repeat(auto-fill | auto-fit, …)`, whose count depends on the grid container's size.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct AutoRepeat {
    /// Index of the first repeated track among the list's tracks.
    pub at: usize,
    /// `auto-fit` collapses repeated tracks left empty by placement.
    pub fit: bool,
    /// Names of the lines around and between the repeated tracks.
    pub lines: Vec<LineNames>,
    pub tracks: Vec<TrackSize>,
}

/// `grid-template-rows`/`-columns`; `none` has no tracks.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct TrackList {
    pub subgrid: Option<Subgrid>,
    /// Names of each line. With an auto repetition there is one more entry: `lines[at]`
    /// precedes the repeated tracks and `lines[at + 1]` follows them.
    pub lines: Vec<LineNames>,
    pub tracks: Vec<TrackSize>,
    pub repeat: Option<AutoRepeat>,
}

/// A named grid area from `grid-template-areas`, in track indices of the explicit grid.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct NamedArea {
    pub name: Arc<str>,
    /// Row and column tracks, indexed by [`ROWS`] and [`COLUMNS`].
    pub tracks: [Range<usize>; 2],
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct GridAreas {
    /// Rows and columns of the template, indexed by [`ROWS`] and [`COLUMNS`].
    pub size: [usize; 2],
    pub areas: Vec<NamedArea>,
}
impl GridAreas {
    /// Names the implicit `*-start` and `*-end` lines of each area give `line` (CSS Grid 1
    /// §7.3.2) along `axis`.
    pub fn line_names(&self, axis: usize, line: usize) -> impl Iterator<Item = String> + '_ {
        self.areas.iter().flat_map(move |area| {
            let tracks = &area.tracks[axis];
            [
                (tracks.start == line).then(|| format!("{}{START_SUFFIX}", area.name)),
                (tracks.end == line).then(|| format!("{}{END_SUFFIX}", area.name)),
            ]
            .into_iter()
            .flatten()
        })
    }
}

/// A grid-placement property (`grid-row-start` and the like).
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub enum GridLine {
    #[default]
    Auto,
    /// A bare `<custom-ident>`: a named area's edge, else the first line of that name.
    Name(Arc<str>),
    /// `<integer> <custom-ident>?`; the integer is not zero.
    Line(i32, Option<Arc<str>>),
    /// `span <integer>? <custom-ident>?`; the count is positive.
    Span(u32, Option<Arc<str>>),
}
impl GridLine {
    /// The suffixed line name a bare name tries first.
    pub fn area_line(name: &str, end: bool) -> String {
        format!("{name}{}", if end { END_SUFFIX } else { START_SUFFIX })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct AutoFlow {
    pub column: bool,
    pub dense: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct GridStyle {
    /// Explicit tracks, indexed by [`ROWS`] and [`COLUMNS`].
    pub templates: [Arc<TrackList>; 2],
    pub areas: Option<Arc<GridAreas>>,
    /// Implicit track sizes, indexed by [`ROWS`] and [`COLUMNS`].
    pub auto_tracks: [Arc<[TrackSize]>; 2],
    pub auto_flow: AutoFlow,
    /// Start and end lines, indexed by [`ROWS`] and [`COLUMNS`].
    pub placement: [[GridLine; 2]; 2],
    pub justify_items: Alignment,
    /// `justify-items: legacy …`, which descendants' `legacy` inherits (CSS Align 3 §6.1.3).
    pub legacy_justify: bool,
    pub justify_self: Alignment,
}
impl Default for GridStyle {
    fn default() -> Self {
        Self {
            templates: [Arc::default(), Arc::default()],
            areas: None,
            auto_tracks: [Arc::from([TrackSize::AUTO]), Arc::from([TrackSize::AUTO])],
            auto_flow: AutoFlow::default(),
            placement: Default::default(),
            justify_items: Alignment::Normal,
            legacy_justify: false,
            justify_self: Alignment::Auto,
        }
    }
}
impl GridStyle {
    /// The computed `justify-items` of a box that does not specify it: `legacy` inherits a
    /// legacy value and is `normal` otherwise.
    pub(crate) fn initial_justify_items(&mut self, parent: &Self) {
        (self.justify_items, self.legacy_justify) = if parent.legacy_justify {
            (parent.justify_items, true)
        } else {
            (Alignment::Normal, false)
        };
    }
}

/// Grammar productions that make up a track list, in source order.
const TRACK_PARTS: [Label; 6] = [
    Label::Production(Production::LineNames),
    Label::Production(Production::TrackSize),
    Label::Production(Production::FixedSize),
    Label::Production(Production::TrackRepeat),
    Label::Production(Production::FixedRepeat),
    Label::Production(Production::AutoRepeat),
];

#[derive(Default)]
struct TrackListBuilder {
    lines: Vec<LineNames>,
    tracks: Vec<TrackSize>,
    repeat: Option<AutoRepeat>,
}
impl TrackListBuilder {
    fn new() -> Self {
        Self {
            lines: vec![Vec::new()],
            ..Self::default()
        }
    }
    /// Adjacent line-name sets name the same line.
    fn names(&mut self, names: impl IntoIterator<Item = Arc<str>>) {
        self.lines
            .last_mut()
            .expect("a builder always has a current line")
            .extend(names);
    }
    fn track(&mut self, track: TrackSize) {
        if self.tracks.len() < MAX_EXPLICIT_TRACKS {
            self.tracks.push(track);
            self.lines.push(Vec::new());
        }
    }
    fn parts(
        &mut self,
        values: &[Component],
        relative: &crate::units::Relative<'_>,
    ) -> Result<(), StyleError> {
        let mut parts = Vec::new();
        for value in values {
            value.references(&TRACK_PARTS, &mut parts);
        }
        for (label, part) in parts {
            match label {
                Label::Production(Production::LineNames) => self.names(line_names(part)),
                Label::Production(Production::TrackSize | Production::FixedSize) => {
                    self.track(track_size(part, relative)?);
                }
                Label::Production(Production::TrackRepeat | Production::FixedRepeat) => {
                    let (count, body) = repeat_arguments(part)?;
                    let count = usize::try_from(crate::values::integer(count, relative)?)
                        .map_err(|_| unsupported(count))?;
                    let mut repeated = Self::new();
                    repeated.parts(body, relative)?;
                    for _ in 0..count.min(MAX_EXPLICIT_TRACKS) {
                        self.names(repeated.lines[0].iter().cloned());
                        for (track, after) in repeated.tracks.iter().zip(&repeated.lines[1..]) {
                            self.track(track.clone());
                            self.names(after.iter().cloned());
                        }
                    }
                }
                Label::Production(Production::AutoRepeat) => {
                    let (count, body) = repeat_arguments(part)?;
                    let mut repeated = Self::new();
                    repeated.parts(body, relative)?;
                    self.repeat = Some(AutoRepeat {
                        at: self.tracks.len(),
                        fit: count.keyword() == Some(Keyword::AutoFit),
                        lines: repeated.lines,
                        tracks: repeated.tracks,
                    });
                    self.lines.push(Vec::new());
                }
                _ => return Err(unsupported(part)),
            }
        }
        Ok(())
    }
    fn finish(self) -> TrackList {
        TrackList {
            subgrid: None,
            lines: self.lines,
            tracks: self.tracks,
            repeat: self.repeat,
        }
    }
}

/// `repeat( count , tracks )` arguments.
fn repeat_arguments(value: &Component) -> Result<(&Component, &[Component]), StyleError> {
    match value.arguments() {
        [count, Component::Delim(','), body @ ..] => Ok((count, body)),
        _ => Err(unsupported(value)),
    }
}

fn line_names(value: &Component) -> impl Iterator<Item = Arc<str>> + '_ {
    let body = match value.unwrap() {
        Component::Block(_, body) => body.items(),
        _ => &[],
    };
    body.iter().filter_map(|name| match name.unwrap() {
        Component::Ident(name) => Some(Arc::from(name.as_ref())),
        _ => None,
    })
}

fn breadth(
    value: &Component,
    relative: &crate::units::Relative<'_>,
) -> Result<TrackBreadth, StyleError> {
    Ok(match (value.keyword(), value.unwrap()) {
        (Some(Keyword::MinContent), _) => TrackBreadth::MinContent,
        (Some(Keyword::MaxContent), _) => TrackBreadth::MaxContent,
        (Some(Keyword::Auto), _) => TrackBreadth::Auto,
        (_, Component::Flex(factor)) if factor.is_finite() => TrackBreadth::Flex(*factor),
        _ => TrackBreadth::Length(length_percentage(value, relative)?),
    })
}

/// `<track-size>` or `<fixed-size>`.
fn track_size(
    value: &Component,
    relative: &crate::units::Relative<'_>,
) -> Result<TrackSize, StyleError> {
    let inner = value.unwrap();
    match inner {
        Component::Function(Keyword::Minmax, _) => match inner.arguments() {
            [minimum, Component::Delim(','), maximum] => Ok(TrackSize::MinMax(
                breadth(minimum, relative)?,
                breadth(maximum, relative)?,
            )),
            _ => Err(unsupported(value)),
        },
        Component::Function(Keyword::FitContent, Some(limit)) => {
            Ok(TrackSize::FitContent(length_percentage(limit, relative)?))
        }
        _ => breadth(value, relative).map(TrackSize::Breadth),
    }
}

/// Computed `grid-template-rows` and `-columns`.
pub(crate) fn template(
    value: &Component,
    relative: &crate::units::Relative<'_>,
) -> Result<TrackList, StyleError> {
    if value.keyword() == Some(Keyword::None) {
        return Ok(TrackList::default());
    }
    if value
        .find(Label::Production(Production::LineNameList))
        .is_some()
        || value.items().first().and_then(Component::keyword) == Some(Keyword::Subgrid)
    {
        return Ok(TrackList {
            subgrid: Some(Subgrid::parse(value, relative)?),
            ..Default::default()
        });
    }
    let mut builder = TrackListBuilder::new();
    builder.parts(std::slice::from_ref(value), relative)?;
    Ok(builder.finish())
}

/// `grid-auto-rows` and `-columns`.
pub(crate) fn auto_tracks(
    value: &Component,
    relative: &crate::units::Relative<'_>,
) -> Result<Arc<[TrackSize]>, StyleError> {
    let mut sizes = Vec::new();
    value.references(&[Label::Production(Production::TrackSize)], &mut sizes);
    sizes
        .into_iter()
        .map(|(_, size)| track_size(size, relative))
        .collect()
}

pub(crate) fn auto_flow(value: &Component) -> AutoFlow {
    let mut keywords = Vec::new();
    value.keywords(&mut keywords);
    AutoFlow {
        column: keywords.contains(&Keyword::Column),
        dense: keywords.contains(&Keyword::Dense),
    }
}

/// CSS Syntax 3 §4.2 whitespace.
fn is_css_whitespace(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r' | '\u{c}')
}
/// CSS Syntax 3 §4.2 name code points.
fn is_name_code_point(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '-' | '_') || !c.is_ascii()
}

/// Cells of one `grid-template-areas` string (CSS Grid 1 §7.3.1): `None` for null cells;
/// characters outside names, full stops and whitespace make the declaration invalid.
fn area_row(row: &str) -> Option<Vec<Option<&str>>> {
    let mut cells = Vec::new();
    let mut rest = row;
    loop {
        rest = rest.trim_start_matches(is_css_whitespace);
        let Some(first) = rest.chars().next() else {
            return Some(cells);
        };
        let (cell, length) = if first == NULL_CELL {
            (None, rest.len() - rest.trim_start_matches(NULL_CELL).len())
        } else {
            let length = rest
                .find(|c: char| !is_name_code_point(c))
                .unwrap_or(rest.len());
            if length == 0 {
                return None;
            }
            (Some(&rest[..length]), length)
        };
        cells.push(cell);
        rest = &rest[length..];
    }
}

/// Named areas of `rows`, or `None` when rows differ in length, are empty, or an area is not
/// a filled rectangle.
fn named_areas<'a>(rows: impl IntoIterator<Item = &'a str>) -> Option<GridAreas> {
    let rows = rows.into_iter().map(area_row).collect::<Option<Vec<_>>>()?;
    let columns = rows.first()?.len();
    if columns == 0 || rows.iter().any(|row| row.len() != columns) {
        return None;
    }
    let mut areas: Vec<NamedArea> = Vec::new();
    for (row, cells) in rows.iter().enumerate() {
        for (column, cell) in cells.iter().enumerate() {
            let Some(name) = cell else { continue };
            match areas.iter_mut().find(|area| *area.name == **name) {
                Some(area) => {
                    let [rows, columns] = &mut area.tracks;
                    rows.end = rows.end.max(row + 1);
                    columns.end = columns.end.max(column + 1);
                    columns.start = columns.start.min(column);
                }
                None => areas.push(NamedArea {
                    name: Arc::from(*name),
                    tracks: [row..row + 1, column..column + 1],
                }),
            }
        }
    }
    let filled = areas.iter().all(|area| {
        let [area_rows, area_columns] = &area.tracks;
        let count = rows
            .iter()
            .flat_map(|cells| cells.iter())
            .filter(|cell| **cell == Some(&*area.name))
            .count();
        count == area_rows.len() * area_columns.len()
            && area_rows.clone().all(|row| {
                area_columns
                    .clone()
                    .all(|column| rows[row][column] == Some(&*area.name))
            })
    });
    filled.then_some(GridAreas {
        size: [rows.len(), columns],
        areas,
    })
}

fn strings(value: &Component) -> impl Iterator<Item = &str> {
    value.items().iter().filter_map(|item| match item.unwrap() {
        Component::String(text) => Some(text.as_ref()),
        _ => None,
    })
}

/// `grid-template-areas`.
pub(crate) fn areas(value: &Component) -> Result<Option<Arc<GridAreas>>, StyleError> {
    if value.keyword() == Some(Keyword::None) {
        return Ok(None);
    }
    named_areas(strings(value))
        .map(|areas| Some(Arc::new(areas)))
        .ok_or_else(|| StyleError::Invalid(format!("grid-template-areas: {value}")))
}

/// `<grid-line>`.
pub(crate) fn line(
    value: &Component,
    relative: &crate::units::Relative<'_>,
) -> Result<GridLine, StyleError> {
    let name = |component: Option<&Component>| {
        component.and_then(|component| {
            component
                .items()
                .iter()
                .find_map(|item| match item.unwrap() {
                    Component::Ident(name) => Some(Arc::<str>::from(name.as_ref())),
                    _ => None,
                })
        })
    };
    let inner = value.unwrap();
    if inner.keyword() == Some(Keyword::Auto) {
        return Ok(GridLine::Auto);
    }
    if let Component::Ident(text) = inner {
        return Ok(GridLine::Name(Arc::from(text.as_ref())));
    }
    if inner.slot(0).and_then(Component::keyword) == Some(Keyword::Span) {
        let parts = inner.slot(1);
        let count = match parts.and_then(|parts| parts.slot(0)) {
            Some(count) => u32::try_from(crate::values::integer(count, relative)?)
                .ok()
                .filter(|count| *count > 0)
                .ok_or_else(|| format!("non-positive grid span: {value}"))?,
            None => 1,
        };
        return Ok(GridLine::Span(
            count,
            name(parts.and_then(|parts| parts.slot(1))),
        ));
    }
    let index = crate::values::integer(inner.slot(0).ok_or_else(|| unsupported(value))?, relative)?;
    if index == 0 {
        return Err(format!("grid line zero: {value}").into());
    }
    Ok(GridLine::Line(index, name(inner.slot(1))))
}

/// `justify-self`, and `justify-items` without `legacy`.
pub(crate) fn justify(value: &Component) -> Result<Alignment, StyleError> {
    Alignment::from_component(value)
}

/// `justify-items`: the alignment and whether it is `legacy`; a lone `legacy` is `None`,
/// computed from the parent.
pub(crate) fn justify_items(value: &Component) -> Result<Option<(Alignment, bool)>, StyleError> {
    let mut keywords = Vec::new();
    value.keywords(&mut keywords);
    match keywords.as_slice() {
        [Keyword::Legacy] => Ok(None),
        [Keyword::Legacy, keyword] => Ok(Some((
            match keyword {
                Keyword::Left => Alignment::Left,
                Keyword::Right => Alignment::Right,
                Keyword::Center => Alignment::Center,
                _ => return Err(unsupported(value)),
            },
            true,
        ))),
        _ => Ok(Some((Alignment::from_component(value)?, false))),
    }
}

/// Validity the grammar cannot express (CSS Grid 1 §7.3, §8.3): a grid line's integer is
/// not zero, a span is positive, and template strings form rectangular areas.
pub(crate) fn valid(property: Property, value: &Component) -> bool {
    let mut lines = Vec::new();
    value.references(&[Label::Production(Production::GridLine)], &mut lines);
    let lines_valid = lines.iter().all(|(_, line)| {
        let integer = |component: Option<&Component>| match component.map(Component::unwrap) {
            Some(Component::Integer(integer)) => Some(*integer),
            _ => None,
        };
        let line = line.unwrap();
        if line.slot(0).and_then(Component::keyword) == Some(Keyword::Span) {
            integer(line.slot(1).and_then(|parts| parts.slot(0))).is_none_or(|count| count > 0)
        } else {
            integer(line.slot(0)) != Some(0)
        }
    });
    let areas_valid = match property {
        Property::GridTemplateAreas | Property::GridTemplate | Property::Grid => {
            let rows: Vec<&str> = template_strings(value).collect();
            rows.is_empty() || named_areas(rows).is_some()
        }
        _ => true,
    };
    lines_valid && areas_valid && subgrid::valid(value)
}

/// Area strings anywhere in a template value, in source order.
fn template_strings(value: &Component) -> Box<dyn Iterator<Item = &str> + '_> {
    match value {
        Component::String(text) => Box::new(std::iter::once(text.as_ref())),
        Component::Reference(_, inner) => template_strings(inner),
        Component::Group(items) => Box::new(items.iter().flat_map(template_strings)),
        _ => Box::new(std::iter::empty()),
    }
}
