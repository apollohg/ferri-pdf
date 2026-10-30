//! Display-list slices rendered to pixels for effects that PDF compositing cannot express:
//! the slice is written as a one-page PDF and rasterized, so every paint the writer emits
//! renders exactly as it would in place.
use crate::graphics::{DisplayItem, Rect};
use crate::writer::{
    PdfError,
    display::{Painter, Space},
};
use krilla::{
    Document,
    geom::{Size, Transform},
    image::Image,
    page::PageSettings,
    surface::Surface,
};

/// Raster effects resolve at the image DPI the caller requested, else at the scale the SVG
/// renderer rasterizes SVG filters at, four pixels per point (288 dpi): above the 200 to
/// 300 dpi that print workflows sample photographs at, so filtered content is never coarser
/// than the images beside it.
pub(super) struct RasterContext<'a> {
    pub(super) pixels_per_point: f32,
    pub(super) definitions: &'a crate::document::svg::SvgDefinitions,
}
impl<'a> RasterContext<'a> {
    pub(super) fn new(
        dpi: Option<krilla::image::ImageDpi>,
        definitions: &'a crate::document::svg::SvgDefinitions,
    ) -> Self {
        Self {
            pixels_per_point: pixels_per_point(dpi),
            definitions,
        }
    }
}

/// The density raster effects resolve at for the image DPI the caller requested.
pub(super) fn pixels_per_point(dpi: Option<krilla::image::ImageDpi>) -> f32 {
    const POINTS_PER_INCH: f32 = 72.0;
    dpi.map_or_else(
        || crate::svg::krilla_svg::SvgSettings::default().filter_scale,
        |dpi| dpi.get() / POINTS_PER_INCH,
    )
}

/// A premultiplied raster of `area`, at `density` pixels per unit of the drawing space.
pub(super) struct Raster {
    pub(super) pixmap: tiny_skia::Pixmap,
    pub(super) area: Rect,
    pub(super) density: f32,
}
impl Raster {
    /// The drawing-space bounds of the pixels with any coverage.
    pub(super) fn ink(&self) -> Option<Rect> {
        let width = self.pixmap.width() as usize;
        let mut bounds: Option<[usize; 4]> = None;
        for (index, pixel) in self.pixmap.pixels().iter().enumerate() {
            if pixel.alpha() > 0 {
                let [column, row] = [index % width, index / width];
                let [left, top, right, bottom] = bounds.get_or_insert([column, row, column, row]);
                *left = (*left).min(column);
                *top = (*top).min(row);
                *right = (*right).max(column);
                *bottom = (*bottom).max(row);
            }
        }
        bounds.map(|[left, top, right, bottom]| Rect {
            x: self.area.x + left as f32 / self.density,
            y: self.area.y + top as f32 / self.density,
            width: (right + 1 - left) as f32 / self.density,
            height: (bottom + 1 - top) as f32 / self.density,
        })
    }
    /// The whole pixels covering `keep`, or `None` when none do.
    pub(super) fn cropped(&self, keep: Rect) -> Option<Self> {
        let pixel = |value: f32, limit: u32| (value * self.density).clamp(0.0, limit as f32);
        let (width, height) = (self.pixmap.width(), self.pixmap.height());
        let left = pixel(keep.x - self.area.x, width).floor() as u32;
        let top = pixel(keep.y - self.area.y, height).floor() as u32;
        let right = pixel(keep.x + keep.width - self.area.x, width).ceil() as u32;
        let bottom = pixel(keep.y + keep.height - self.area.y, height).ceil() as u32;
        let pixmap = self.pixmap.clone_rect(tiny_skia::IntRect::from_ltrb(
            left as i32,
            top as i32,
            right as i32,
            bottom as i32,
        )?)?;
        Some(Self {
            pixmap,
            area: Rect {
                x: self.area.x + left as f32 / self.density,
                y: self.area.y + top as f32 / self.density,
                width: (right - left) as f32 / self.density,
                height: (bottom - top) as f32 / self.density,
            },
            density: self.density,
        })
    }
    /// Maps the drawing space onto the raster's pixels.
    pub(super) fn transform(&self) -> tiny_skia::Transform {
        tiny_skia::Transform::from_scale(self.density, self.density)
            .pre_translate(-self.area.x, -self.area.y)
    }
}

