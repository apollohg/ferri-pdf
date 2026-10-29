//! `appearance` and `field-sizing` (CSS UI 4 §7.2, CSS UI 4 §6.1), and the author styling
//! that devolves a widget's native appearance (CSS UI 4 §7.2.1).
use crate::style::{
    StyleError, box_values::unsupported, component::Component, grammar::Keyword, grammar::Property,
};
use serde::Serialize;

/// The computed `appearance`. The `<compat-auto>` keywords compute as specified but render
/// exactly as `auto`, so they share its variant.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
pub(crate) enum Appearance {
    #[default]
    None,
    Auto,
    Base,
    BaseSelect,
    /// A search field looks like a text field.
    Textfield,
    /// A drop-down box looks as it does devolved.
    MenulistButton,
}
impl Appearance {
    pub(super) fn from_component(value: &Component) -> Result<Self, StyleError> {
        Ok(match value.keyword() {
            Some(Keyword::None) => Self::None,
            Some(Keyword::Base) => Self::Base,
            Some(Keyword::BaseSelect) => Self::BaseSelect,
            Some(Keyword::Textfield) => Self::Textfield,
            Some(Keyword::MenulistButton) => Self::MenulistButton,
            Some(
                Keyword::Auto
                | Keyword::Searchfield
                | Keyword::Textarea
                | Keyword::Checkbox
                | Keyword::Radio
                | Keyword::Menulist
                | Keyword::Listbox
                | Keyword::Meter
                | Keyword::ProgressBar
                | Keyword::Button,
            ) => Self::Auto,
            _ => return Err(unsupported(value)),
        })
    }
}

/// The computed `field-sizing`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
pub(crate) enum FieldSizing {
    #[default]
    Fixed,
    Content,
}
impl FieldSizing {
    pub(super) fn from_component(value: &Component) -> Result<Self, StyleError> {
        match value.keyword() {
            Some(Keyword::Fixed) => Ok(Self::Fixed),
            Some(Keyword::Content) => Ok(Self::Content),
            _ => Err(unsupported(value)),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
pub(crate) struct WidgetStyle {
    pub appearance: Appearance,
    pub field_sizing: FieldSizing,
    /// The author origin cascades a property that disables native appearance.
    pub author_decorated: bool,
}

/// CSS UI 4 §7.2.1: the properties that disable native appearance for widgets.
pub(super) const NATIVE_APPEARANCE_DISABLERS: [Property; 45] = [
    Property::BackgroundColor,
    Property::BorderTopColor,
    Property::BorderTopStyle,
    Property::BorderTopWidth,
    Property::BorderRightColor,
    Property::BorderRightStyle,
    Property::BorderRightWidth,
    Property::BorderBottomColor,
    Property::BorderBottomStyle,
    Property::BorderBottomWidth,
    Property::BorderLeftColor,
    Property::BorderLeftStyle,
    Property::BorderLeftWidth,
    Property::BorderBlockStartColor,
    Property::BorderBlockEndColor,
    Property::BorderInlineStartColor,
    Property::BorderInlineEndColor,
    Property::BorderBlockStartStyle,
    Property::BorderBlockEndStyle,
    Property::BorderInlineStartStyle,
    Property::BorderInlineEndStyle,
    Property::BorderBlockStartWidth,
    Property::BorderBlockEndWidth,
    Property::BorderInlineStartWidth,
    Property::BorderInlineEndWidth,
    Property::BackgroundImage,
    Property::BackgroundAttachment,
    Property::BackgroundPositionX,
    Property::BackgroundPositionY,
    Property::BackgroundClip,
    Property::BackgroundOrigin,
    Property::BackgroundSize,
    Property::BorderImageSource,
    Property::BorderImageSlice,
    Property::BorderImageWidth,
    Property::BorderImageOutset,
    Property::BorderImageRepeat,
    Property::BorderTopLeftRadius,
    Property::BorderTopRightRadius,
    Property::BorderBottomRightRadius,
    Property::BorderBottomLeftRadius,
    Property::BorderStartStartRadius,
    Property::BorderStartEndRadius,
    Property::BorderEndStartRadius,
    Property::BorderEndEndRadius,
];
