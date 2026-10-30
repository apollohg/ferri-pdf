//! CSS Anchor Positioning 1: anchor names and scopes (§2.1–2.2), default anchors (§2.4),
//! position areas (§3.1), the anchor functions (§3.2, §5.1), position options (§6.1–6.5)
//! and conditional hiding (§6.6).
//!
//! An anchor function resolves at computed-value time from the anchor boxes the previous
//! layout pass measured, which the document restyles with until a layout reproduces every
//! measurement its styles consulted, as container query lengths do (§9).
use crate::dom::ElementId;
use crate::fonts::FontNamespace;
use crate::paint::Rect;
use crate::style::{
    Alignment, PageStyleTarget, Size, Style, StyleError,
    box_values::unsupported,
    component::Component,
    grammar::{Keyword, Label, Production, Property},
    logical::{BOTTOM, Flow, LEFT, RIGHT, TOP},
    units::Relative,
};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    sync::Arc,
};

const ANCHOR_FUNCTION: &str = "anchor";
const ANCHOR_SIZE_FUNCTION: &str = "anchor-size";
const CALC_FUNCTION: &str = "calc";
/// `center` is the anchor side halfway between start and end (§3.2).
const CENTER_FRACTION: f32 = 0.5;
/// Physical axes: `width` and horizontal sides, then `height` and vertical sides.
pub(crate) const HORIZONTAL: usize = 0;
pub(crate) const VERTICAL: usize = 1;
/// Position-area tracks in start-to-end order (§3.1.1).
const START_TRACK: usize = 0;
const CENTER_TRACK: usize = 1;
const END_TRACK: usize = 2;
const SIDES: [usize; 4] = [TOP, RIGHT, BOTTOM, LEFT];

/// A tree-scoped anchor name (§2.1): its `<dashed-ident>` and, as for counter style names,
/// the tree scope of the element it applies to.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct AnchorName {
    pub(crate) name: Arc<str>,
    pub(crate) scope: FontNamespace,
}

/// `anchor-scope` (§2.2).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) enum AnchorScope {
    #[default]
    None,
    /// Every name of the element's own tree scope.
    All(FontNamespace),
    Names(Arc<[AnchorName]>),
}
impl AnchorScope {
    /// Whether the scope limits `name`, which it matches strictly (§2.2).
    pub(crate) fn covers(&self, name: &AnchorName) -> bool {
        match self {
            Self::None => false,
            Self::All(scope) => *scope == name.scope,
            Self::Names(names) => names.contains(name),
        }
    }
}

/// `position-anchor` (§2.4).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) enum PositionAnchor {
    #[default]
    Normal,
    None,
    Auto,
    Named(AnchorName),
    MatchParent,
}

/// What an anchor reference names: an anchor name, the implicit anchor element, or the
/// parent's default anchor element (`match-parent`).
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) enum AnchorReference {
    Named(AnchorName),
    Implicit,
    Parent,
}

/// Which keywords' direction a position-area axis uses (§3.1.2).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AreaReference {
    Physical,
    /// The containing block's writing mode.
    Containing,
    /// The box's own writing mode.
    Own,
}

/// The tracks one axis of a `<position-area>` selects, in its reference's start-to-end order.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct AreaTracks {
    first: usize,
    last: usize,
    reference: AreaReference,
}
impl AreaTracks {
    const ALL: Self = Self {
        first: START_TRACK,
        last: END_TRACK,
        reference: AreaReference::Physical,
    };
    fn physical(self, axis: usize, own: Flow, containing: Flow) -> [usize; 2] {
        let mirrored = match self.reference {
            AreaReference::Physical => false,
            AreaReference::Containing => containing.axis_start(axis) != start_side_of(axis),
            AreaReference::Own => own.axis_start(axis) != start_side_of(axis),
        };
        if mirrored {
            [END_TRACK - self.last, END_TRACK - self.first]
        } else {
            [self.first, self.last]
        }
    }
}

/// A computed `<position-area>`: the tracks of the horizontal, then the vertical axis.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PositionArea([AreaTerm; 2]);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct AreaTerm {
    axis: AreaAxis,
    tracks: AreaTracks,
}
impl AreaTerm {
    fn axis(self, own: Flow, containing: Flow) -> Option<usize> {
        let flow = if self.tracks.reference == AreaReference::Own {
            own
        } else {
            containing
        };
        match self.axis {
            AreaAxis::X => Some(HORIZONTAL),
            AreaAxis::Y => Some(VERTICAL),
            AreaAxis::Block => Some(flow.block_axis()),
            AreaAxis::Inline => Some(1 - flow.block_axis()),
            AreaAxis::Either => None,
        }
    }
}

/// The default self-alignment a position area gives one axis (§4.1), physically.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AreaAlignment {
    /// Toward the left or top edge.
    Start,
    Center,
    /// Toward the right or bottom edge.
    End,
    AnchorCenter,
}

impl PositionArea {
    /// The first and last physical track of each axis, left to right and top to bottom.
    pub(crate) fn tracks(self, own: Flow, containing: Flow) -> [[usize; 2]; 2] {
        let axes = match (
            self.0[0].axis(own, containing),
            self.0[1].axis(own, containing),
        ) {
            (Some(first), _) => [first, 1 - first],
            (None, Some(second)) => [1 - second, second],
            (None, None) => [containing.block_axis(), 1 - containing.block_axis()],
        };
        let mut tracks = [[START_TRACK, END_TRACK]; 2];
        for (term, axis) in self.0.into_iter().zip(axes) {
            tracks[axis] = term.tracks.physical(axis, own, containing);
        }
        tracks
    }

    fn from_physical(tracks: [[usize; 2]; 2]) -> Self {
        Self(std::array::from_fn(|axis| AreaTerm {
            axis: if axis == HORIZONTAL {
                AreaAxis::X
            } else {
                AreaAxis::Y
            },
            tracks: AreaTracks {
                first: tracks[axis][0],
                last: tracks[axis][1],
                reference: AreaReference::Physical,
            },
        }))
    }

