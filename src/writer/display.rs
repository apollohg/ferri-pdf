//! Display lists drawn onto a surface: scopes nest pushes and pops, filtered groups are
//! drawn through their filters, and backdrops and additive groups read back what the
//! list painted before them.
use crate::graphics::{BlendMode, DisplayItem, Rect};
use crate::layout::TextLine;
use crate::writer::{
    PdfError, backdrops::Prepared, images::ImageWriter, painting, raster::RasterContext,
    text::TextWriter,
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
    pub(super) prepared: Prepared,
}

/// Where the content of an enclosing group or Backdrop Root starts in the items a frame draws,
/// and the raster density of its drawing space there.
#[derive(Clone, Copy)]
pub(super) struct Origin {
    pub(super) index: usize,
    pub(super) density: f32,
}

#[derive(Clone, Copy)]
enum Kind {
    Clip,
    Transform([f32; 6]),
    /// A transparency group, whose content is the backdrop of blending inside it.
    Group(Origin),
    /// A Backdrop Root (Filter Effects 2 §3).
    Root(Origin),
}

/// An open push: the surface pushes to pop and the drawing space to restore.
struct Scope {
    pops: usize,
    space: Space,
    kind: Kind,
}

/// The open scopes of a frame whose items start at `frame`.
pub(super) struct Scopes<'s> {
    frame: Origin,
    open: &'s [Scope],
}
impl Scopes<'_> {
    /// The innermost enclosing Backdrop Root's content start, or the frame's, which the
    /// document root, filters and masks begin, and the transform from the current drawing
    /// space to the space there.
    pub(super) fn backdrop_root(&self) -> (Origin, tiny_skia::Transform) {
        self.innermost(|kind| match kind {
            Kind::Root(origin) => Some(origin),
            Kind::Clip | Kind::Transform(_) | Kind::Group(_) => None,
        })
    }
    /// The innermost enclosing transparency group's content start, likewise.
    pub(super) fn group(&self) -> (Origin, tiny_skia::Transform) {
        self.innermost(|kind| match kind {
            Kind::Group(origin) => Some(origin),
            Kind::Clip | Kind::Transform(_) | Kind::Root(_) => None,
        })
    }
    fn innermost(&self, origin: impl Fn(Kind) -> Option<Origin>) -> (Origin, tiny_skia::Transform) {
        let found = self
            .open
            .iter()
            .enumerate()
            .rev()
            .find_map(|(index, scope)| origin(scope.kind).map(|origin| (index + 1, origin)));
        let (inner, origin) = found.unwrap_or((0, self.frame));
        let transform =
            self.open[inner..]
                .iter()
                .fold(
                    tiny_skia::Transform::identity(),
                    |transform, scope| match scope.kind {
                        Kind::Transform(matrix) => transform.pre_concat(matrix_transform(matrix)),
                        Kind::Clip | Kind::Group(_) | Kind::Root(_) => transform,
                    },
                );
        (origin, transform)
    }
}

