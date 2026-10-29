use crate::style::{
    Display, Style, StyleError,
    box_values::unsupported,
    component::Component,
    declaration::Specified,
    grammar::{Keyword, Property},
    units::Relative,
};
use std::{collections::HashMap, sync::Arc};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub(crate) enum Overflow {
    #[default]
    Visible,
    Hidden,
    Clip,
    Auto,
    Scroll,
}
impl Overflow {
    pub(crate) fn from_component(value: &Component) -> Result<Self, StyleError> {
        match value.keyword() {
            Some(Keyword::Visible) => Ok(Self::Visible),
            Some(Keyword::Hidden) => Ok(Self::Hidden),
            Some(Keyword::Clip) => Ok(Self::Clip),
            Some(Keyword::Auto) => Ok(Self::Auto),
            Some(Keyword::Scroll) => Ok(Self::Scroll),
            _ => Err(unsupported(value)),
        }
    }
    fn scroll_container(self) -> bool {
        matches!(self, Self::Hidden | Self::Auto | Self::Scroll)
    }
    fn paired(self, other_scroll_container: bool) -> Self {
        match (self, other_scroll_container) {
            (Self::Visible, true) => Self::Auto,
            (Self::Clip, true) => Self::Hidden,
            _ => self,
        }
    }
    fn clips(self) -> bool {
        matches!(self, Self::Hidden | Self::Clip)
    }
}
const AXIS_PROPERTIES: [Property; 2] = [Property::OverflowX, Property::OverflowY];
type ArgumentList = *const [crate::style::declaration::ToggleArgument];

#[derive(Clone)]
pub(super) enum AxisValue {
    Specified(Specified),
    Computed(Overflow),
}

pub(super) fn compute(
    style: &mut Style,
    parent: &Style,
    winners: &mut crate::style::cascade::Winners,
    relative: &Relative<'_>,
) -> Result<(), StyleError> {
    let specified = AXIS_PROPERTIES.map(|property| winners.remove(&property));
    if specified
        .iter()
        .any(|value| matches!(value, Some(Specified::Toggle(_))))
    {
        let source = |axis: usize| {
            Ok(specified[axis].clone().map_or(
                AxisValue::Computed(style.overflow[axis]),
                AxisValue::Specified,
            ))
        };
        let mut axes = Axes::new(style, parent, relative, &source);
        style.overflow = [axes.raw(0)?, axes.raw(1)?];
    } else {
        for (property, value) in AXIS_PROPERTIES.into_iter().zip(specified) {
            if let Some(value) = value {
                crate::style::properties::apply(property, &value, style, parent, relative)?;
            }
        }
    }
    style.pair_overflow();
    Ok(())
}

pub(super) fn compute_axis(
    axis: usize,
    style: &Style,
    parent: &Style,
    relative: &Relative<'_>,
    source: impl Fn(usize) -> Result<AxisValue, StyleError>,
) -> Result<Overflow, StyleError> {
    Axes::new(style, parent, relative, &source).computed(axis)
}

struct Axes<'a, 'r, 's> {
    source: &'s dyn Fn(usize) -> Result<AxisValue, StyleError>,
    specified: [Option<AxisValue>; 2],
    style: &'a Style,
    parent: &'a Style,
    relative: &'a Relative<'r>,
    raw: [Option<Overflow>; 2],
    active: Vec<usize>,
    choices: HashMap<(usize, ArgumentList), usize>,
    uniform: HashMap<(usize, ArgumentList), Option<bool>>,
}