    fn physical(self, own: Flow, containing: Flow) -> Self {
        Self::from_physical(self.tracks(own, containing))
    }
    /// The region of the position-area grid the area selects (§3.1.1), in the coordinates of
    /// a containing block of size `containing` with the default anchor box at `anchor`.
    pub(crate) fn rect(
        self,
        containing: [f32; 2],
        anchor: Rect,
        own: Flow,
        containing_flow: Flow,
    ) -> Rect {
        let tracks = self.tracks(own, containing_flow);
        let lines = |axis: usize| {
            let (start, end) = if axis == HORIZONTAL {
                (anchor.x, anchor.x + anchor.width)
            } else {
                (anchor.y, anchor.y + anchor.height)
            };
            [start.min(0.0), start, end, end.max(containing[axis])]
        };
        let [x, y] = [HORIZONTAL, VERTICAL].map(|axis| {
            let lines = lines(axis);
            let [first, last] = tracks[axis];
            [lines[first], lines[last + 1]]
        });
        Rect {
            x: x[0],
            y: y[0],
            width: x[1] - x[0],
            height: y[1] - y[0],
        }
    }
    /// §4.1: the alignment `normal` resolves to in each axis.
    pub(crate) fn alignment(self, own: Flow, containing: Flow) -> [AreaAlignment; 2] {
        self.tracks(own, containing).map(|tracks| match tracks {
            [CENTER_TRACK, CENTER_TRACK] => AreaAlignment::Center,
            [START_TRACK, END_TRACK] => AreaAlignment::AnchorCenter,
            [START_TRACK, _] => AreaAlignment::End,
            _ => AreaAlignment::Start,
        })
    }
    /// The area selecting the same grid region once `tactic` moves its sides (§6.5.2).
    fn transformed(self, tactic: Tactic, own: Flow, containing: Flow) -> Self {
        let tracks = self.tracks(own, containing);
        let mut moved = [AreaTracks::ALL; 2];
        for axis in [HORIZONTAL, VERTICAL] {
            let start_side = if axis == HORIZONTAL { LEFT } else { TOP };
            let side = tactic.side(start_side);
            let target = axis_of(side);
            let [first, last] = tracks[axis];
            let [first, last] = if side == start_side_of(target) {
                [first, last]
            } else {
                [END_TRACK - last, END_TRACK - first]
            };
            moved[target] = AreaTracks {
                first,
                last,
                reference: AreaReference::Physical,
            };
        }
        Self::from_physical(moved.map(|tracks| [tracks.first, tracks.last]))
    }
}

fn axis_of(side: usize) -> usize {
    if side == LEFT || side == RIGHT {
        HORIZONTAL
    } else {
        VERTICAL
    }
}
fn start_side_of(axis: usize) -> usize {
    if axis == HORIZONTAL { LEFT } else { TOP }
}
fn opposite(side: usize) -> usize {
    match side {
        TOP => BOTTOM,
        BOTTOM => TOP,
        LEFT => RIGHT,
        _ => LEFT,
    }
}

/// The axis an ambiguous position-area keyword takes from its partner.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AreaAxis {
    X,
    Y,
    Block,
    Inline,
    Either,
}

/// One position-area keyword: its axis, tracks and reference.
fn area_keyword(keyword: Keyword) -> Option<(AreaAxis, usize, usize, AreaReference)> {
    use AreaAxis::{Block, Either, Inline, X, Y};
    use AreaReference::{Containing, Own, Physical};
    use Keyword as K;
    let (start, center, end, span_start, span_end) = (
        (START_TRACK, START_TRACK),
        (CENTER_TRACK, CENTER_TRACK),
        (END_TRACK, END_TRACK),
        (START_TRACK, CENTER_TRACK),
        (CENTER_TRACK, END_TRACK),
    );
    let ((first, last), axis, reference) = match keyword {
        K::Left => (start, X, Physical),
        K::Right => (end, X, Physical),
        K::SpanLeft => (span_start, X, Physical),
        K::SpanRight => (span_end, X, Physical),
        K::Top => (start, Y, Physical),
        K::Bottom => (end, Y, Physical),
        K::SpanTop => (span_start, Y, Physical),
        K::SpanBottom => (span_end, Y, Physical),
        K::XStart => (start, X, Containing),
        K::InlineStart => (start, Inline, Containing),
        K::XEnd => (end, X, Containing),
        K::InlineEnd => (end, Inline, Containing),
        K::SpanXStart => (span_start, X, Containing),
        K::SpanInlineStart => (span_start, Inline, Containing),
        K::SpanXEnd => (span_end, X, Containing),
        K::SpanInlineEnd => (span_end, Inline, Containing),
        K::SelfXStart => (start, X, Own),
        K::SelfInlineStart => (start, Inline, Own),
        K::SelfXEnd => (end, X, Own),
        K::SelfInlineEnd => (end, Inline, Own),
        K::SpanSelfXStart => (span_start, X, Own),
        K::SpanSelfInlineStart => (span_start, Inline, Own),
        K::SpanSelfXEnd => (span_end, X, Own),
        K::SpanSelfInlineEnd => (span_end, Inline, Own),
        K::YStart => (start, Y, Containing),
        K::BlockStart => (start, Block, Containing),
        K::YEnd => (end, Y, Containing),
        K::BlockEnd => (end, Block, Containing),
        K::SpanYStart => (span_start, Y, Containing),
        K::SpanBlockStart => (span_start, Block, Containing),
        K::SpanYEnd => (span_end, Y, Containing),
        K::SpanBlockEnd => (span_end, Block, Containing),
        K::SelfYStart => (start, Y, Own),
        K::SelfBlockStart => (start, Block, Own),
        K::SelfYEnd => (end, Y, Own),
        K::SelfBlockEnd => (end, Block, Own),
        K::SpanSelfYStart => (span_start, Y, Own),
        K::SpanSelfBlockStart => (span_start, Block, Own),
        K::SpanSelfYEnd => (span_end, Y, Own),
        K::SpanSelfBlockEnd => (span_end, Block, Own),
        K::Start => (start, Either, Containing),
        K::End => (end, Either, Containing),
        K::SpanStart => (span_start, Either, Containing),
        K::SpanEnd => (span_end, Either, Containing),
        K::SelfStart => (start, Either, Own),
        K::SelfEnd => (end, Either, Own),
        K::SpanSelfStart => (span_start, Either, Own),
        K::SpanSelfEnd => (span_end, Either, Own),
        K::Center => (center, Either, Physical),
        K::SpanAll => ((START_TRACK, END_TRACK), Either, Physical),
        _ => return None,
    };
    Some((axis, first, last, reference))
}

/// `position-area`; `None` is `none`. §3.1.2: an ambiguous keyword takes the axis its
/// partner does not; two ambiguous keywords are the block axis, then the inline axis; a lone
/// unambiguous keyword pairs with `span-all`, and a lone ambiguous one repeats.
pub(crate) fn position_area(value: &Component) -> Result<Option<PositionArea>, StyleError> {
    if value.keyword() == Some(Keyword::None) {
        return Ok(None);
    }
    let mut keywords = Vec::new();
    value.keywords(&mut keywords);
    let parsed = keywords
        .iter()
        .map(|keyword| area_keyword(*keyword))
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| unsupported(value))?;
    let term = |(axis, first, last, reference)| AreaTerm {
        axis,
        tracks: AreaTracks {
            first,
            last,
            reference,
        },
    };
    let all = AreaTerm {
        axis: AreaAxis::Either,
        tracks: AreaTracks::ALL,
    };
    let terms = match parsed.as_slice() {
        [single] if single.0 == AreaAxis::Either => [term(*single); 2],
        [single] => [term(*single), all],
        [first, second] => [term(*first), term(*second)],
        _ => return Err(unsupported(value)),
    };
    Ok(Some(PositionArea(terms)))
}

