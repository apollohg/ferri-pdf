use crate::{
    Size, Style, StyleError, box_values::unsupported, component::Component, grammar::Keyword,
    units::Relative,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub enum Position {
    #[default]
    Static,
    Relative,
    Absolute,
    Fixed,
}
impl Position {
    pub(crate) fn from_component(value: &Component) -> Result<Self, StyleError> {
        match value.keyword() {
            Some(Keyword::Static) => Ok(Self::Static),
            Some(Keyword::Relative) => Ok(Self::Relative),
            Some(Keyword::Absolute) => Ok(Self::Absolute),
            Some(Keyword::Fixed) => Ok(Self::Fixed),
            _ => Err(unsupported(value)),
        }
    }
    /// Absolutely positioned boxes leave normal flow.
    pub fn is_out_of_flow(self) -> bool {
        matches!(self, Self::Absolute | Self::Fixed)
    }
}
/// `z-index`: `None` is `auto`.
pub(crate) fn z_index(
    value: &Component,
    relative: &Relative<'_>,
) -> Result<Option<i32>, StyleError> {
    match value.keyword() {
        Some(Keyword::Auto) => Ok(None),
        _ => crate::values::integer(value, relative).map(Some),
    }
}
impl Style {
    pub fn relative_offset(
        &self,
        width: f32,
        height: Option<f32>,
        containing_rtl: bool,
    ) -> [f32; 2] {
        if self.position != Position::Relative {
            return [0.0; 2];
        }
        let [top, right, bottom, left]: &[Size; 4] = &self.inset;
        let left = left.resolve(width);
        let right = right.resolve(width);
        let x = if containing_rtl {
            right.map(|v| -v).or(left)
        } else {
            left.or_else(|| right.map(|v| -v))
        };
        let y = top
            .resolve_definite(height)
            .or_else(|| bottom.resolve_definite(height).map(|v| -v));
        [x.unwrap_or(0.0), y.unwrap_or(0.0)]
    }
}
