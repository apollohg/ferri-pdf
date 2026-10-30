//! Nested navigables' documents laid out as continuous media in their viewports (HTML §15.4.1).
use crate::document::{Frame, FrameDocument};
use crate::fonts::FontBook;
use crate::graphics::{DisplayItem, ImagePaint, Rect};
use crate::images::Image;
use crate::layout::{LayoutError, LayoutTrace, TextLine, work::Budget};
use crate::resources::ResourceResolver;
use std::sync::Arc;

/// What a nested navigable's viewport shows: its document's only page.
pub struct ViewportPaint {
    pub(crate) items: Vec<DisplayItem>,
    pub(crate) lines: Vec<TextLine>,
    pub(crate) fonts: FontBook,
    /// The SVG filters and masks the nested document's `url()` references name.
    pub(crate) definitions: crate::document::svg::SvgDefinitions,
    /// The page area, which is the whole viewport.
    pub(crate) area: Rect,
    /// The renderings of the elements that the document's `element()` images show.
    pub(crate) elements: Box<crate::layout::ElementRenderings>,
}
impl std::fmt::Debug for ViewportPaint {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ViewportPaint")
            .field("items", &self.items.len())
            .field("lines", &self.lines.len())
            .field("area", &self.area)
            .finish_non_exhaustive()
    }
}

/// The content `frame` shows in a `size` viewport, in points. An empty viewport shows nothing.
pub(super) fn view(
    frame: &Frame,
    size: [f32; 2],
    color_scheme: crate::style::ColorScheme,
    resolver: &dyn ResourceResolver,
    budget: &Budget,
) -> Result<Arc<Image>, LayoutError> {
    if size.iter().any(|side| *side <= 0.0) {
        return Ok(Arc::new(Image::Transparent));
    }
    if let Some(view) = frame.cached(size) {
        return Ok(view);
    }
    let area = Rect {
        x: 0.0,
        y: 0.0,
        width: size[0],
        height: size[1],
    };
    let paint = match frame.document(size, color_scheme, resolver)? {
        FrameDocument::Html(mut document) => {
            let trace = crate::layout::stabilize(&mut document, resolver, budget, Vec::new())?;
            let LayoutTrace {
                page_area,
                pages,
                display_lists,
                element_images,
                ..
            } = trace;
            let fonts = document.styles().fonts.clone();
            let definitions = document.svg_definitions();
            frame.record_diagnostics(document.into_diagnostics());
            ViewportPaint {
                items: display_lists.into_iter().next().unwrap_or_default(),
                lines: pages.into_iter().next().unwrap_or_default(),
                fonts,
                definitions,
                area: page_area,
                elements: element_images,
            }
        }
        FrameDocument::Svg(image) => svg_document(image, area),
    };
    let view = Arc::new(Image::Viewport(Arc::new(paint)));
    frame.store(size, view.clone());
    Ok(view)
}

/// An SVG document's root element, whose `auto` width and height fill the viewport
/// (SVG 2 §8.2).
fn svg_document(image: Arc<Image>, area: Rect) -> ViewportPaint {
    let natural = image.natural_size();
    let points = |pixels: f32| pixels * crate::style::POINTS_PER_PIXEL;
    let rect = Rect {
        width: natural.width.map_or(area.width, points),
        height: natural.height.map_or(area.height, points),
        ..area
    };
    ViewportPaint {
        items: vec![DisplayItem::Image(ImagePaint {
            image,
            rect,
            clip: None,
            source_scale: crate::paint::INITIAL_IMAGE_SCALE,
        })],
        lines: Vec::new(),
        fonts: FontBook::default(),
        definitions: crate::document::svg::SvgDefinitions::default(),
        area,
        elements: Box::default(),
    }
}