/// A `<try-tactic>` keyword (§6.1).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TacticKeyword {
    FlipBlock,
    FlipInline,
    FlipStart,
    FlipX,
    FlipY,
}
impl TacticKeyword {
    fn from_keyword(keyword: Keyword) -> Option<Self> {
        Some(match keyword {
            Keyword::FlipBlock => Self::FlipBlock,
            Keyword::FlipInline => Self::FlipInline,
            Keyword::FlipStart => Self::FlipStart,
            Keyword::FlipX => Self::FlipX,
            Keyword::FlipY => Self::FlipY,
            _ => return None,
        })
    }
}

/// A composed try tactic as the physical side each side's values move to (§6.5.2).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Tactic([usize; 4]);
impl Default for Tactic {
    fn default() -> Self {
        Self::IDENTITY
    }
}
impl Tactic {
    pub(crate) const IDENTITY: Self = Self(SIDES);
    /// The keywords composed in order, their logical directions resolved against the
    /// containing block's writing mode.
    pub(crate) fn of(keywords: &[TacticKeyword], containing: Flow) -> Self {
        keywords.iter().fold(Self::IDENTITY, |tactic, keyword| {
            let swap = |pairs: &[[usize; 2]]| {
                let mut sides = SIDES;
                for [a, b] in pairs {
                    sides[*a] = *b;
                    sides[*b] = *a;
                }
                Self(sides)
            };
            let block = containing.axis_start(containing.block_axis());
            let inline = containing.axis_start(1 - containing.block_axis());
            let step = match keyword {
                TacticKeyword::FlipBlock => swap(&[[block, opposite(block)]]),
                TacticKeyword::FlipInline => swap(&[[inline, opposite(inline)]]),
                TacticKeyword::FlipStart => {
                    swap(&[[block, inline], [opposite(block), opposite(inline)]])
                }
                TacticKeyword::FlipX => swap(&[[LEFT, RIGHT]]),
                TacticKeyword::FlipY => swap(&[[TOP, BOTTOM]]),
            };
            Self(tactic.0.map(|side| step.0[side]))
        })
    }
    pub(crate) fn side(self, side: usize) -> usize {
        self.0[side]
    }
    fn source(self, side: usize) -> usize {
        SIDES
            .into_iter()
            .find(|source| self.0[*source] == side)
            .expect("a tactic permutes the sides")
    }
    /// Whether the tactic moves horizontal values into the vertical axis.
    pub(crate) fn swaps_axes(self) -> bool {
        axis_of(self.0[LEFT]) == VERTICAL
    }
    pub(crate) fn inherited_basis(self, parent: &Style) -> Style {
        let mut moved = parent.clone();
        moved.inset = std::array::from_fn(|side| parent.inset[self.source(side)].clone());
        moved.margin = std::array::from_fn(|side| parent.margin[self.source(side)].clone());
        if self.swaps_axes() {
            std::mem::swap(&mut moved.width, &mut moved.height);
            std::mem::swap(&mut moved.min_width, &mut moved.min_height);
            std::mem::swap(&mut moved.max_width, &mut moved.max_height);
            std::mem::swap(&mut moved.grid.justify_self, &mut moved.flex.align_self);
        }
        moved
    }

    /// The physical longhand whose value `property` takes under the tactic, for the
    /// properties a tactic moves between sides and axes.
    pub(crate) fn source_property(self, property: Property) -> Property {
        let groups: [[Property; 4]; 2] = [
            [
                Property::Top,
                Property::Right,
                Property::Bottom,
                Property::Left,
            ],
            [
                Property::MarginTop,
                Property::MarginRight,
                Property::MarginBottom,
                Property::MarginLeft,
            ],
        ];
        for group in groups {
            if let Some(side) = group.iter().position(|member| *member == property) {
                return group[self.source(side)];
            }
        }
        let pairs = [
            [Property::Width, Property::Height],
            [Property::MinWidth, Property::MinHeight],
            [Property::MaxWidth, Property::MaxHeight],
            [Property::JustifySelf, Property::AlignSelf],
        ];
        if self.swaps_axes()
            && let Some(pair) = pairs.iter().find(|pair| pair.contains(&property))
        {
            return if pair[0] == property {
                pair[1]
            } else {
                pair[0]
            };
        }
        property
    }
}

/// One entry of `position-try-fallbacks` (§6.1).
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum TryOption {
    /// A `@position-try` rule, a try tactic, or both.
    Rule {
        rule: Option<AnchorName>,
        tactic: Arc<[TacticKeyword]>,
    },
    /// An option of only this `position-area` value.
    Area(Arc<Component>),
}

/// `position-try-fallbacks`; empty is `none`.
pub(crate) fn try_fallbacks(
    value: &Component,
    scope: FontNamespace,
) -> Result<Arc<[TryOption]>, StyleError> {
    if value.keyword() == Some(Keyword::None) {
        return Ok(Arc::from([]));
    }
    value
        .items()
        .iter()
        .map(|option| {
            if position_area(option).is_ok() {
                return Ok(TryOption::Area(Arc::new(option.clone())));
            }
            let rule = option
                .slot(0)
                .map(|name| dashed_name(name, scope))
                .transpose()?;
            let mut keywords = Vec::new();
            if let Some(tactic) = option.slot(1) {
                tactic.keywords(&mut keywords);
            }
            let tactic = keywords
                .into_iter()
                .map(TacticKeyword::from_keyword)
                .collect::<Option<Arc<[_]>>>()
                .ok_or_else(|| unsupported(option))?;
            Ok(TryOption::Rule { rule, tactic })
        })
        .collect()
}

/// `position-try-order` (§6.2); logical axes retain their containing-block basis.
pub(crate) fn try_order(value: &Component) -> Result<Option<AnchorDimension>, StyleError> {
    match value.keyword() {
        Some(Keyword::Normal) => Ok(None),
        Some(Keyword::MostWidth) => Ok(Some(AnchorDimension::Width)),
        Some(Keyword::MostHeight) => Ok(Some(AnchorDimension::Height)),
        Some(Keyword::MostBlockSize) => Ok(Some(AnchorDimension::Block)),
        Some(Keyword::MostInlineSize) => Ok(Some(AnchorDimension::Inline)),
        _ => Err(unsupported(value)),
    }
}