impl<'a, 'r, 's> Axes<'a, 'r, 's> {
    fn new(
        style: &'a Style,
        parent: &'a Style,
        relative: &'a Relative<'r>,
        source: &'s dyn Fn(usize) -> Result<AxisValue, StyleError>,
    ) -> Self {
        Self {
            source,
            specified: [None, None],
            style,
            parent,
            relative,
            raw: [None; 2],
            active: Vec::new(),
            choices: HashMap::new(),
            uniform: HashMap::new(),
        }
    }
    fn source(&mut self, axis: usize) -> Result<AxisValue, StyleError> {
        if let Some(value) = &self.specified[axis] {
            return Ok(value.clone());
        }
        let value = (self.source)(axis)?;
        self.specified[axis] = Some(value.clone());
        Ok(value)
    }
    fn scrollability(&mut self, axis: usize) -> Result<bool, StyleError> {
        let constant = match self.source(axis)? {
            AxisValue::Specified(value) => self.uniform_scroll(axis, &value)?,
            AxisValue::Computed(value) => Some(value.scroll_container()),
        };
        match constant {
            Some(value) => Ok(value),
            None => Ok(self.raw(axis)?.scroll_container()),
        }
    }
    fn computed(&mut self, axis: usize) -> Result<Overflow, StyleError> {
        let value = self.raw(axis)?;
        if value.paired(true) == value {
            return Ok(value);
        }
        Ok(value.paired(self.scrollability(1 - axis)?))
    }
    fn raw(&mut self, axis: usize) -> Result<Overflow, StyleError> {
        if let Some(value) = self.raw[axis] {
            return Ok(value);
        }
        if let Some(start) = self.active.iter().position(|active| *active == axis) {
            let mut sources = Vec::new();
            let mut dependencies = Vec::new();
            for &index in self.active[start..].iter().chain([&axis]) {
                dependencies.push(AXIS_PROPERTIES[index].name().to_owned());
                if let Some(AxisValue::Specified(Specified::Toggle(values))) =
                    self.specified[index].as_ref()
                    && let Some(source) = values
                        .iter()
                        .find_map(|argument| argument.location.as_deref())
                    && !sources.contains(source)
                {
                    sources.push(source.clone());
                }
            }
            return Err(StyleError::DependencyCycle {
                sources,
                dependencies,
            });
        }
        self.active.push(axis);
        let computed = (|| match self.source(axis)? {
            AxisValue::Specified(value) => self.value(axis, &value),
            AxisValue::Computed(value) => Ok(value),
        })();
        self.active.pop();
        let computed = computed?;
        self.raw[axis] = Some(computed);
        Ok(computed)
    }

    fn leaf(&self, axis: usize, specified: &Specified) -> Result<Overflow, StyleError> {
        let mut style = self.style.clone();
        crate::style::properties::apply(
            AXIS_PROPERTIES[axis],
            specified,
            &mut style,
            self.parent,
            self.relative,
        )?;
        Ok(style.overflow[axis])
    }

    // A constant scrollability result needs no dependency on the other axis's selection.
    fn uniform_scroll(
        &mut self,
        axis: usize,
        specified: &Specified,
    ) -> Result<Option<bool>, StyleError> {
        let Specified::Toggle(values) = specified else {
            return self
                .leaf(axis, specified)
                .map(|value| Some(value.scroll_container()));
        };
        let _work = self.relative.work.enter(self.relative.grammar_only);
        self.relative.work.charge(1)?;
        let key = (axis, Arc::as_ptr(values));
        if let Some(value) = self.uniform.get(&key) {
            return Ok(*value);
        }
        let mut uniform = None;
        let mut mixed = false;
        for argument in values.iter() {
            self.relative.work.charge(1)?;
            let value = self.uniform_scroll(axis, &argument.value)?;
            mixed |= value.is_none() || uniform.is_some_and(|uniform| Some(uniform) != value);
            uniform = value;
        }
        let value = if mixed { None } else { uniform };
        self.uniform.insert(key, value);
        Ok(value)
    }

    fn matches_parent(&mut self, axis: usize, value: Overflow) -> Result<bool, StyleError> {
        let inherited = self.parent.overflow[axis];
        let paired = value.paired(true);
        if paired == value || (value != inherited && paired != inherited) {
            return Ok(value == inherited);
        }
        let scrollable = self.scrollability(1 - axis)?;
        Ok(value.paired(scrollable) == inherited)
    }

    fn value(&mut self, axis: usize, specified: &Specified) -> Result<Overflow, StyleError> {
        let Specified::Toggle(values) = specified else {
            return self.leaf(axis, specified);
        };
        let relative = self.relative;
        let _work = relative.work.enter(relative.grammar_only);
        relative.work.charge(1)?;
        let key = (axis, Arc::as_ptr(values));
        let chosen = if let Some(&chosen) = self.choices.get(&key) {
            chosen
        } else {
            let chosen =
                crate::style::properties::toggle_index(values, &true, relative, |argument| {
                    let value = self.value(axis, &argument.value)?;
                    self.matches_parent(axis, value)
                })?;
            self.choices.insert(key, chosen);
            chosen
        };
        self.value(axis, &values[chosen].value)
    }
}

