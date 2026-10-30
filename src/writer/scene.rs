//! CSS Transforms 2 §4.1.2 3D rendering contexts: the pieces of the context's planes, depth
//! sorted, each draw the part of their plane's content they hold. The SVG content of an `svg`
//! element whose used `transform-style` is `preserve-3d` participates in the context (§4.1.2,
//! §11): the planes of the context its root establishes in the SVG renderer join the document's,
//! after the plane holding the element, and that plane draws the rest of its content.
use crate::graphics::{DisplayItem, ImagePaint, Rect, ScenePaint};
use crate::paint::{
    depth::{self, Piece},
    projection,
};
use crate::writer::{
    PdfError, bounds,
    display::{Painter, Space},
    images::Decoded,
    painting,
};
use krilla::{geom::Transform, paint::FillRule, surface::Surface};
use std::{borrow::Cow, sync::Arc};

/// One plane of the depth-sorted context.
enum Plane<'s> {
    /// A plane of the document's, which draws `items`.
    Document {
        items: Cow<'s, [DisplayItem]>,
        matrix: [f32; 16],
    },
    /// Plane `index` of the 3D rendering context the root of `paint`'s SVG content establishes.
    Svg {
        paint: &'s ImagePaint,
        tree: Arc<crate::svg::usvg::Tree>,
        filter_scale: f32,
        index: usize,
        matrix: [f32; 16],
    },
}

impl Plane<'_> {
    fn matrix(&self) -> [f32; 16] {
        match self {
            Self::Document { matrix, .. } | Self::Svg { matrix, .. } => *matrix,
        }
    }
    /// The SVG renderer's plane group of an SVG plane.
    fn svg_plane(tree: &crate::svg::usvg::Tree, index: usize) -> Option<&crate::svg::usvg::Group> {
        let (scene, _) = tree.root_scene()?;
        scene.scene_planes().get(index).map(|(plane, _)| *plane)
    }
}

pub(super) fn draw(
    painter: &mut Painter<'_, '_, '_>,
    scene: &ScenePaint,
    space: Space,
    surface: &mut Surface<'_>,
) -> Result<(), PdfError> {
    let density = painter.density(space, surface);
    let mut planes = Vec::with_capacity(scene.planes.len());
    for plane in &scene.planes {
        let joined = joined_svg(painter, &plane.items)?;
        let items = match &joined {
            Some((index, ..)) => {
                let mut items = plane.items.clone();
                items.remove(*index);
                Cow::Owned(items)
            }
            None => Cow::Borrowed(plane.items.as_slice()),
        };
        planes.push(Plane::Document {
            items,
            matrix: plane.matrix,
        });
        if let Some((index, tree, filter_scale)) = joined {
            let DisplayItem::Image(paint) = &plane.items[index] else {
                return Err(PdfError::InvalidGeometry);
            };
            let Some((root, to_scene)) = tree.root_scene() else {
                return Err(PdfError::InvalidGeometry);
            };
            let size = tree.size();
            let to_plane = tiny_skia::Transform::from_row(
                paint.rect.width / size.width(),
                0.0,
                0.0,
                paint.rect.height / size.height(),
                paint.rect.x,
                paint.rect.y,
            )
            .pre_concat(to_scene);
            let placement = [
                to_plane.sx,
                to_plane.ky,
                to_plane.kx,
                to_plane.sy,
                to_plane.tx,
                to_plane.ty,
            ];
            // Depth scales as the plane does, where the placement keeps shapes.
            let depth = (to_plane.sx * to_plane.sy - to_plane.kx * to_plane.ky)
                .abs()
                .sqrt();
            let deepened = crate::style::transform::scaling(1.0, 1.0, depth);
            for (index, (_, matrix)) in root.scene_planes().into_iter().enumerate() {
                planes.push(Plane::Svg {
                    paint,
                    tree: tree.clone(),
                    filter_scale,
                    index,
                    matrix: crate::style::transform::multiply(
                        &plane.matrix,
                        &crate::style::transform::conjugated(
                            &crate::style::transform::multiply(&deepened, &matrix),
                            placement,
                            crate::style::transform::IDENTITY_MATRIX,
                        ),
                    ),
                });
            }
        }
    }
    let mut geometry = Vec::with_capacity(planes.len());
    for plane in &planes {
        let ink = match plane {
            Plane::Document { items, .. } => {
                bounds::ink(items, painter.lines, painter.raster.definitions, density)?
            }
            Plane::Svg { tree, index, .. } => {
                Plane::svg_plane(tree, *index).and_then(crate::svg::usvg::Group::content_ink)
            }
        };
        geometry.push((plane.matrix(), ink));
    }
    for piece in depth::back_to_front(&geometry) {
        draw_piece(painter, &planes[piece.plane], &piece, space, surface)?;
    }
    Ok(())
}

