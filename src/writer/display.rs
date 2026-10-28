//! Display lists drawn onto a surface: scopes nest pushes and pops, and filtered groups are
//! drawn through their filters.
use crate::graphics::{DisplayItem, Rect};
use crate::layout::TextLine;
use crate::writer::{
    PdfError, images::ImageWriter, painting, raster::RasterContext, text::TextWriter,
};
use krilla::{geom::Transform, num::NormalizedF32, surface::Surface};

/// What one page's display list draws with.
pub(super) struct Painter<'a, 'w, 'f> {
    pub(super) lines: &'a [TextLine],
    pub(super) page: usize,
    /// Page content area and positioning area of document `fixed` backgrounds.
    pub(super) page_area: Rect,
    pub(super) text: &'w mut TextWriter<'f>,
    pub(super) fonts: &'f crate::fonts::FontBook,
    pub(super) images: &'w mut ImageWriter,
    pub(super) raster: &'w RasterContext<'w>,
}

/// The drawing space of a surface: the physical area visible in it, and how many page points
/// one of its units spans, which raster effects resolve at.
#[derive(Clone, Copy)]
pub(super) struct Space {
    pub(super) viewport: Rect,
    pub(super) scale: f32,
}
impl Space {
    /// The space nested groups start in: krilla draws a group from the identity transform.
    fn nested(self, surface: &Surface<'_>) -> Result<Self, PdfError> {
        Ok(Self {
            viewport: painting::local_bounds(self.viewport, surface)?,
            scale: self.scale * painting::transform_scale(surface.cur_transform()),
        })
    }
}

impl<'a> Painter<'a, '_, '_> {
    /// Draws `items`, which open and close their own scopes.
    pub(super) fn draw(
        &mut self,
        items: &'a [DisplayItem],
        space: Space,
        surface: &mut Surface<'_>,
    ) -> Result<(), PdfError> {
        let mut scopes = Vec::new();
        let result = self.draw_scoped(items, space, &mut scopes, surface);
        while let Some((count, _)) = scopes.pop() {
            painting::pop_clips(count, surface);
        }
        result
    }

    fn draw_scoped(
        &mut self,
        items: &'a [DisplayItem],
        mut space: Space,
        scopes: &mut Vec<(usize, Space)>,
        surface: &mut Surface<'_>,
    ) -> Result<(), PdfError> {
        let mut index = 0;
        while let Some(item) = items.get(index) {
            index += 1;
            match item {
                DisplayItem::Image(paint) => match paint.image.as_ref() {
                    crate::images::Image::Viewport(view) => {
                        self.draw_viewport(paint, view, space, surface)?;
                    }
                    _ => self.images.draw(paint, space.viewport, surface)?,
                },
                DisplayItem::Box(box_paint) => painting::draw(
                    box_paint,
                    self.page_area,
                    space.viewport,
                    surface,
                    self.images,
                )?,
                DisplayItem::Text { line } => {
                    let fragment = self
                        .lines
                        .get(*line)
                        .ok_or(PdfError::InvalidTextReference {
                            page: self.page,
                            line: *line,
                        })?;
                    self.text.draw(fragment, surface)?;
                }
                DisplayItem::TextShadow(shadow) => crate::writer::text_shadows::draw(
                    shadow,
                    self.text,
                    self.fonts,
                    space.viewport,
                    surface,
                )?,
                DisplayItem::Decoration(decoration) => {
                    for fill in decoration.fills() {
                        painting::draw_fill(fill, space.viewport, surface)?;
                    }
                }
                DisplayItem::PushClip(clip) => {
                    scopes.push((painting::push_clip(clip, space.viewport, surface)?, space));
                }
                DisplayItem::PushTransform([a, b, c, d, e, f]) => {
                    surface.push_transform(&Transform::from_row(*a, *b, *c, *d, *e, *f));
                    scopes.push((1, space));
                }
                DisplayItem::PushOpacity(alpha) => {
                    let opacity =
                        NormalizedF32::new(*alpha).ok_or(PdfError::InvalidAlpha(*alpha))?;
                    let nested = if opacity == NormalizedF32::ONE {
                        space
                    } else {
                        space.nested(surface)?
                    };
                    surface.push_opacity(opacity);
                    scopes.push((1, space));
                    space = nested;
                }
                DisplayItem::PushBlend(mode) => {
                    let nested = space.nested(surface)?;
                    scopes.push((painting::push_blend(*mode, surface), space));
                    space = nested;
                }
                DisplayItem::PushFilter(filter) => {
                    let end = index + group_length(&items[index..])?;
                    crate::writer::filters::draw(self, filter, &items[index..end], space, surface)?;
                    index = end + 1;
                }
                DisplayItem::Pop => {
                    let (count, previous) = scopes.pop().ok_or(PdfError::InvalidGeometry)?;
                    painting::pop_clips(count, surface);
                    space = previous;
                }
            }
        }
        if scopes.is_empty() {
            Ok(())
        } else {
            Err(PdfError::InvalidGeometry)
        }
    }
}

impl Painter<'_, '_, '_> {
    /// Paints a nested document's viewport at the paint rectangle's origin, with its own
    /// text lines, faces and SVG definitions.
    fn draw_viewport(
        &mut self,
        paint: &crate::graphics::ImagePaint,
        view: &crate::layout::ViewportPaint,
        space: Space,
        surface: &mut Surface<'_>,
    ) -> Result<(), PdfError> {
        let clips = paint
            .clip
            .as_ref()
            .map(|clip| painting::push_clip(clip, space.viewport, surface))
            .transpose()?
            .unwrap_or(0);
        let offset = [paint.rect.x - view.area.x, paint.rect.y - view.area.y];
        surface.push_transform(&Transform::from_translate(offset[0], offset[1]));
        let local = Space {
            viewport: Rect {
                x: space.viewport.x - offset[0],
                y: space.viewport.y - offset[1],
                ..space.viewport
            },
            ..space
        };
        let raster = RasterContext {
            pixels_per_point: self.raster.pixels_per_point,
            definitions: &view.definitions,
        };
        let mut text = TextWriter::new(&view.fonts);
        let drawn = Painter {
            lines: &view.lines,
            page: 0,
            page_area: view.area,
            text: &mut text,
            fonts: &view.fonts,
            images: self.images,
            raster: &raster,
        }
        .draw(&view.items, local, surface);
        surface.pop();
        painting::pop_clips(clips, surface);
        drawn
    }
}

/// The number of items before the `Pop` closing a group that `items` follow.
fn group_length(items: &[DisplayItem]) -> Result<usize, PdfError> {
    let mut depth = 0_usize;
    for (index, item) in items.iter().enumerate() {
        match item {
            DisplayItem::PushClip(_)
            | DisplayItem::PushTransform(_)
            | DisplayItem::PushOpacity(_)
            | DisplayItem::PushBlend(_)
            | DisplayItem::PushFilter(_) => depth += 1,
            DisplayItem::Pop if depth == 0 => return Ok(index),
            DisplayItem::Pop => depth -= 1,
            DisplayItem::Image(_)
            | DisplayItem::Box(_)
            | DisplayItem::Text { .. }
            | DisplayItem::TextShadow(_)
            | DisplayItem::Decoration(_) => {}
        }
    }
    Err(PdfError::InvalidGeometry)
}