/// `position-visibility` (§6.6); every flag clear is `always`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PositionVisibility {
    pub(crate) anchors_valid: bool,
    pub(crate) anchors_visible: bool,
    pub(crate) no_overflow: bool,
}
impl Default for PositionVisibility {
    fn default() -> Self {
        Self {
            anchors_valid: false,
            anchors_visible: true,
            no_overflow: false,
        }
    }
}
pub(crate) fn position_visibility(value: &Component) -> Result<PositionVisibility, StyleError> {
    let mut keywords = Vec::new();
    value.keywords(&mut keywords);
    let mut visibility = PositionVisibility {
        anchors_valid: false,
        anchors_visible: false,
        no_overflow: false,
    };
    for keyword in keywords {
        match keyword {
            Keyword::Always => {}
            Keyword::AnchorsValid => visibility.anchors_valid = true,
            Keyword::AnchorsVisible => visibility.anchors_visible = true,
            Keyword::NoOverflow => visibility.no_overflow = true,
            _ => return Err(unsupported(value)),
        }
    }
    Ok(visibility)
}

fn dashed_name(value: &Component, scope: FontNamespace) -> Result<AnchorName, StyleError> {
    match value.unwrap() {
        Component::Ident(name) => Ok(AnchorName {
            name: Arc::from(name.as_ref()),
            scope,
        }),
        other => Err(unsupported(other)),
    }
}
fn names(value: &Component, scope: FontNamespace) -> Result<Arc<[AnchorName]>, StyleError> {
    value
        .items()
        .iter()
        .map(|name| dashed_name(name, scope))
        .collect()
}

/// `anchor-name`; empty is `none`.
pub(crate) fn anchor_names(
    value: &Component,
    scope: FontNamespace,
) -> Result<Arc<[AnchorName]>, StyleError> {
    if value.keyword() == Some(Keyword::None) {
        return Ok(Arc::from([]));
    }
    names(value, scope)
}
pub(crate) fn anchor_scope(
    value: &Component,
    scope: FontNamespace,
) -> Result<AnchorScope, StyleError> {
    match value.keyword() {
        Some(Keyword::None) => Ok(AnchorScope::None),
        Some(Keyword::All) => Ok(AnchorScope::All(scope)),
        _ => names(value, scope).map(AnchorScope::Names),
    }
}
pub(crate) fn position_anchor(
    value: &Component,
    scope: FontNamespace,
) -> Result<PositionAnchor, StyleError> {
    match value.keyword() {
        Some(Keyword::Normal) => Ok(PositionAnchor::Normal),
        Some(Keyword::None) => Ok(PositionAnchor::None),
        Some(Keyword::Auto) => Ok(PositionAnchor::Auto),
        Some(Keyword::MatchParent) => Ok(PositionAnchor::MatchParent),
        _ => dashed_name(value, scope).map(PositionAnchor::Named),
    }
}

/// CSS Anchor Positioning 1 §6.4: the properties a position option may override.
pub(crate) fn accepted(property: Property) -> bool {
    use Property as P;
    matches!(
        property,
        P::Top
            | P::Right
            | P::Bottom
            | P::Left
            | P::InsetBlockStart
            | P::InsetBlockEnd
            | P::InsetInlineStart
            | P::InsetInlineEnd
            | P::MarginTop
            | P::MarginRight
            | P::MarginBottom
            | P::MarginLeft
            | P::MarginBlockStart
            | P::MarginBlockEnd
            | P::MarginInlineStart
            | P::MarginInlineEnd
            | P::Width
            | P::MinWidth
            | P::MaxWidth
            | P::Height
            | P::MinHeight
            | P::MaxHeight
            | P::BlockSize
            | P::MinBlockSize
            | P::MaxBlockSize
            | P::InlineSize
            | P::MinInlineSize
            | P::MaxInlineSize
            | P::AlignSelf
            | P::JustifySelf
            | P::PositionAnchor
            | P::PositionArea
    )
}

/// Tree-scoped names retain the namespace of their declaring rule (§2.1, §6.4).
pub(super) fn namespaced(property: Property, value: &crate::style::declaration::Specified) -> bool {
    match value {
        crate::style::declaration::Specified::Value(value) => match property {
            Property::AnchorName | Property::AnchorScope | Property::PositionTryFallbacks => {
                value.keyword() != Some(Keyword::None)
            }
            Property::PositionAnchor => value.keyword().is_none(),
            _ => mentions_anchor(value),
        },
        crate::style::declaration::Specified::Toggle(values) => values
            .iter()
            .any(|value| namespaced(property, &value.value)),
        _ => false,
    }
}

/// Validity the grammar cannot express: each `<try-tactic>` keyword occurs once (§6.1).
pub(crate) fn valid(property: Property, value: &Component) -> bool {
    if !matches!(
        property,
        Property::PositionTryFallbacks | Property::PositionTry
    ) {
        return true;
    }
    let mut tactics = Vec::new();
    value.references(&[Label::Production(Production::TryTactic)], &mut tactics);
    tactics.iter().all(|(_, tactic)| {
        let mut keywords = Vec::new();
        tactic.keywords(&mut keywords);
        keywords
            .iter()
            .enumerate()
            .all(|(index, keyword)| !keywords[..index].contains(keyword))
    })
}

/// The anchor functions a property's values may contain (§3.2, §5.1).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct AnchorFunctions {
    pub(crate) anchor: bool,
    pub(crate) size: bool,
}
impl AnchorFunctions {
    pub(crate) fn allows(self, name: &str) -> bool {
        (self.anchor && name.eq_ignore_ascii_case(ANCHOR_FUNCTION))
            || (self.size && name.eq_ignore_ascii_case(ANCHOR_SIZE_FUNCTION))
    }
}
/// The functions the longhands of `property` accept, flow-relative ones as their physical
/// counterparts.
pub(crate) fn functions(property: Property) -> AnchorFunctions {
    crate::style::declaration::longhands(property).fold(
        AnchorFunctions::default(),
        |functions, longhand| {
            let target = target(crate::style::logical::physical(longhand, Flow::default()));
            let allowed = target.map(AnchorTarget::functions).unwrap_or_default();
            AnchorFunctions {
                anchor: functions.anchor || allowed.anchor,
                size: functions.size || allowed.size,
            }
        },
    )
}

