use crate::document::PageBox;
use crate::graphics::{BackgroundGeometry, BoxAreas, BoxPaint, Clip, DisplayItem, Rect};
use std::sync::Arc;

pub(super) struct PagePaint {
    pub canvas: BoxPaint,
    background: Option<BoxPaint>,
    border: Option<BoxPaint>,
}
/// The page box inside the page margins.
fn page_box(page: &PageBox) -> BoxPaint {
    let margin = page.geometry.margin_box();
    let [top, right, bottom, left] = page.geometry.margin;
    let border_rect = Rect {
        x: left,
        y: top,
        width: margin.width - left - right,
        height: margin.height - top - bottom,
    };
    BoxPaint {
        padding: page.geometry.padding,
        ..BoxPaint::whole(border_rect, page.decoration.clone())
    }
}

/// The page box's outline, which paints over the page's content and margin boxes
/// (CSS Page 3 Appendix A applies the outline properties to the page context).
pub(super) fn page_outline(page: &PageBox) -> Option<DisplayItem> {
    page_box(page).outline().map(DisplayItem::Box)
}

impl PagePaint {
    pub(crate) fn new(
        page: &PageBox,
        canvas_background: &crate::graphics::BoxDecoration,
        bleed: Rect,
    ) -> Self {
        let margin = page.geometry.margin_box();
        let mut base = page_box(page);
        // `page_outline` paints the outline over the page's content.
        base.decoration.outline = crate::graphics::Outline::default();
        let border_rect = base.rect;
        let clips = base.background_clips();
        let canvas = BoxPaint {
            background_geometry: Some(Arc::new(BackgroundGeometry {
                origins: None,
                clips,
                viewport: None,
            })),
            ..BoxPaint::whole(border_rect, canvas_background.clone())
        };
        let mut background = base.clone();
        let origins = background.background_origins();
        background.rect = bleed;
        for border in &mut background.decoration.borders {
            border.color.alpha = 0.0;
        }
        background.decoration.border_image = Arc::default();
        background.decoration.shadows = Arc::default();
        let clip = Clip {
            rect: bleed,
            radii: [[0.0; 2]; 4],
            outline: None,
            reference_box: None,
            axes: [true; 2],
        };
        background.background_geometry = Some(Arc::new(BackgroundGeometry {
            origins: Some(origins),
            clips: BoxAreas {
                border: clip.clone(),
                padding: clip.clone(),
                content: clip,
            },
            viewport: Some(margin),
        }));
        let mut border = base;
        border.decoration.clear_background();
        border.draw_background = false;
        Self {
            canvas,
            background: background.has_ink().then_some(background),
            border: border.has_ink().then_some(border),
        }
    }
    pub(crate) fn in_flow(
        page: &PageBox,
        canvas: &crate::graphics::BoxDecoration,
        flow: crate::style::logical::Flow,
    ) -> Result<Self, crate::layout::LayoutError> {
        let record = crate::layout::PageRecord::new(page)?;
        let frame = crate::layout::writing::Frame::physical(flow, page.geometry.paper).inverse();
        let mut paint = Self::new(page, canvas, record.output.bleed);
        frame.paint(&mut paint.canvas);
        Ok(paint)
    }

    /// Puts the page box's background and border under the page's display list, and
    /// `underlay` between the background and the document canvas (CSS Page 3 §3.1);
    /// text-clipped backgrounds paint within `text_area`, the page's text.
    pub(crate) fn decorate(
        &self,
        list: &mut Vec<DisplayItem>,
        text_area: Option<crate::graphics::FlowPaint>,
        underlay: Vec<DisplayItem>,
    ) {
        if let Some(border) = &self.border {
            list.insert(
                usize::from(self.canvas.has_ink()),
                DisplayItem::Box(border.clone()),
            );
        }
        list.splice(0..0, underlay);
        if let Some(background) = &self.background {
            list.insert(
                0,
                DisplayItem::Box(BoxPaint {
                    text_area: text_area.map(Arc::new),
                    ..background.clone()
                }),
            );
        }
    }
}
