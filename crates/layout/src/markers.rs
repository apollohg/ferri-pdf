use crate::measure::Measurer;
use crate::{LayoutError, TextLine};
use ferri_pdf_document::Paragraph;

#[derive(Clone)]
pub(super) struct MeasuredMarker {
    pub fragment: TextLine,
    pub image: Option<ferri_pdf_graphics::BoxPaint>,
    pub ascent: f32,
    pub ink_bounds: Option<[f32; 2]>,
    pub decorations: Vec<ferri_pdf_graphics::DecorationPaint>,
}

pub(super) fn measure(
    paragraph: &Paragraph,
    measurer: &Measurer<'_>,
    index: usize,
    left: f32,
    right: f32,
) -> Result<MeasuredMarker, LayoutError> {
    let mut line = measurer
        .text_lines(paragraph, f32::MAX, &[])?
        .into_iter()
        .next()
        .expect("a generated marker contains nonempty, unwrapped text");
    let style = paragraph.style();
    let image = paragraph.marker_image().map(|image| {
        let [width, height] = paragraph
            .marker_image_size()
            .expect("image marker has size");
        let mut decoration = ferri_pdf_graphics::BoxDecoration::default();
        std::sync::Arc::make_mut(&mut decoration.backgrounds).images = vec![image.clone()];
        let x = if style.rtl { line.width } else { 0.0 };
        if !style.rtl {
            for run in &mut line.runs {
                run.x += width;
            }
        }
        line.width += width;
        line.ascent = line.ascent.max(height);
        line.height = line.height.max(height);
        line.ink_bounds = Some([
            line.ink_bounds
                .map_or(-height, |bounds| bounds[0].min(-height)),
            line.ink_bounds.map_or(0.0, |bounds| bounds[1]),
        ]);
        ferri_pdf_graphics::BoxPaint::whole(
            ferri_pdf_graphics::Rect {
                x,
                y: -height,
                width,
                height,
            },
            decoration,
        )
    });
    Ok(MeasuredMarker {
        image,
        ascent: line.ascent,
        ink_bounds: line.ink_bounds,
        decorations: line.decorations,
        fragment: TextLine {
            paragraph: index,
            x: if style.rtl { right } else { left - line.width },
            baseline: line.ascent,
            text: line.text,
            width: line.width,
            height: line.height,
            runs: line.runs,
            font_size: style.size,
            color: style.color,
        },
    })
}
