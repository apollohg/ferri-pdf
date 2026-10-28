//! CSS Position 3 §3.4: sticky positioning. Static media never scroll, so every scrollport
//! keeps its initial scroll position: the padding box of the nearest scroll container, or
//! else the page area, which is the viewport in paged media (CSS Page 3 §3).
use crate::graphics::Rect;
use crate::style::Style;

const TOP: usize = 0;
const RIGHT: usize = 1;
const BOTTOM: usize = 2;
const LEFT: usize = 3;

/// The shift of a sticky box whose border box is `border` and whose margins are `margins`
/// (top, right, bottom, left), inside `containing`, the content box of its containing block,
/// against `scrollport`. A containing block in a right-to-left inline direction has its
/// inline end on the left.
pub(super) fn offset(
    style: &Style,
    border: Rect,
    margins: [f32; 4],
    containing: Rect,
    scrollport: Rect,
    containing_rtl: bool,
) -> [f32; 2] {
    // Percentage insets refer to the scrollport's size.
    let inset = |side: usize, basis: f32| style.inset[side].resolve(basis);
    let x = axis(
        [border.x, border.x + border.width],
        [margins[LEFT], margins[RIGHT]],
        [containing.x, containing.x + containing.width],
        [scrollport.x, scrollport.x + scrollport.width],
        [
            inset(LEFT, scrollport.width),
            inset(RIGHT, scrollport.width),
        ],
        containing_rtl,
    );
    let y = axis(
        [border.y, border.y + border.height],
        [margins[TOP], margins[BOTTOM]],
        [containing.y, containing.y + containing.height],
        [scrollport.y, scrollport.y + scrollport.height],
        [
            inset(TOP, scrollport.height),
            inset(BOTTOM, scrollport.height),
        ],
        false,
    );
    [x, y]
}

/// One axis, with physical start and end edges; `end_is_start` makes the physical start
/// edge the end edge whose inset gives way.
fn axis(
    border: [f32; 2],
    margins: [f32; 2],
    containing: [f32; 2],
    scrollport: [f32; 2],
    insets: [Option<f32>; 2],
    end_is_start: bool,
) -> f32 {
    // An auto inset is a zero inset of the sticky view rectangle.
    let mut view = [
        scrollport[0] + insets[0].unwrap_or(0.0),
        scrollport[1] - insets[1].unwrap_or(0.0),
    ];
    // The end-edge inset gives way to fit the border box.
    let size = border[1] - border[0];
    if view[1] - view[0] < size {
        if end_is_start {
            view[0] = view[1] - size;
        } else {
            view[1] = view[0] + size;
        }
    }
    // The position box is the margin box, its margins limited to the space to the
    // containing block's edges.
    let position = [
        border[0] - margins[0].min((border[0] - containing[0]).max(0.0)),
        border[1] + margins[1].min((containing[1] - border[1]).max(0.0)),
    ];
    if insets[0].is_some() && border[0] < view[0] {
        return (view[0] - border[0])
            .min(containing[1] - position[1])
            .max(0.0);
    }
    if insets[1].is_some() && border[1] > view[1] {
        return (view[1] - border[1])
            .max(containing[0] - position[0])
            .min(0.0);
    }
    0.0
}

#[cfg(test)]
mod tests;
