//! Properties whose specifications give them no effect in static paged media. Their values
//! cascade, compute and inherit like every other property's, but layout and paint never read
//! them.
use crate::paint::{Easing, LengthPercentage};
use crate::style::{
    Style, StyleError,
    box_values::{length_percentage, unsupported},
    calc::Kind,
    component::{Component, target},
    declaration,
    grammar::{Base, BlockKind, Keyword, Label, NODES, Node, Production, Property},
    properties::Input,
    units::Relative,
    values,
};
use std::{collections::BTreeMap, sync::Arc};

const NONE: &str = "none";

/// Whether `property` has no effect in static media.
pub(super) fn defined(property: Property) -> bool {
    use Property as P;
    matches!(
        property,
        // CSS Animations 1 §2: implementations may ignore animations when the rendering
        // medium is not interactive, such as print; the computed style is the un-animated
        // one. Timelines (Scroll-driven Animations 1), triggers (Animation Triggers 1) and
        // sizing-keyword interpolation (CSS Values 5 §10.4) only drive those animations.
        P::AnimationComposition
            | P::AnimationDelay
            | P::AnimationDirection
            | P::AnimationDuration
            | P::AnimationFillMode
            | P::AnimationIterationCount
            | P::AnimationName
            | P::AnimationPlayState
            | P::AnimationRangeEnd
            | P::AnimationRangeStart
            | P::AnimationTimeline
            | P::AnimationTimingFunction
            | P::AnimationTrigger
            | P::InterpolateSize
            | P::ScrollTimelineAxis
            | P::ScrollTimelineName
            | P::TimelineScope
            | P::TimelineTriggerExitRangeEnd
            | P::TimelineTriggerExitRangeStart
            | P::TimelineTriggerName
            | P::TimelineTriggerRangeEnd
            | P::TimelineTriggerRangeStart
            | P::TimelineTriggerSource
            | P::TriggerScope
            | P::ViewTimelineAxis
            | P::ViewTimelineInset
            | P::ViewTimelineName
            // CSS Transitions 1 §3: transitions start only at a style change event after the
            // first, and a static rendering computes style once.
            | P::TransitionBehavior
            | P::TransitionDelay
            | P::TransitionDuration
            | P::TransitionProperty
            | P::TransitionTimingFunction
            // CSS View Transitions 2: classes only select view-transition pseudo-elements,
            // which exist during a view transition.
            | P::ViewTransitionClass
            // CSS Overflow 3 §3.1.3: static media cannot scroll. Snapping, anchoring and
            // overscroll adjust scroll positions; proximity snapping only permits a snap
            // (CSS Scroll Snap 1 §4.1.2), and margins and padding only shape snap areas and
            // snapports.
            | P::OverflowAnchor
            | P::OverscrollBehaviorX
            | P::OverscrollBehaviorY
            | P::ScrollBehavior
            | P::ScrollMarginBottom
            | P::ScrollMarginLeft
            | P::ScrollMarginRight
            | P::ScrollMarginTop
            | P::ScrollPaddingBottom
            | P::ScrollPaddingLeft
            | P::ScrollPaddingRight
            | P::ScrollPaddingTop
            | P::ScrollSnapAlign
            | P::ScrollSnapStop
            | P::ScrollSnapType
            // CSS Overflow 3 §3.1.3: static media need not display scrollbars, and this
            // engine displays none, so their colour and thickness never render.
            | P::ScrollbarColor
            | P::ScrollbarWidth
            // CSS UI 4: pointer, caret, selection, resizing, inertness and interest only
            // concern user interaction.
            | P::CaretAnimation
            | P::CaretColor
            | P::CaretShape
            | P::Cursor
            | P::ImeMode
            | P::Interactivity
            | P::InterestDelayEnd
            | P::InterestDelayStart
            | P::PointerEvents
            | P::Resize
            | P::TouchAction
            | P::UserSelect
            // CSS Size Adjust 1: adjustment only applies when displaying on a small device.
            | P::TextSizeAdjust
            // CSS Color Adjust 1 §3: forced colors mode never applies to this output, and
            // `economy` leaves adjustments to the user agent, which makes none.
            | P::ForcedColorAdjust
            | P::PrintColorAdjust
            // CSS Color HDR 1: every colour this engine outputs is within HDR Reference White.
            | P::DynamicRangeLimit
            // CSS Speech 1 and CSS Display 4 §4: aural rendering and reading order, which a
            // visual rendering without a tagged structure does not have.
            | P::SpeakAs
            | P::ReadingFlow
            | P::ReadingOrder
    )
}

/// CSS Transitions 1 §2.1 and §2.5: the transitioned property's `<custom-ident>` excludes
/// `none`, which only a single-item list may contain.
pub(super) fn valid(property: Property, value: &Component) -> bool {
    if !matches!(
        property,
        Property::Transition | Property::TransitionProperty
    ) {
        return true;
    }
    let mut names = Vec::new();
    value.references(
        &[Label::Production(Production::SingleTransitionProperty)],
        &mut names,
    );
    let named_none = names.iter().any(|(_, name)| {
        matches!(name.unwrap(), Component::Ident(name) if name.eq_ignore_ascii_case(NONE))
    });
    let items = value.items();
    let listed_none = items.len() > 1
        && items.iter().any(|item| {
            item.keyword() == Some(Keyword::None)
                || item.slot(0).and_then(Component::keyword) == Some(Keyword::None)
        });
    !named_none && !listed_none
}