/// A `text-overflow` keyword; `<string>` values are unsupported.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub(crate) enum TextOverflow {
    #[default]
    Clip,
    Ellipsis,
}
impl TextOverflow {
    /// Line-left and line-right edge behaviour; one value applies to both (CSS Overflow 4 §4.1).
    pub(crate) fn from_component(value: &Component) -> Result<[Self; 2], StyleError> {
        let side = |item: &Component| match item.keyword() {
            Some(Keyword::Clip) => Ok(Self::Clip),
            Some(Keyword::Ellipsis) => Ok(Self::Ellipsis),
            _ => Err(unsupported(value)),
        };
        match value.items() {
            [both] => Ok([side(both)?; 2]),
            [left, right] => Ok([side(left)?, side(right)?]),
            _ => Err(unsupported(value)),
        }
    }
}
/// `block-ellipsis` (CSS Overflow 4 §4.2).
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub(crate) enum BlockEllipsis {
    #[default]
    None,
    Auto,
    String(Arc<str>),
}
impl BlockEllipsis {
    pub(super) fn from_component(value: &Component) -> Result<Self, StyleError> {
        match value.unwrap() {
            Component::Keyword(Keyword::None) => Ok(Self::None),
            Component::Keyword(Keyword::Auto) => Ok(Self::Auto),
            Component::String(text) => Ok(Self::String(Arc::from(text.as_ref()))),
            other => Err(unsupported(other)),
        }
    }
}

/// `continue` (CSS Overflow 4 §5.3 and §5.1.1).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Continue {
    #[default]
    Auto,
    Discard,
    WebkitDiscard,
}
impl Continue {
    pub(super) fn from_component(value: &Component) -> Result<Self, StyleError> {
        match value.keyword() {
            Some(Keyword::Auto) => Ok(Self::Auto),
            Some(Keyword::Discard) => Ok(Self::Discard),
            Some(Keyword::DashWebkitDiscard) => Ok(Self::WebkitDiscard),
            _ => Err(unsupported(value)),
        }
    }
}

/// The longhands of `line-clamp` (CSS Overflow 4 §5).
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub(crate) struct LineClamp {
    /// `None` is `none`.
    pub max_lines: Option<usize>,
    pub block_ellipsis: BlockEllipsis,
    pub continuation: Continue,
}

impl Style {
    /// CSS Overflow 3: visible and clip become auto and hidden beside a scroll container axis.
    pub(crate) fn pair_overflow(&mut self) {
        let [x, y] = self.overflow;
        self.overflow = [
            x.paired(y.scroll_container()),
            y.paired(x.scroll_container()),
        ];
    }
    /// Horizontal and vertical clipping at the padding edge. Auto and scroll print their
    /// overflow, except that a scrollable vertical axis beside a clipped horizontal one is
    /// also clipped. Paint containment clips both (CSS Containment 2 §3.4).
    pub(crate) fn clipped_axes(&self) -> [bool; 2] {
        let [x, y] = self.overflow;
        let paint = self.containment().paint;
        [
            x.clips() || paint,
            y.clips() || (x.clips() && y.scroll_container()) || paint,
        ]
    }
    /// Overflow other than `visible` and `clip`, after axis pairing.
    pub(crate) fn is_scroll_container(&self) -> bool {
        self.overflow[0].scroll_container()
    }
    /// Whether lines overflowing the inline end are ellipsed (CSS Overflow 3 §3.1): only in
    /// block containers whose inline axis does not overflow visibly. Overflowing lines are
    /// start-aligned, so only the inline-end edge's value applies.
    pub(crate) fn ellipsizes(&self) -> bool {
        self.text_overflow[usize::from(!self.rtl)] == TextOverflow::Ellipsis
            && self.overflow[0] != Overflow::Visible
    }
    /// Formatting roots include table cells/captions (CSS Display 3 §2.4), scroll containers,
    /// positioned boxes, atomic inlines, multicol containers, layout and paint containment
    /// boxes (CSS Containment 2 §3.2, §3.4) and size query containers (CSS Conditional 5
    /// §5.1); `overflow: clip` does not.
    pub(crate) fn establishes_formatting_context(&self) -> bool {
        let containment = self.containment();
        containment.layout
            || containment.paint
            || containment.query_container
            || self.independent_box()
            || matches!(self.display, Display::TableCell | Display::TableCaption)
            || matches!(self.display, Display::FlowRoot | Display::FlowRootListItem)
            || self.is_scroll_container()
            || self.position.is_out_of_flow()
            || self.floating().is_some()
            || self.display.is_atomic_inline()
            || self.multicol.is_multicol()
            // CSS Box Alignment 3 §5.4: content alignment makes a block container independent.
            || self.flex.align_content != crate::style::Alignment::Normal
    }
}
