use crate::document::{DocumentError, Paragraph};
use crate::style::{Display, Style};

/// The style a `::marker` box lays out its contents with (CSS Lists 3 §3.1.1): of its own
/// properties only `content`, `direction`, `unicode-bidi` and `text-combine-upright` apply,
/// while the inherited properties it computes take effect on its text.
pub(super) fn used_style(marker: &Style) -> Style {
    let mut used = marker.anonymous(Display::Inline);
    // Inheritance copies `currentcolor` text paint unresolved; the marker resolved its own.
    used.text_paint.clone_from(&marker.text_paint);
    used.text_decorations
        .propagated
        .clone_from(&marker.text_decorations.propagated);
    used.unicode_bidi = marker.unicode_bidi;
    // `initial-letter` applies to inside markers, and an outside marker takes no inline place
    // to start (CSS Inline 3 §7.3).
    used.initial_letter = marker.initial_letter;
    used
}

/// The marker contents list item `item` generates when its `::marker` has `content: normal`
/// (CSS Lists 3 §3.2): its marker image followed by a space, else its marker string.
pub(super) fn marker(
    item: &Style,
    style: &Style,
    ordinal: i128,
    resources: &mut crate::document::formatting::ImageResources<'_>,
) -> Result<Option<Paragraph>, DocumentError> {
    // CSS Images 4 §2.1: an invalid image, or one that cannot be loaded, is treated as `none`.
    let mut image = item.list_style_image.with_image_type();
    if image.is_displayable() {
        let Ok(()) = image.load(&mut |request| {
            Ok::<_, std::convert::Infallible>(resources.load_decoded(request, item))
        });
        if image.is_displayable() {
            return Ok(Some(Paragraph::image_marker(image, style)?));
        }
    }
    let Some(text) = item.list_style_type.marker(ordinal, style.rtl)? else {
        return Ok(None);
    };
    Paragraph::marker(&text, style)
}