/// The property an anchor function's value is computed for.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AnchorTarget {
    Inset(usize),
    Margin(usize),
    /// A sizing property of the physical axis.
    Size(usize),
}
impl AnchorTarget {
    fn functions(self) -> AnchorFunctions {
        AnchorFunctions {
            anchor: matches!(self, Self::Inset(_)),
            size: true,
        }
    }
    fn axis(self) -> usize {
        match self {
            Self::Inset(side) | Self::Margin(side) => axis_of(side),
            Self::Size(axis) => axis,
        }
    }
    /// The value of an invalid-at-computed-value-time declaration: the initial value.
    fn initial(self) -> Size {
        match self {
            Self::Margin(_) => Size::ZERO,
            Self::Inset(_) | Self::Size(_) => Size::Auto,
        }
    }
}
fn target(property: Property) -> Option<AnchorTarget> {
    use Property as P;
    Some(match property {
        P::Top => AnchorTarget::Inset(TOP),
        P::Right => AnchorTarget::Inset(RIGHT),
        P::Bottom => AnchorTarget::Inset(BOTTOM),
        P::Left => AnchorTarget::Inset(LEFT),
        P::MarginTop => AnchorTarget::Margin(TOP),
        P::MarginRight => AnchorTarget::Margin(RIGHT),
        P::MarginBottom => AnchorTarget::Margin(BOTTOM),
        P::MarginLeft => AnchorTarget::Margin(LEFT),
        P::Width | P::MinWidth | P::MaxWidth => AnchorTarget::Size(HORIZONTAL),
        P::Height | P::MinHeight | P::MaxHeight => AnchorTarget::Size(VERTICAL),
        _ => return None,
    })
}

/// An `<anchor-side>` (§3.2).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum AnchorSide {
    Inside,
    Outside,
    Physical(usize),
    Start,
    End,
    SelfStart,
    SelfEnd,
    /// A `<percentage>` or `center`, from start to end.
    Fraction(f32),
}
impl AnchorSide {
    pub(crate) fn from_name(name: &str) -> Option<Self> {
        Some(match name.to_ascii_lowercase().as_str() {
            "inside" => Self::Inside,
            "outside" => Self::Outside,
            "top" => Self::Physical(TOP),
            "right" => Self::Physical(RIGHT),
            "bottom" => Self::Physical(BOTTOM),
            "left" => Self::Physical(LEFT),
            "start" => Self::Start,
            "end" => Self::End,
            "self-start" => Self::SelfStart,
            "self-end" => Self::SelfEnd,
            "center" => Self::Fraction(CENTER_FRACTION),
            _ => return None,
        })
    }
}

/// An `<anchor-size>` (§5.1).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AnchorDimension {
    Width,
    Height,
    Block,
    Inline,
    SelfBlock,
    SelfInline,
}
impl AnchorDimension {
    pub(crate) fn from_name(name: &str) -> Option<Self> {
        Some(match name.to_ascii_lowercase().as_str() {
            "width" => Self::Width,
            "height" => Self::Height,
            "block" => Self::Block,
            "inline" => Self::Inline,
            "self-block" => Self::SelfBlock,
            "self-inline" => Self::SelfInline,
            _ => return None,
        })
    }
    pub(crate) fn axis(self, containing: Flow, own: Flow) -> usize {
        match self {
            Self::Width => HORIZONTAL,
            Self::Height => VERTICAL,
            Self::Block => containing.block_axis(),
            Self::Inline => 1 - containing.block_axis(),
            Self::SelfBlock => own.block_axis(),
            Self::SelfInline => 1 - own.block_axis(),
        }
    }
}

/// One anchor function's arguments, as the math parser reads them.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum AnchorQuery {
    Anchor {
        name: Option<Arc<str>>,
        side: AnchorSide,
    },
    Size {
        name: Option<Arc<str>>,
        dimension: Option<AnchorDimension>,
    },
}

/// An anchor box in the coordinates of the positioned box's containing block, and whether it
/// is visible to it (§6.6).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct AnchorBox {
    pub(crate) rect: Rect,
    pub(crate) visible: bool,
}

/// Static lookup data for an element, including elements that generate no principal box.
#[derive(Clone)]
pub(crate) struct AnchorNode {
    pub(crate) multicol: bool,
    pub(crate) has_box: bool,
    pub(crate) visible: bool,
    pub(crate) parent: Option<PageStyleTarget>,
    pub(crate) order: usize,
    pub(crate) position: crate::style::Position,
    pub(crate) contains_absolute: bool,
    pub(crate) contains_fixed: bool,
    pub(crate) names: Arc<[AnchorName]>,
    pub(crate) scope: AnchorScope,
    pub(crate) default: Option<AnchorReference>,
    pub(crate) page_defaults: Arc<[(usize, Option<AnchorReference>)]>,
}

