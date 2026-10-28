use crate::document::{DocumentError, Paragraph};
use crate::style::Style;

pub(super) fn marker(
    style: &Style,
    ordinal: i128,
    resources: &mut crate::document::formatting::ImageResources<'_>,
) -> Result<Option<Paragraph>, DocumentError> {
    let mut image = style.list_style_image.with_image_type();
    if !matches!(image, crate::graphics::BackgroundImage::None)
        && image
            .load(&mut |reference| {
                resources
                    .cache
                    .load_marker(
                        reference,
                        resources.base,
                        resources.resolver,
                        style.element_font_namespace(),
                    )
                    .ok_or(())
            })
            .is_ok()
    {
        let mut marker_style = style.clone();
        marker_style.text_transform = Default::default();
        let mut paragraph = Paragraph::marker(" ".into(), &marker_style)?;
        paragraph.marker_image = Some(image);
        return Ok(Some(paragraph));
    }
    let Some(text) = style.list_style_type.marker(ordinal, style.rtl)? else {
        return Ok(None);
    };
    Ok(Some(Paragraph::marker(text, style)?))
}