/// Renders `items` as they would draw at `area` of the current drawing space, or `None`
/// when the area covers no pixel.
pub(super) fn rasterize(
    painter: &mut Painter<'_, '_, '_>,
    items: &[DisplayItem],
    area: Rect,
    density: f32,
) -> Result<Option<Raster>, PdfError> {
    render(painter, area, density, |painter, space, surface| {
        painter.draw(items, space, surface)
    })
}

/// Renders what `draw` paints in the current drawing space at `area`, or `None` when the area
/// covers no pixel.
pub(super) fn render(
    painter: &mut Painter<'_, '_, '_>,
    area: Rect,
    density: f32,
    draw: impl FnOnce(&mut Painter<'_, '_, '_>, Space, &mut Surface<'_>) -> Result<(), PdfError>,
) -> Result<Option<Raster>, PdfError> {
    let scale = density / painter.raster.pixels_per_point;
    render_surface(area, density, |viewport, surface| {
        draw(painter, Space { viewport, scale }, surface)
    })
}

/// Renders what `draw` paints at `area` of a drawing space whose visible area it is given, or
/// `None` when the area covers no pixel.
pub(super) fn render_surface(
    area: Rect,
    density: f32,
    draw: impl FnOnce(Rect, &mut Surface<'_>) -> Result<(), PdfError>,
) -> Result<Option<Raster>, PdfError> {
    let columns = (area.width * density).ceil();
    let rows = (area.height * density).ceil();
    if !(columns >= 1.0 && rows >= 1.0) {
        return Ok(None);
    }
    let limit = f32::from(u16::MAX);
    if columns > limit || rows > limit {
        return Err(PdfError::RasterSize {
            width: columns,
            height: rows,
        });
    }
    // The renderer truncates page sizes to whole pixels; half a pixel more keeps the count.
    const HALF_PIXEL: f32 = 0.5;
    let page = [
        (columns + HALF_PIXEL) / density,
        (rows + HALF_PIXEL) / density,
    ];
    let area = Rect {
        width: columns / density,
        height: rows / density,
        ..area
    };
    let mut document = Document::new();
    let settings = PageSettings::from_wh(page[0], page[1]).ok_or(PdfError::InvalidPageSize {
        width: page[0],
        height: page[1],
    })?;
    let mut pdf_page = document.start_page_with(settings);
    let mut surface = pdf_page.surface();
    surface.push_transform(&Transform::from_translate(-area.x, -area.y));
    let drawn = draw(
        Rect {
            x: 0.0,
            y: 0.0,
            width: page[0],
            height: page[1],
        },
        &mut surface,
    );
    surface.pop();
    surface.finish();
    pdf_page.finish();
    drawn?;
    let bytes = document.finish()?;
    let pdf = hayro::hayro_syntax::Pdf::new(bytes).map_err(|_| PdfError::Raster)?;
    let page = pdf.pages().first().ok_or(PdfError::Raster)?;
    let pixels = hayro::render(
        page,
        &hayro::RenderCache::new(),
        &hayro::hayro_interpret::InterpreterSettings::default(),
        &hayro::RenderSettings::default(),
        &hayro::PixmapSettings {
            x_scale: density,
            y_scale: density,
            bg_color: hayro::vello_cpu::color::palette::css::TRANSPARENT,
        },
    );
    let size = tiny_skia::IntSize::from_wh(u32::from(pixels.width()), u32::from(pixels.height()))
        .ok_or(PdfError::Raster)?;
    let pixmap = tiny_skia::Pixmap::from_vec(pixels.data_as_u8_slice().to_vec(), size)
        .ok_or(PdfError::Raster)?;
    Ok(Some(Raster {
        pixmap,
        area,
        density,
    }))
}

/// Draws `raster` in place.
pub(super) fn embed(raster: &Raster, surface: &mut Surface<'_>) -> Result<(), PdfError> {
    let (width, height) = (raster.pixmap.width(), raster.pixmap.height());
    let rgba = raster
        .pixmap
        .pixels()
        .iter()
        .flat_map(|pixel| {
            let color = pixel.demultiply();
            [color.red(), color.green(), color.blue(), color.alpha()]
        })
        .collect();
    let image = Image::from_rgba8(rgba, width, height);
    let size =
        Size::from_wh(raster.area.width, raster.area.height).ok_or(PdfError::InvalidGeometry)?;
    surface.push_transform(&Transform::from_translate(raster.area.x, raster.area.y));
    surface.draw_image(image, size);
    surface.pop();
    Ok(())
}