/// A computed value: the specified match tree with lengths, math functions and easing
/// functions resolved against the element (CSS Values 4 §4.4).
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Computed {
    /// Keywords, identifiers, strings, absolute URLs and colours compute as specified.
    Specified(Component),
    LengthPercentage(LengthPercentage),
    /// A number, time, angle or resolution in its canonical unit.
    Dimension(Kind, f32),
    Integer(i32),
    Easing(Easing),
    Function(Keyword, Option<Box<Computed>>),
    Block(BlockKind, Box<Computed>),
    Group(Vec<Computed>),
    Slots(Vec<Option<Computed>>),
}

/// The non-initial computed values of the properties in this module.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct InertValues(Arc<BTreeMap<Property, Computed>>);

impl InertValues {
    #[cfg(test)]
    pub(crate) fn get(&self, property: Property) -> Option<&Computed> {
        self.0.get(&property)
    }
    fn set(&mut self, property: Property, value: Option<Computed>) {
        if self.0.get(&property) == value.as_ref() {
            return;
        }
        let values = Arc::make_mut(&mut self.0);
        match value {
            Some(value) => values.insert(property, value),
            None => values.remove(&property),
        };
    }
}

pub(super) fn apply(
    property: Property,
    style: &mut Style,
    input: Input<'_>,
    relative: &Relative<'_>,
) -> Result<(), StyleError> {
    let value = match input {
        Input::Copy(source) => {
            copy(property, style, source);
            return Ok(());
        }
        Input::Value(value) => value,
    };
    let computed = compute(property, value, relative)?;
    let initial = declaration::initial(property)
        .map(|initial| compute(property, initial, relative))
        .transpose()?;
    style.inert.set(
        property,
        (initial.as_ref() != Some(&computed)).then_some(computed),
    );
    Ok(())
}

pub(super) fn copy(property: Property, target: &mut Style, source: &Style) {
    copy_from(property, target, property, source);
}

/// Gives `target`'s `property` the value `source` has for `from`.
pub(super) fn copy_from(property: Property, target: &mut Style, from: Property, source: &Style) {
    target
        .inert
        .set(property, source.inert.0.get(&from).cloned());
}

pub(super) fn compute(
    property: Property,
    value: &Component,
    relative: &Relative<'_>,
) -> Result<Computed, StyleError> {
    // CSS Scroll Snap 1 §4.1.2: a mandatory snap container must rest on a snap position,
    // which can move its initial scroll position and so what static media show.
    if property == Property::ScrollSnapType {
        let mut keywords = Vec::new();
        value.keywords(&mut keywords);
        if keywords.contains(&Keyword::Mandatory) {
            return Err(unsupported(value));
        }
    }
    if matches!(
        NODES[usize::from(target(Label::Property(property)))],
        Node::Base(Base::Integer, _)
    ) {
        return Ok(Computed::Integer(values::integer(value, relative)?));
    }
    resolve(value, relative)
}

fn resolve(value: &Component, relative: &Relative<'_>) -> Result<Computed, StyleError> {
    let resolve_all = |items: &[Component]| {
        items
            .iter()
            .map(|item| resolve(item, relative))
            .collect::<Result<Vec<_>, _>>()
    };
    Ok(match value {
        Component::Reference(Label::Production(Production::EasingFunction), _) => Computed::Easing(
            crate::style::easing::function(value, relative)?.ok_or_else(|| unsupported(value))?,
        ),
        Component::Reference(_, inner) => resolve(inner, relative)?,
        Component::Length(_)
        | Component::Percentage(_)
        | Component::Math(_, Kind::Length | Kind::Percentage, _) => {
            Computed::LengthPercentage(length_percentage(value, relative)?)
        }
        Component::Math(source, kind, range) => Computed::Dimension(
            *kind,
            values::math(source, relative, *kind, None)?.resolve(0.0, *range),
        ),
        Component::Number(number) => Computed::Dimension(Kind::Number, *number),
        Component::Time(seconds) => Computed::Dimension(Kind::Time, *seconds),
        Component::Angle(degrees) => Computed::Dimension(Kind::Angle, *degrees),
        Component::Resolution(dppx) => Computed::Dimension(Kind::Resolution, *dppx),
        Component::Integer(integer) => Computed::Integer(*integer),
        Component::Function(name, body) => Computed::Function(
            *name,
            body.as_deref()
                .map(|body| resolve(body, relative).map(Box::new))
                .transpose()?,
        ),
        Component::Block(kind, body) => Computed::Block(*kind, Box::new(resolve(body, relative)?)),
        Component::Group(items) => Computed::Group(resolve_all(items)?),
        Component::Slots(slots) => Computed::Slots(
            slots
                .iter()
                .map(|slot| {
                    slot.as_ref()
                        .map(|slot| resolve(slot, relative))
                        .transpose()
                })
                .collect::<Result<_, _>>()?,
        ),
        leaf => Computed::Specified(leaf.clone()),
    })
}
