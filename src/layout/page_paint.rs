use crate::document::PageBox;
use crate::graphics::{BackgroundGeometry, BoxAreas, BoxPaint, Clip, DisplayItem, Rect};
use std::sync::Arc;

pub(super) struct PagePaint {
    pub canvas: BoxPaint,
    background: Option<BoxPaint>,
    border: Option<BoxPaint>,
}
impl PagePaint {
    pub(crate) fn new(
        page: &PageBox,
        canvas_background: &crate::graphics::BoxDecoration,
        bleed: Rect,
    ) -> Self {
        let margin = page.geometry.margin_box();
        let [top, right, bottom, left] = page.geometry.margin;
        let border_rect = Rect {
            x: left,
            y: top,
            width: margin.width - left - right,
            height: margin.height - top - bottom,
        };
        let base = BoxPaint {
            padding: page.geometry.padding,
            ..BoxPaint::whole(border_rect, page.decoration.clone())
        };
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
            reference_box: None,
            axes: [true; 2],
        };
        background.background_geometry = Some(Arc::new(BackgroundGeometry {
            origins: Some(origins),
            clips: BoxAreas {
                border: clip,
                padding: clip,
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
    pub(crate) fn decorate(&self, lists: &mut [Vec<DisplayItem>]) {
        for list in lists {
            if let Some(border) = &self.border {
                list.insert(
                    usize::from(self.canvas.has_ink()),
                    DisplayItem::Box(border.clone()),
                );
            }
            if let Some(background) = &self.background {
                list.insert(0, DisplayItem::Box(background.clone()));
            }
        }
    }
}