/// A display-list affine `[a, b, c, d, e, f]` as a transform.
pub(super) fn matrix_transform([a, b, c, d, e, f]: [f32; 6]) -> tiny_skia::Transform {
    tiny_skia::Transform::from_row(a, b, c, d, e, f)
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

impl Painter<'_, '_, '_> {
    /// Draws `items`, which open and close their own scopes.
    pub(super) fn draw(
        &mut self,
        items: &[DisplayItem],
        space: Space,
        surface: &mut Surface<'_>,
    ) -> Result<(), PdfError> {
        self.draw_frame(items, space, surface, false)
    }

    /// Draws the start of a display list, closing the scopes still open where it ends.
    pub(super) fn draw_prefix(
        &mut self,
        items: &[DisplayItem],
        space: Space,
        surface: &mut Surface<'_>,
    ) -> Result<(), PdfError> {
        self.draw_frame(items, space, surface, true)
    }

    /// The raster density of `space` as the surface currently draws it.
    pub(super) fn density(&self, space: Space, surface: &Surface<'_>) -> f32 {
        self.raster.pixels_per_point
            * space.scale
            * painting::transform_scale(surface.cur_transform())
    }

    fn draw_frame(
        &mut self,
        items: &[DisplayItem],
        space: Space,
        surface: &mut Surface<'_>,
        prefix: bool,
    ) -> Result<(), PdfError> {
        let mut scopes = Vec::new();
        let prepared = self.prepared.mark();
        let result = self.draw_scoped(items, space, &mut scopes, surface);
        let unclosed = !scopes.is_empty();
        while let Some(scope) = scopes.pop() {
            painting::pop_clips(scope.pops, surface);
        }
        self.prepared.release(prepared);
        result?;
        if unclosed && !prefix {
            return Err(PdfError::InvalidGeometry);
        }
        Ok(())
    }

    fn draw_scoped(
        &mut self,
        items: &[DisplayItem],
        mut space: Space,
        scopes: &mut Vec<Scope>,
        surface: &mut Surface<'_>,
    ) -> Result<(), PdfError> {
        let frame = Origin {
            index: 0,
            density: self.density(space, surface),
        };
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
                DisplayItem::Frame(frame) => {
                    let [a, b, c, d, e, f] = frame.matrix;
                    surface.push_transform(&Transform::from_row(a, b, c, d, e, f));
                    let result = Painter {
                        lines: &frame.lines,
                        page: self.page,
                        page_area: frame.area,
                        text: self.text,
                        fonts: self.fonts,
                        images: self.images,
                        raster: self.raster,
                        prepared: Prepared::default(),
                    }
                    .draw(&frame.items, space, surface);
                    surface.pop();
                    result?;
                }
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
                DisplayItem::Backdrop(backdrop) => {
                    crate::writer::backdrops::draw(self, backdrop, item, space, surface)?;
                }
                DisplayItem::PushClip(clip) => {
                    scopes.push(Scope {
                        pops: painting::push_clip(clip, space.viewport, surface)?,
                        space,
                        kind: Kind::Clip,
                    });
                }
                DisplayItem::PushTransform(matrix) => {
                    let [a, b, c, d, e, f] = *matrix;
                    surface.push_transform(&Transform::from_row(a, b, c, d, e, f));
                    scopes.push(Scope {
                        pops: 1,
                        space,
                        kind: Kind::Transform(*matrix),
                    });
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
                    scopes.push(Scope {
                        pops: 1,
                        space,
                        kind: Kind::Group(Origin {
                            index,
                            density: self.density(nested, surface),
                        }),
                    });
                    space = nested;
                }
                DisplayItem::PushBlend(mode @ (BlendMode::PlusDarker | BlendMode::PlusLighter)) => {
                    let end = index + group_length(&items[index..])?;
                    let open = Scopes {
                        frame,
                        open: scopes,
                    };
                    crate::writer::backdrops::additive(
                        self,
                        *mode,
                        items,
                        index - 1..end,
                        &open,
                        space,
                        surface,
                    )?;
                    index = end + 1;
                }
                DisplayItem::PushBlend(mode) => {
                    let nested = space.nested(surface)?;
                    let pops = painting::push_blend(*mode, surface)?;
                    scopes.push(Scope {
                        pops,
                        space,
                        kind: Kind::Group(Origin {
                            index,
                            density: self.density(nested, surface),
                        }),
                    });
                    space = nested;
                }
                DisplayItem::PushBackdropRoot => {
                    let open = Scopes {
                        frame,
                        open: scopes,
                    };
                    crate::writer::backdrops::prepare(self, items, index - 1, &open)?;
                    scopes.push(Scope {
                        pops: 0,
                        space,
                        kind: Kind::Root(Origin {
                            index,
                            density: self.density(space, surface),
                        }),
                    });
                }
                DisplayItem::PushFilter(filter) => {
                    let end = index + group_length(&items[index..])?;
                    crate::writer::filters::draw(self, filter, &items[index..end], space, surface)?;
                    index = end + 1;
                }
                DisplayItem::PushMask(mask) => {
                    let end = index + group_length(&items[index..])?;
                    crate::writer::masks::draw(self, mask, &items[index..end], space, surface)?;
                    index = end + 1;
                }
                DisplayItem::Pop => {
                    let scope = scopes.pop().ok_or(PdfError::InvalidGeometry)?;
                    painting::pop_clips(scope.pops, surface);
                    space = scope.space;
                }
            }
        }
        Ok(())
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
            prepared: Prepared::default(),
        }
        .draw(&view.items, local, surface);
        surface.pop();
        painting::pop_clips(clips, surface);
        drawn
    }
}

/// The number of items before the `Pop` closing a group that `items` follow.
pub(super) fn group_length(items: &[DisplayItem]) -> Result<usize, PdfError> {
    let mut depth = 0_usize;
    for (index, item) in items.iter().enumerate() {
        match item {
            DisplayItem::PushClip(_)
            | DisplayItem::PushTransform(_)
            | DisplayItem::PushOpacity(_)
            | DisplayItem::PushBlend(_)
            | DisplayItem::PushFilter(_)
            | DisplayItem::PushMask(_)
            | DisplayItem::PushBackdropRoot => depth += 1,
            DisplayItem::Pop if depth == 0 => return Ok(index),
            DisplayItem::Pop => depth -= 1,
            DisplayItem::Frame(_)
            | DisplayItem::Image(_)
            | DisplayItem::Box(_)
            | DisplayItem::Text { .. }
            | DisplayItem::TextShadow(_)
            | DisplayItem::Decoration(_)
            | DisplayItem::Backdrop(_) => {}
        }
    }
    Err(PdfError::InvalidGeometry)
}