/// What a layout pass measured for one absolutely positioned box that anchors.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct AnchorFrame {
    /// The fragmentainer whose containing block fragment the box is laid out against.
    pub(crate) page: usize,
    /// The size of that containing block fragment, before any position area modifies it.
    pub(crate) containing: [f32; 2],
    pub(crate) containing_flow: Flow,
    /// The target anchor box of each reference the box's styles make, if any.
    pub(crate) anchors: Vec<(AnchorReference, Option<AnchorBox>)>,
    /// The position option layout chose (§6.5), an index into the box's options.
    pub(crate) option: usize,
    /// The box overflows its inset-modified containing block with that option.
    pub(crate) overflows: bool,
}
impl AnchorFrame {
    pub(crate) fn anchor(&self, reference: &AnchorReference) -> Option<AnchorBox> {
        self.anchors
            .iter()
            .find(|(candidate, _)| candidate == reference)
            .and_then(|(_, anchor)| *anchor)
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct AnchorKey {
    pub(crate) target: PageStyleTarget,
    pub(crate) page: usize,
}
pub(crate) type AnchorFrameMap = HashMap<AnchorKey, AnchorFrame>;
type Consultation = (PageStyleTarget, Option<usize>);

/// Frames and consultations retain the physical page of each positioned occurrence.
#[derive(Clone, Default)]
pub(crate) struct AnchorFrames {
    known: AnchorFrameMap,
    consulted: RefCell<HashMap<Consultation, Option<AnchorFrame>>>,
    consulted_pages: RefCell<HashMap<PageStyleTarget, Vec<usize>>>,
}
impl AnchorFrames {
    fn find(
        known: &AnchorFrameMap,
        target: PageStyleTarget,
        page: Option<usize>,
    ) -> Option<&AnchorFrame> {
        if let Some(page) = page {
            known.get(&AnchorKey { target, page })
        } else {
            known
                .iter()
                .filter(|(key, _)| key.target == target)
                .min_by_key(|(key, _)| key.page)
                .map(|(_, frame)| frame)
        }
    }
    fn page_list(known: &AnchorFrameMap, target: PageStyleTarget) -> Vec<usize> {
        let mut pages: Vec<_> = known
            .keys()
            .filter_map(|key| (key.target == target).then_some(key.page))
            .collect();
        pages.sort_unstable();
        pages
    }
    pub(crate) fn pages(&self, target: PageStyleTarget) -> Vec<usize> {
        let pages = Self::page_list(&self.known, target);
        self.consulted_pages
            .borrow_mut()
            .insert(target, pages.clone());
        pages
    }
    pub(crate) fn frame(
        &self,
        target: PageStyleTarget,
        page: Option<usize>,
    ) -> Option<&AnchorFrame> {
        let frame = Self::find(&self.known, target, page);
        self.consulted
            .borrow_mut()
            .insert((target, page), frame.cloned());
        frame
    }
    pub(crate) fn settled(&self, measured: &AnchorFrameMap) -> bool {
        self.consulted
            .borrow()
            .iter()
            .all(|((target, page), frame)| Self::find(measured, *target, *page) == frame.as_ref())
            && self
                .consulted_pages
                .borrow()
                .iter()
                .all(|(target, pages)| Self::page_list(measured, *target) == *pages)
    }
    pub(crate) fn measured(&mut self, measured: AnchorFrameMap) {
        self.known = measured;
        self.consulted.get_mut().clear();
        self.consulted_pages.get_mut().clear();
    }
}

/// The element whose styles compute, the frames measured for it, and the try tactic of the
/// position option computing.
#[derive(Clone, Copy)]
pub(crate) struct Anchoring<'a> {
    pub(crate) page: Option<usize>,
    pub(crate) scope: Option<FontNamespace>,
    pub(crate) frames: &'a AnchorFrames,
    pub(crate) key: PageStyleTarget,
    pub(crate) tactic: Tactic,
}

/// The computed anchor-positioning properties and what their anchor functions consulted.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct AnchorStyle {
    pub(crate) pages: Arc<[(usize, Style)]>,
    pub(crate) namespaces: HashMap<Property, FontNamespace>,
    pub(crate) protected: Arc<[Property]>,
    pub(crate) option: usize,
    pub(crate) frame: Option<Arc<AnchorFrame>>,
    pub(crate) names: Arc<[AnchorName]>,
    pub(crate) scope: AnchorScope,
    pub(crate) position_anchor: PositionAnchor,
    pub(crate) area: Option<PositionArea>,
    pub(crate) fallbacks: Arc<[TryOption]>,
    /// `position-try-order`; `None` is `normal`.
    pub(crate) order: Option<AnchorDimension>,
    pub(crate) visibility: PositionVisibility,
    /// Anchor references the box's anchor functions made, which layout locates.
    pub(crate) references: Arc<[AnchorReference]>,
    /// An anchor function without a fallback failed to resolve (§6.6 `anchors-valid`).
    pub(crate) invalid_reference: bool,
    /// The box's position options, its fallback base styles first (§6.1); empty without
    /// `position-try-fallbacks`.
    pub(crate) options: Arc<[Style]>,
    /// The tactic that produced these styles from the base styles.
    pub(crate) tactic: Tactic,
}
impl AnchorStyle {
    pub(crate) fn physical_areas(&mut self, own: Flow, containing: Flow) {
        self.area = self.area.map(|area| area.physical(own, containing));
        self.order = self.order.map(|axis| {
            if axis.axis(containing, own) == HORIZONTAL {
                AnchorDimension::Width
            } else {
                AnchorDimension::Height
            }
        });
        for (_, style) in Arc::make_mut(&mut self.pages) {
            style.anchor.physical_areas(own, containing);
        }
        for style in Arc::make_mut(&mut self.options) {
            style.anchor.physical_areas(own, containing);
        }
    }

    pub(crate) fn reflowed_geometry(&mut self, from: Flow, to: Flow) {
        let sources = to.sides(from, &SIDES);
        let mapping = Tactic(std::array::from_fn(|side| {
            sources
                .iter()
                .position(|source| *source == side)
                .expect("flows permute all sides")
        }));
        let swapped = from.writing_mode.vertical() != to.writing_mode.vertical();
        self.area = self.area.map(|area| {
            area.transformed(
                mapping,
                from,
                self.frame
                    .as_ref()
                    .map_or(from, |frame| frame.containing_flow),
            )
        });
        if swapped {
            self.order = self.order.map(|axis| {
                if axis.axis(from, from) == HORIZONTAL {
                    AnchorDimension::Height
                } else {
                    AnchorDimension::Width
                }
            });
        }
        if let Some(frame) = &mut self.frame {
            let frame = Arc::make_mut(frame);
            let containing = frame.containing;
            if swapped {
                frame.containing.swap(HORIZONTAL, VERTICAL);
            }
            for (_, anchor) in &mut frame.anchors {
                if let Some(anchor) = anchor {
                    let rect = anchor.rect;
                    let distances = [
                        rect.y,
                        containing[HORIZONTAL] - rect.x - rect.width,
                        containing[VERTICAL] - rect.y - rect.height,
                        rect.x,
                    ];
                    let mapped = to.sides(from, &distances);
                    anchor.rect = Rect {
                        x: mapped[LEFT],
                        y: mapped[TOP],
                        width: frame.containing[HORIZONTAL] - mapped[LEFT] - mapped[RIGHT],
                        height: frame.containing[VERTICAL] - mapped[TOP] - mapped[BOTTOM],
                    };
                }
            }
        }
    }

    /// §2.4: the reference naming the default anchor element.
    pub(crate) fn default_reference(&self) -> Option<AnchorReference> {
        match &self.position_anchor {
            PositionAnchor::Normal => self.area.map(|_| AnchorReference::Implicit),
            PositionAnchor::None => None,
            PositionAnchor::Auto => Some(AnchorReference::Implicit),
            PositionAnchor::Named(name) => Some(AnchorReference::Named(name.clone())),
            PositionAnchor::MatchParent => Some(AnchorReference::Parent),
        }
    }
    /// Every reference layout locates for the box: its default anchor and its functions'.
    pub(crate) fn located_references(&self) -> Vec<AnchorReference> {
        let mut references: Vec<AnchorReference> = self.default_reference().into_iter().collect();
        for reference in self.references.iter() {
            if !references.contains(reference) {
                references.push(reference.clone());
            }
        }
        for option in self.options.iter() {
            for reference in option.anchor.located_references() {
                if !references.contains(&reference) {
                    references.push(reference);
                }
            }
        }
        for (_, page) in self.pages.iter() {
            for reference in page.anchor.located_references() {
                if !references.contains(&reference) {
                    references.push(reference);
                }
            }
        }
        references
    }
    /// Whether the box takes part in anchor positioning as a positioned box.
    pub(crate) fn anchors(&self) -> bool {
        self.default_reference().is_some()
            || !self.references.is_empty()
            || !self.fallbacks.is_empty()
            || self.invalid_reference
            || self.visibility.no_overflow
    }
}

