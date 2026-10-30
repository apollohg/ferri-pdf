//! CSS Transforms 2 §4.1.2 3D rendering contexts: the pieces of the context's planes, depth
//! sorted, each draw the part of their plane's content they hold.
use crate::graphics::{Rect, ScenePaint};
use crate::paint::{
    depth::{self, Piece},
    projection,
};
use crate::writer::{
    PdfError, bounds,
    display::{Painter, Space},
    painting,
};
use krilla::{geom::Transform, paint::FillRule, surface::Surface};

pub(super) fn draw(
    painter: &mut Painter<'_, '_, '_>,
    scene: &ScenePaint,
    space: Space,
    surface: &mut Surface<'_>,
) -> Result<(), PdfError> {
    let density = painter.density(space, surface);
    let mut planes = Vec::with_capacity(scene.planes.len());
    for plane in &scene.planes {
        let ink = bounds::ink(
            &plane.items,
            painter.lines,
            painter.raster.definitions,
            density,
        )?;
        planes.push((plane.matrix, ink));
    }
    for piece in depth::back_to_front(&planes) {
        draw_piece(painter, scene, &piece, space, surface)?;
    }
    Ok(())
}

/// Draws the part of a plane's content within `piece`.
fn draw_piece(
    painter: &mut Painter<'_, '_, '_>,
    scene: &ScenePaint,
    piece: &Piece,
    space: Space,
    surface: &mut Surface<'_>,
) -> Result<(), PdfError> {
    let plane = &scene.planes[piece.plane];
    let m = projection::of_plane(plane.matrix);
    if piece.cut {
        let outline: Vec<_> = piece
            .polygon
            .iter()
            .map(|&point| projection::apply(m, point))
            .collect();
        let path = painting::contour_path(&[crate::graphics::Contour::polygon(&outline)])?;
        surface.push_clip_path(&path, &FillRule::NonZero);
    }
    let drawn = match projection::affine_over(m, &piece.polygon) {
        Some([a, b, c, d, e, f]) => {
            surface.push_transform(&Transform::from_row(a, b, c, d, e, f));
            let drawn = painter.draw(&plane.items, space, surface);
            surface.pop();
            drawn
        }
        None => crate::writer::projection::draw(painter, m, &plane.items, space, surface),
    };
    if piece.cut {
        surface.pop();
    }
    drawn
}

/// A bound on what `scene` paints, from the content its planes project.
pub(super) fn ink(
    scene: &ScenePaint,
    lines: &[crate::layout::TextLine],
    definitions: &crate::document::svg::SvgDefinitions,
    density: f32,
) -> Result<Option<Rect>, PdfError> {
    let mut found: Option<Rect> = None;
    for plane in &scene.planes {
        if let Some(rect) = bounds::ink(&plane.items, lines, definitions, density)?
            .and_then(|rect| bounds::mapped(projection::of_plane(plane.matrix), rect))
        {
            found = Some(found.map_or(rect, |found| found.union(rect)));
        }
    }
    Ok(found)
}
