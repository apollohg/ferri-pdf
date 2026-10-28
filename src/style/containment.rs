//! CSS Containment 2 and 3 computed values, and the containment a principal box is subject to.
use crate::style::{
    Display, Style, StyleError, box_values::unsupported, component::Component, grammar::Keyword,
    units::Relative,
};
use serde::Serialize;
use std::sync::Arc;

/// `contain`; `strict` and `content` compute to their keyword sets (CSS Containment 2 §2).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
pub(crate) struct Contain {
    pub size: bool,
    pub inline_size: bool,
    pub layout: bool,
    pub style: bool,
    pub paint: bool,
}
impl Contain {
    const LAYOUT: usize = 1;
    const STYLE: usize = 2;
    const PAINT: usize = 3;
    pub(super) fn from_component(value: &Component) -> Result<Self, StyleError> {
        let content = Self {
            layout: true,
            style: true,
            paint: true,
            ..Self::default()
        };
        match value.keyword() {
            Some(Keyword::None) => return Ok(Self::default()),
            Some(Keyword::Strict) => {
                return Ok(Self {
                    size: true,
                    ..content
                });
            }
            Some(Keyword::Content) => return Ok(content),
            Some(_) => return Err(unsupported(value)),
            None => {}
        }
        let sizes = value.slot(0);
        Ok(Self {
            size: sizes.and_then(|sizes| sizes.slot(0)).is_some(),
            inline_size: sizes.and_then(|sizes| sizes.slot(1)).is_some(),
            layout: value.slot(Self::LAYOUT).is_some(),
            style: value.slot(Self::STYLE).is_some(),
            paint: value.slot(Self::PAINT).is_some(),
        })
    }
}

/// `container-type` (CSS Conditional 5 §5.1).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
pub(crate) struct ContainerType {
    pub size: bool,
    pub inline_size: bool,
    pub scroll_state: bool,
}
impl ContainerType {
    pub(super) fn from_component(value: &Component) -> Self {
        if value.keyword() == Some(Keyword::Normal) {
            return Self::default();
        }
        let axes = value.slot(0).and_then(Component::keyword);
        Self {
            size: axes == Some(Keyword::Size),
            inline_size: axes == Some(Keyword::InlineSize),
            scroll_state: value.slot(1).is_some(),
        }
    }
}

/// `content-visibility` (CSS Containment 2 §4).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
pub(crate) enum ContentVisibility {
    #[default]
    Visible,
    Auto,
    Hidden,
}
impl ContentVisibility {
    pub(super) fn from_component(value: &Component) -> Result<Self, StyleError> {
        match value.keyword() {
            Some(Keyword::Visible) => Ok(Self::Visible),
            Some(Keyword::Auto) => Ok(Self::Auto),
            Some(Keyword::Hidden) => Ok(Self::Hidden),
            _ => Err(unsupported(value)),
        }
    }
}

/// One axis of `contain-intrinsic-size` (CSS Sizing 4 §5.2).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize)]
pub(crate) struct ContainIntrinsicSize {
    /// `auto` substitutes the last remembered size of an element skipping its contents.
    pub auto: bool,
    /// The explicit intrinsic inner size in points; `None` is `none`.
    pub length: Option<f32>,
}
impl ContainIntrinsicSize {
    pub(super) fn from_component(
        value: &Component,
        relative: &Relative<'_>,
    ) -> Result<Self, StyleError> {
        if value.keyword() == Some(Keyword::None) {
            return Ok(Self::default());
        }
        let [auto, size] = value.items() else {
            return Err(unsupported(value));
        };
        let length = match size.keyword() {
            Some(Keyword::None) => None,
            _ => Some(crate::style::values::length(size, relative)?),
        };
        Ok(Self {
            auto: !auto.items().is_empty(),
            length,
        })
    }
    /// A static rendering lays each element out once, so no element has a last remembered
    /// size (CSS Sizing 4 §5.2.1) and `auto` falls back to the length.
    pub(crate) fn explicit(self) -> Option<f32> {
        self.length
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub(crate) struct ContainmentStyle {
    pub contain: Contain,
    pub container_type: ContainerType,
    /// `container-name`; empty is `none`.
    pub container_names: Arc<[Arc<str>]>,
    pub content_visibility: ContentVisibility,
    /// `contain-intrinsic-width` and `contain-intrinsic-height`.
    pub intrinsic: [ContainIntrinsicSize; 2],
}

/// `none | <custom-ident>+`.
pub(super) fn container_names(value: &Component) -> Result<Arc<[Arc<str>]>, StyleError> {
    if value.keyword() == Some(Keyword::None) {
        return Ok(Arc::from([]));
    }
    value
        .items()
        .iter()
        .map(|name| match name.unwrap() {
            Component::Ident(name) => Ok(Arc::from(name.as_ref())),
            other => Err(unsupported(other)),
        })
        .collect()
}

/// The containment a box is subject to once `contain`, `container-type` and
/// `content-visibility` combine and the boxes each type has no effect on are excluded.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct Containment {
    /// Size containment of the width and the height.
    pub size: [bool; 2],
    pub layout: bool,
    pub style: bool,
    pub paint: bool,
    /// Size container queries need an independent formatting context (CSS Conditional 5 §5.1).
    pub query_container: bool,
}

impl Style {
    /// The box's used containment. Inline-level boxes the layout sees are atomic: inline
    /// replaced elements, since non-replaced inline boxes are not boxes of their own there.
    pub(crate) fn containment(&self) -> Containment {
        let ContainmentStyle {
            contain,
            container_type,
            content_visibility,
            ..
        } = &self.containment;
        // CSS Containment 2 §4: `auto` turns on layout, style and paint containment, and
        // skipping contents (`hidden`) adds size containment.
        let visibility = *content_visibility != ContentVisibility::Visible;
        let skips = self.skips_contents();
        let size = contain.size || container_type.size || skips;
        let inline_size = size || contain.inline_size || container_type.inline_size;
        let display = &self.display;
        let internal = display.is_table_internal();
        // §3.1: size containment has no effect on tables, internal table boxes and non-atomic
        // inline boxes; §3.2 and §3.4 exempt table cells for layout and paint containment.
        let sizes = !display.is_table() && !internal && *display != Display::None;
        let boxes = (!internal || *display == Display::TableCell) && *display != Display::None;
        Containment {
            size: [inline_size && sizes, size && sizes],
            layout: (contain.layout || visibility) && boxes,
            style: contain.style || visibility || container_type.size || container_type.inline_size,
            paint: (contain.paint || visibility) && boxes,
            query_container: (container_type.size || container_type.inline_size) && sizes,
        }
    }
    /// `content-visibility: hidden` skips the element's contents. Paged static media make
    /// every element relevant to the user (CSS Containment 2 §4), so `auto` never skips.
    pub(crate) fn skips_contents(&self) -> bool {
        self.containment.content_visibility == ContentVisibility::Hidden
    }
    /// Layout or paint containment, which make the box an absolute and fixed positioning
    /// containing block and a stacking context.
    pub(super) fn contains_positioned(&self) -> bool {
        let containment = self.containment();
        containment.layout || containment.paint
    }
}