/// Anchor functions resolving for one property of one element.
pub(crate) struct AnchorContext<'a> {
    frame: Option<&'a AnchorFrame>,
    zoom: f32,
    positioned: bool,
    default: Option<AnchorReference>,
    area: Option<PositionArea>,
    own_flow: Flow,
    scope: FontNamespace,
    tactic: Tactic,
    target: AnchorTarget,
    references: RefCell<Vec<AnchorReference>>,
    unresolved: Cell<bool>,
}
impl AnchorContext<'_> {
    fn reference(&self, name: Option<&Arc<str>>) -> Option<AnchorReference> {
        let reference = match name {
            Some(name) => AnchorReference::Named(AnchorName {
                name: name.clone(),
                scope: self.scope,
            }),
            None => self.default.clone()?,
        };
        let mut references = self.references.borrow_mut();
        if !references.contains(&reference) {
            references.push(reference.clone());
        }
        Some(reference)
    }
    fn anchor_box(&self, name: Option<&Arc<str>>) -> Option<Rect> {
        let reference = self.reference(name);
        if !self.positioned {
            return None;
        }
        Some(self.frame?.anchor(&reference?)?.rect)
    }
    /// The containing block insets measure from: the position area's grid region, if the
    /// box has a default anchor box (§3.1).
    fn containing(&self) -> Option<Rect> {
        let frame = self.frame?;
        let whole = Rect {
            x: 0.0,
            y: 0.0,
            width: frame.containing[HORIZONTAL],
            height: frame.containing[VERTICAL],
        };
        let area = self.area.and_then(|area| {
            let anchor = frame.anchor(self.default.as_ref()?)?;
            Some(area.rect(
                frame.containing,
                anchor.rect,
                self.own_flow,
                frame.containing_flow,
            ))
        });
        Some(area.unwrap_or(whole))
    }
    /// `anchor()` in the inset property of physical side `property` (§3.2): the inset that
    /// aligns that edge of the inset-modified containing block with the anchor edge.
    fn anchor(&self, name: Option<&Arc<str>>, side: AnchorSide) -> Option<f32> {
        let AnchorTarget::Inset(property) = self.target else {
            return None;
        };
        let anchor = self.anchor_box(name)?;
        let containing = self.containing()?;
        let written = self.tactic.source(property);
        let axis = axis_of(written);
        let containing_start = self.frame?.containing_flow.axis_start(axis);
        let own_start = self.own_flow.axis_start(axis);
        let edge = |side: usize| match side {
            TOP => anchor.y,
            BOTTOM => anchor.y + anchor.height,
            LEFT => anchor.x,
            _ => anchor.x + anchor.width,
        };
        let coordinate = match side {
            AnchorSide::Inside => edge(property),
            AnchorSide::Outside => edge(opposite(property)),
            AnchorSide::Physical(physical) if axis_of(physical) == axis => {
                edge(self.tactic.side(physical))
            }
            AnchorSide::Physical(_) => return None,
            AnchorSide::Start => edge(self.tactic.side(containing_start)),
            AnchorSide::End => edge(self.tactic.side(opposite(containing_start))),
            AnchorSide::SelfStart => edge(self.tactic.side(own_start)),
            AnchorSide::SelfEnd => edge(self.tactic.side(opposite(own_start))),
            AnchorSide::Fraction(fraction) => {
                let start = edge(self.tactic.side(containing_start));
                let end = edge(self.tactic.side(opposite(containing_start)));
                start + fraction * (end - start)
            }
        };
        Some(match property {
            TOP => coordinate - containing.y,
            BOTTOM => containing.y + containing.height - coordinate,
            LEFT => coordinate - containing.x,
            _ => containing.x + containing.width - coordinate,
        })
    }
    /// `anchor-size()` (§5.1): the anchor box's size in the named axis, or the property's.
    fn anchor_size(
        &self,
        name: Option<&Arc<str>>,
        dimension: Option<AnchorDimension>,
    ) -> Option<f32> {
        let anchor = self.anchor_box(name)?;
        let axis = match dimension {
            Some(dimension) => {
                let axis = dimension.axis(self.frame?.containing_flow, self.own_flow);
                if self.tactic.swaps_axes() && matches!(self.target, AnchorTarget::Size(_)) {
                    1 - axis
                } else {
                    axis
                }
            }
            None => self.target.axis(),
        };
        Some(if axis == HORIZONTAL {
            anchor.width
        } else {
            anchor.height
        })
    }
    /// The value of `query`, or `None` if it is unresolvable.
    pub(crate) fn resolve(&self, query: &AnchorQuery) -> Option<f32> {
        match query {
            AnchorQuery::Anchor { name, side } => self.anchor(name.as_ref(), *side),
            AnchorQuery::Size { name, dimension } => self.anchor_size(name.as_ref(), *dimension),
        }
        .map(|length| length / self.zoom)
    }
    pub(crate) fn checkpoint(&self) -> (usize, bool) {
        (self.references.borrow().len(), self.unresolved.get())
    }
    pub(crate) fn restore(&self, (references, unresolved): (usize, bool)) {
        self.references.borrow_mut().truncate(references);
        self.unresolved.set(unresolved);
    }

    /// Records an unresolvable function without a fallback.
    pub(crate) fn fail(&self) {
        self.unresolved.set(true);
    }
}

/// Anchor functions while one math function parses: the functions the property accepts and,
/// once an element computes, what they resolve against.
#[derive(Clone, Copy)]
pub(crate) struct AnchorEvaluation<'a> {
    pub(crate) functions: AnchorFunctions,
    pub(crate) context: Option<&'a AnchorContext<'a>>,
}

/// Whether a math source names an anchor function, which the element's anchors resolve.
fn mentions_anchor(value: &Component) -> bool {
    if value
        .find(Label::Production(Production::AnchorFn))
        .is_some()
        || value
            .find(Label::Production(Production::AnchorSizeFn))
            .is_some()
    {
        return true;
    }
    match value.unwrap() {
        Component::Math(source, ..) => {
            fn nested(input: &mut cssparser::Parser<'_, '_>) -> bool {
                let mut found = false;
                while let Ok(token) = input.next().cloned() {
                    match token {
                        cssparser::Token::Function(name)
                            if name.eq_ignore_ascii_case(ANCHOR_FUNCTION)
                                || name.eq_ignore_ascii_case(ANCHOR_SIZE_FUNCTION) =>
                        {
                            found = true;
                        }
                        cssparser::Token::Function(_)
                        | cssparser::Token::ParenthesisBlock
                        | cssparser::Token::SquareBracketBlock
                        | cssparser::Token::CurlyBracketBlock => {
                            found |= input
                                .parse_nested_block(|input| {
                                    Ok::<_, cssparser::ParseError<'_, ()>>(nested(input))
                                })
                                .unwrap_or(false);
                        }
                        _ => {}
                    }
                }
                found
            }
            nested(&mut cssparser::Parser::new(
                &mut cssparser::ParserInput::new(source),
            ))
        }
        _ => false,
    }
}

