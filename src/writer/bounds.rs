//! Conservative bounds on what display items paint, in the drawing space they start in.
use crate::graphics::{DisplayItem, Rect};
use crate::layout::TextLine;
use crate::paint::projection;
use crate::writer::{PdfError, display::group_length, filters, text_shadows};

/// The area `items` can paint, whose text items draw `lines`, with raster effects resolved at
/// `density`; `None` when they paint nothing.
pub(super) fn ink(
    items: &[DisplayItem],
    lines: &[TextLine],
    definitions: &crate::document::svg::SvgDefinitions,
    density: f32,
) -> Result<Option<Rect>, PdfError> {
    let mut bounds: Option<Rect> = None;
    let mut index = 0;
    while let Some(item) = items.get(index) {
        index += 1;
        let rect = match item {
            // Outlines and shadows paint around boxes too empty to ink themselves.
            DisplayItem::Box(paint) => (paint.has_ink()
                || paint.decoration.outline.is_drawn()
                || paint.decoration.has_shadows())
            .then(|| paint.ink()),
            DisplayItem::Image(paint) => Some(paint.visible_rect()),
            DisplayItem::Text { line } => {
                let line = lines.get(*line).ok_or(PdfError::InvalidGeometry)?;
                line.runs
                    .iter()
                    .filter_map(|run| run.ink_rect([line.x, line.baseline]))
                    .reduce(Rect::union)
            }
            DisplayItem::Decoration(decoration) => decoration
                .fills()
                .iter()
                .filter_map(|fill| crate::paint::shape_bounds(&fill.shape))
                .reduce(Rect::union),
            DisplayItem::TextShadow(shadow) => text_shadows::bounds(shadow),
            DisplayItem::Backdrop(backdrop) => Some(backdrop.border.rect),
            DisplayItem::Frame(frame) => ink(&frame.items, &frame.lines, definitions, density)?
                .and_then(|rect| mapped(projection::from_affine(frame.matrix), rect)),
            DisplayItem::Scene(scene) => {
                crate::writer::scene::ink(scene, lines, definitions, density)?
            }
            DisplayItem::Pop => return Err(PdfError::InvalidGeometry),
            push => {
                let end = index + group_length(&items[index..])?;
                let inner = ink(&items[index..end], lines, definitions, density)?;
                index = end + 1;
                match (push, inner) {
                    (_, None) => None,
                    (DisplayItem::PushClip(clip), Some(rect)) => {
                        Some(clip.bounds().clip_rect(rect))
                    }
                    (DisplayItem::PushTransform(matrix), Some(rect)) => {
                        mapped(projection::from_affine(*matrix), rect)
                    }
                    (DisplayItem::PushProjection(matrix), Some(rect)) => mapped(*matrix, rect),
                    (DisplayItem::PushFilter(filter), Some(rect)) => {
                        Some(filtered(filter, rect, definitions, density))
                    }
                    (_, inner) => inner,
                }
            }
        };
        if let Some(rect) = rect {
            bounds = Some(bounds.map_or(rect, |bounds| bounds.union(rect)));
        }
    }
    Ok(bounds)
}

/// The bounds of `rect` under `m`, cut where it passes behind the viewer.
pub(super) fn mapped(m: projection::Projection, rect: Rect) -> Option<Rect> {
    projection::bounds(&projection::map_polygon(m, projection::corners(rect)))
}

/// Content `ink` grown by the reach of `filter`'s chain and the regions of its referenced
/// filters, where primitives may generate content (Filter Effects 1 §13.2).
fn filtered(
    filter: &crate::graphics::FilterPaint,
    ink: Rect,
    definitions: &crate::document::svg::SvgDefinitions,
    density: f32,
) -> Rect {
    // §5: a reference to anything but a filter element ignores the whole chain.
    let Some(steps) = filters::steps(filter, definitions) else {
        return ink;
    };
    let spread: f32 = steps.iter().map(|step| step.spread(density)).sum();
    steps
        .iter()
        .filter_map(filters::Step::region)
        .fold(filters::expand(ink, spread), Rect::union)
}