/// The index of the item of `items`, outside any group, that draws SVG content the document
/// styles whose root establishes a 3D rendering context, with its tree and the density its
/// filters rasterize at.
fn joined_svg(
    painter: &mut Painter<'_, '_, '_>,
    items: &[DisplayItem],
) -> Result<Option<(usize, Arc<crate::svg::usvg::Tree>, f32)>, PdfError> {
    let mut depth = 0_usize;
    for (index, item) in items.iter().enumerate() {
        match item {
            DisplayItem::Pop => depth = depth.saturating_sub(1),
            push if push.is_push() => depth += 1,
            DisplayItem::Image(paint) if depth == 0 => {
                let crate::images::Image::Svg(svg) = paint.image.as_ref() else {
                    continue;
                };
                if !svg.is_styled() {
                    continue;
                }
                if let Some(Decoded::Vector(tree, filter_scale)) = painter.images.decode(paint)?
                    && tree.root_scene().is_some()
                {
                    return Ok(Some((index, tree, filter_scale)));
                }
            }
            _ => {}
        }
    }
    Ok(None)
}

/// Draws the part of a plane's content within `piece`.
fn draw_piece(
    painter: &mut Painter<'_, '_, '_>,
    plane: &Plane<'_>,
    piece: &Piece,
    space: Space,
    surface: &mut Surface<'_>,
) -> Result<(), PdfError> {
    let m = projection::of_plane(plane.matrix());
    if piece.cut {
        let outline: Vec<_> = piece
            .polygon
            .iter()
            .map(|&point| projection::apply(m, point))
            .collect();
        let path = painting::contour_path(&[crate::graphics::Contour::polygon(&outline)])?;
        surface.push_clip_path(&path, &FillRule::NonZero);
    }
    let drawn = match plane {
        Plane::Document { items, .. } => match projection::affine_over(m, &piece.polygon) {
            Some([a, b, c, d, e, f]) => {
                surface.push_transform(&Transform::from_row(a, b, c, d, e, f));
                let drawn = painter.draw(items, space, surface);
                surface.pop();
                drawn
            }
            None => crate::writer::projection::draw(painter, m, items, space, surface),
        },
        Plane::Svg {
            paint,
            tree,
            filter_scale,
            index,
            ..
        } => draw_svg_plane(
            painter,
            SvgPlane {
                paint,
                tree,
                filter_scale: *filter_scale,
                index: *index,
            },
            m,
            &piece.polygon,
            space,
            surface,
        ),
    };
    if piece.cut {
        surface.pop();
    }
    drawn
}

/// A plane of the 3D rendering context the root of an SVG image's content establishes.
#[derive(Clone, Copy)]
struct SvgPlane<'p> {
    paint: &'p ImagePaint,
    tree: &'p crate::svg::usvg::Tree,
    filter_scale: f32,
    index: usize,
}

/// Draws the part within `polygon` of `plane`, whose content `projection` maps into the
/// current space.
fn draw_svg_plane(
    painter: &mut Painter<'_, '_, '_>,
    plane: SvgPlane<'_>,
    projection: crate::graphics::Projection,
    polygon: &[[f32; 2]],
    space: Space,
    surface: &mut Surface<'_>,
) -> Result<(), PdfError> {
    let crate::images::Image::Svg(svg) = plane.paint.image.as_ref() else {
        return Err(PdfError::InvalidGeometry);
    };
    let content = Plane::svg_plane(plane.tree, plane.index).ok_or(PdfError::InvalidGeometry)?;
    let density = painter.density(space, surface);
    let visible = painting::local_bounds(space.viewport, surface)?;
    let errors = std::cell::RefCell::new(std::collections::BTreeSet::new());
    let settings = |layer_masks| crate::svg::krilla_svg::SvgSettings {
        jpeg_images: Some(svg.jpeg_images()),
        image_errors: Some(&errors),
        filter_scale: plane.filter_scale,
        layer_masks,
        ..Default::default()
    };
    if svg.has_layer_masks() {
        let mask_density = painter.raster.pixels_per_point * space.scale;
        let layer_masks = crate::writer::masks::SvgLayerMasks::new(painter, mask_density, &[]);
        crate::svg::krilla_svg::draw_scene_plane(
            plane.tree,
            content,
            projection,
            polygon,
            visible,
            density,
            settings(Some(&layer_masks)),
            surface,
        );
        layer_masks.finish()?;
    } else {
        crate::svg::krilla_svg::draw_scene_plane(
            plane.tree,
            content,
            projection,
            polygon,
            visible,
            density,
            settings(None),
            surface,
        );
    }
    painter
        .images
        .report_svg_errors(plane.paint, errors.into_inner());
    Ok(())
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