/// The computed value of an inset, margin or sizing property of `style` whose value is
/// `value` (§3.2.1, §5.1.1): anchor functions resolve against the element's measured anchors,
/// and a function unresolvable without a fallback makes the declaration invalid at
/// computed-value time, so the property takes its initial value.
pub(crate) fn size(
    value: &Component,
    style: &mut Style,
    relative: &Relative<'_>,
    target: AnchorTarget,
) -> Result<Size, StyleError> {
    if !mentions_anchor(value) {
        return Size::from_component(value, relative);
    }
    let frame = relative
        .anchoring
        .and_then(|anchoring| anchoring.frames.frame(anchoring.key, anchoring.page));
    let context = AnchorContext {
        frame,
        zoom: style.effective_zoom,
        positioned: style.position.is_out_of_flow(),
        default: style.anchor.default_reference(),
        area: style.anchor.area,
        own_flow: Flow::used(style),
        scope: relative
            .anchoring
            .and_then(|anchoring| anchoring.scope)
            .unwrap_or_else(|| style.element_font_namespace()),
        tactic: relative
            .anchoring
            .map_or(Tactic::IDENTITY, |anchoring| anchoring.tactic),
        target,
        references: RefCell::default(),
        unresolved: Cell::new(false),
    };
    let evaluation = AnchorEvaluation {
        functions: target.functions(),
        context: Some(&context),
    };
    let computed = match value.unwrap() {
        Component::Math(..) => {
            crate::style::box_values::anchored_length_percentage(value, relative, evaluation)?
        }
        function => crate::style::values::anchored_math(
            &format!("{CALC_FUNCTION}({function})"),
            relative,
            evaluation,
        )?
        .length_percentage(crate::style::calc::Range::ALL),
    };
    let missing = context.references.borrow().is_empty()
        || frame.is_none_or(|frame| {
            context
                .references
                .borrow()
                .iter()
                .any(|reference| frame.anchor(reference).is_none())
        });
    let recorded = context.references.into_inner();
    if !recorded.is_empty() {
        let mut references = style.anchor.references.to_vec();
        for reference in recorded {
            if !references.contains(&reference) {
                references.push(reference);
            }
        }
        style.anchor.references = Arc::from(references);
    }
    if context.unresolved.get() {
        style.anchor.invalid_reference |= missing;
        return Ok(target.initial());
    }
    Ok(Size::Value(computed))
}

/// Whether `value` is a valid value of `target`'s property, for `@supports`.
pub(crate) fn supports(value: &Component, relative: &Relative<'_>, target: AnchorTarget) -> bool {
    size(value, &mut Style::default(), relative, target).is_ok()
}

/// §6.5.2: alignment keywords follow the sides a tactic moves between their properties.
pub(crate) fn transformed_alignment(
    alignment: Alignment,
    tactic: Tactic,
    inline: bool,
    own: Flow,
    containing: Flow,
) -> Alignment {
    let target_axis = if inline {
        1 - containing.block_axis()
    } else {
        containing.block_axis()
    };
    let source_axis = if tactic.swaps_axes() {
        1 - target_axis
    } else {
        target_axis
    };
    let logical = |side, flow: Flow, start_keyword, end_keyword| {
        if tactic.side(side) == flow.axis_start(target_axis) {
            start_keyword
        } else {
            end_keyword
        }
    };
    match alignment {
        Alignment::Start => logical(
            containing.axis_start(source_axis),
            containing,
            Alignment::Start,
            Alignment::End,
        ),
        Alignment::End => logical(
            opposite(containing.axis_start(source_axis)),
            containing,
            Alignment::Start,
            Alignment::End,
        ),
        Alignment::FlexStart => logical(
            containing.axis_start(source_axis),
            containing,
            Alignment::FlexStart,
            Alignment::FlexEnd,
        ),
        Alignment::FlexEnd => logical(
            opposite(containing.axis_start(source_axis)),
            containing,
            Alignment::FlexStart,
            Alignment::FlexEnd,
        ),
        Alignment::SelfStart => logical(
            own.axis_start(source_axis),
            own,
            Alignment::SelfStart,
            Alignment::SelfEnd,
        ),
        Alignment::SelfEnd => logical(
            opposite(own.axis_start(source_axis)),
            own,
            Alignment::SelfStart,
            Alignment::SelfEnd,
        ),
        Alignment::Left | Alignment::Right => {
            let line_flow = Flow {
                rtl: false,
                ..containing
            };
            let left = if source_axis == HORIZONTAL {
                LEFT
            } else {
                line_flow.axis_start(source_axis)
            };
            let moved = tactic.side(if alignment == Alignment::Left {
                left
            } else {
                opposite(left)
            });
            if inline {
                let left = if target_axis == HORIZONTAL {
                    LEFT
                } else {
                    line_flow.axis_start(target_axis)
                };
                if moved == left {
                    Alignment::Left
                } else {
                    Alignment::Right
                }
            } else if moved == containing.axis_start(target_axis) {
                Alignment::Start
            } else {
                Alignment::End
            }
        }
        other => other,
    }
}

/// Applies a tactic's computed-value effects that a moved specified value cannot express:
/// alignment keywords and the position area (§6.5.2).
pub(crate) fn transform_computed(style: &mut Style, tactic: Tactic, containing: Flow) {
    if tactic == Tactic::IDENTITY {
        return;
    }
    let original = style.clone();
    let own = Flow::used(style);
    style.grid.justify_self =
        transformed_alignment(style.grid.justify_self, tactic, true, own, containing);
    style.flex.align_self =
        transformed_alignment(style.flex.align_self, tactic, false, own, containing);
    style.anchor.area = style
        .anchor
        .area
        .map(|area| area.transformed(tactic, own, containing));
    style.anchor.tactic = tactic;
    for property in style.anchor.protected.clone().iter() {
        crate::style::properties::copy(*property, style, &original);
    }
}

/// The computed-value consequences of position-visibility (§6.6): the box's `visibility`
/// computes to `force-hidden` when its default anchor box is invisible, it overflows, or a
/// required anchor reference fails.
pub(crate) fn finish(style: &mut Style, frame: Option<&AnchorFrame>) {
    let visibility = style.anchor.visibility;
    let hidden_anchor = visibility.anchors_visible
        && style.position.is_out_of_flow()
        && style
            .anchor
            .default_reference()
            .zip(frame)
            .and_then(|(reference, frame)| frame.anchor(&reference))
            .is_some_and(|anchor| !anchor.visible);
    let overflowing = visibility.no_overflow
        && style.position.is_out_of_flow()
        && frame.is_some_and(|frame| frame.overflows);
    let invalid = visibility.anchors_valid
        && style.position.is_out_of_flow()
        && style.anchor.invalid_reference;
    if hidden_anchor || overflowing || invalid {
        style.force_hidden = true;
    }
    if style.force_hidden {
        style.visibility = crate::style::Visibility::Hidden;
    }
}

/// The element and pseudo-element identity anchor frames are keyed by.
pub(crate) fn key(
    element: ElementId,
    pseudo: Option<crate::style::PseudoElement>,
) -> PageStyleTarget {
    PageStyleTarget { element, pseudo }
}
