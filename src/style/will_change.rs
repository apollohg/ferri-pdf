//! CSS Will Change 1 §3.1: naming a property in `will-change` gives the element the stacking
//! context and containing blocks a non-initial value of that property would give it.
use crate::style::{
    StyleError, box_values::unsupported, component::Component, declaration, grammar::Keyword,
    grammar::Property,
};

/// Properties some non-initial value of which creates a stacking context: CSS Transforms 1
/// §3 and 2 §5-6, CSS Color 4 §15, Filter Effects 1 §5, Compositing 1 §3.2, CSS Masking 1 §6
/// and §8, CSS Contain 2 §3.2-3.3 (through `contain`, `container-type` and
/// `content-visibility`), CSS Position 3 §3 (`fixed`) and CSS View Transitions.
const STACKING: &[Property] = &[
    Property::Opacity,
    Property::Transform,
    Property::Translate,
    Property::Rotate,
    Property::Scale,
    Property::Perspective,
    Property::TransformStyle,
    Property::Filter,
    Property::BackdropFilter,
    Property::MixBlendMode,
    Property::Isolation,
    Property::MaskImage,
    Property::MaskBorderSource,
    Property::ClipPath,
    Property::Contain,
    Property::ContainerType,
    Property::ContentVisibility,
    Property::Position,
    Property::ViewTransitionName,
];
/// Properties some non-initial value of which makes a fixed-positioning containing block, and
/// with it an absolute-positioning one.
const FIXED_CONTAINING: &[Property] = &[
    Property::Transform,
    Property::Translate,
    Property::Rotate,
    Property::Scale,
    Property::Perspective,
    Property::TransformStyle,
    Property::Filter,
    Property::BackdropFilter,
    Property::Contain,
    Property::ContainerType,
    Property::ContentVisibility,
];

/// The boxes the computed `will-change` makes the element generate.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, serde::Serialize)]
pub(crate) struct WillChange {
    pub(crate) stacking_context: bool,
    /// `z-index`, which creates a stacking context only where it applies.
    pub(crate) z_index: bool,
    pub(crate) absolute_containing_block: bool,
    pub(crate) fixed_containing_block: bool,
}
impl WillChange {
    /// `auto`, or a list of `<animateable-feature>`s; custom properties and unknown names have
    /// no effect.
    pub(super) fn from_component(value: &Component) -> Result<Self, StyleError> {
        let mut hints = Self::default();
        if value.keyword() == Some(Keyword::Auto) {
            return Ok(hints);
        }
        for feature in value.items() {
            match feature.unwrap() {
                Component::Keyword(Keyword::ScrollPosition | Keyword::Contents) => {}
                Component::Ident(name) => {
                    let Some(property) = Property::from_name(&name.to_ascii_lowercase()) else {
                        continue;
                    };
                    for longhand in declaration::longhands(property) {
                        hints.stacking_context |= STACKING.contains(&longhand);
                        hints.z_index |= longhand == Property::ZIndex;
                        hints.fixed_containing_block |= FIXED_CONTAINING.contains(&longhand);
                        hints.absolute_containing_block |=
                            FIXED_CONTAINING.contains(&longhand) || longhand == Property::Position;
                    }
                }
                _ => return Err(unsupported(feature)),
            }
        }
        Ok(hints)
    }
}
