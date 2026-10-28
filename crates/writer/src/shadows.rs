//! Box shadows: sharp shadows are vector fills; blurred ones fill their colour through a
//! luminosity soft mask holding the blurred alpha.
use crate::{
    PdfError,
    painting::{contour_path, fill, rect_path},
    pixels::MaskPixels,
};
use ferri_pdf_graphics::{ShadowPaint, ShadowShape};
use krilla::{
    geom::{Size, Transform},
    image::Image,
    mask::{Mask, MaskType},
    paint::{Fill, FillRule},
    surface::Surface,
};

pub(super) fn draw(shadow: &ShadowPaint, surface: &mut Surface<'_>) -> Result<(), PdfError> {
    let clip = contour_path(&shadow.clip)?;
    let color = fill(shadow.color)?;
    if shadow.bounds.width <= 0.0 || shadow.bounds.height <= 0.0 {
        return Ok(());
    }
    let bounds = rect_path(shadow.bounds)?;
    surface.push_clip_path(&bounds, &FillRule::NonZero);
    match &shadow.shape {
        ShadowShape::Sharp(contours) => {
            let path = contour_path(contours)?;
            surface.push_clip_path(&clip, &FillRule::EvenOdd);
            surface.set_fill(Some(Fill {
                rule: FillRule::EvenOdd,
                ..color
            }));
            surface.draw_path(&path);
        }
        ShadowShape::Blurred(mask) => {
            let area = rect_path(mask.rect)?;
            let image =
                Image::from_custom(MaskPixels(mask.clone()), true).map_err(PdfError::Image)?;
            let unit = Size::from_wh(1.0, 1.0).ok_or(PdfError::InvalidGeometry)?;
            let mut builder = surface.stream_builder();
            let mut mask_surface = builder.surface();
            mask_surface.push_transform(&Transform::from_row(
                mask.rect.width,
                0.0,
                0.0,
                mask.rect.height,
                mask.rect.x,
                mask.rect.y,
            ));
            mask_surface.draw_image(image, unit);
            mask_surface.pop();
            mask_surface.finish();
            let stream = builder.finish();
            surface.push_clip_path(&clip, &FillRule::EvenOdd);
            surface.push_mask(Mask::new(stream, MaskType::Luminosity));
            surface.set_fill(Some(color));
            surface.draw_path(&area);
            surface.pop();
        }
    }
    surface.pop();
    surface.pop();
    Ok(())
}
