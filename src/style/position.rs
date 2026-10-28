use crate::style::{
    Size, Style, StyleError, box_values::unsupported, component::Component, grammar::Keyword,
    units::Relative,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub(crate) enum Position {
    #[default]
    Static,
    Relative,
    Sticky,
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
    pub(crate) fn is_out_of_flow(self) -> bool {
        matches!(self, Self::Absolute | Self::Fixed)
    }
}
/// CSS Masking 1 §6.1 `rect()` offsets of the top, right, bottom and left clip edges from the
/// border box's top-left corner; `None` is `auto`, the matching border edge.
pub(crate) type ClipRect = [Option<f32>; 4];
const TOP: usize = 0;
const RIGHT: usize = 1;
const BOTTOM: usize = 2;
const LEFT: usize = 3;

/// `clip`: `None` is `auto`.
pub(crate) fn clip_rect(
    value: &Component,
    relative: &Relative<'_>,
) -> Result<Option<ClipRect>, StyleError> {
    if value.keyword() == Some(Keyword::Auto) {
        return Ok(None);
    }
    let Component::Function(Keyword::Rect, _) = value.unwrap() else {
        return Err(unsupported(value));
    };
    let edges = crate::style::transform::arguments(value)
        .into_iter()
        .map(|edge| match edge.keyword() {
            Some(Keyword::Auto) => Ok(None),
            _ => crate::style::values::length(edge, relative).map(Some),
        })
        .collect::<Result<Vec<_>, _>>()?;
    edges.try_into().map(Some).map_err(|_| unsupported(value))
}
/// `z-index`: `None` is `auto`.
pub(crate) fn z_index(
    value: &Component,
    relative: &Relative<'_>,
) -> Result<Option<i32>, StyleError> {
    match value.keyword() {
        Some(Keyword::Auto) => Ok(None),
        _ => crate::style::values::integer(value, relative).map(Some),
    }
}
impl Style {
    /// The `clip` region of an absolutely positioned box with border box `border`; `clip`
    /// does not apply to other boxes (CSS Masking 1 §6.1).
    pub(crate) fn clip_region(&self, border: crate::paint::Rect) -> Option<crate::paint::Rect> {
        let edges = self.clip.filter(|_| self.position.is_out_of_flow())?;
        let x = border.x + edges[LEFT].unwrap_or(0.0);
        let y = border.y + edges[TOP].unwrap_or(0.0);
        Some(crate::paint::Rect {
            x,
            y,
            width: (border.x + edges[RIGHT].unwrap_or(border.width) - x).max(0.0),
            height: (border.y + edges[BOTTOM].unwrap_or(border.height) - y).max(0.0),
        })
    }
    pub(crate) fn relative_offset(
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
