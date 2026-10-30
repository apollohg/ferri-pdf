use crate::document::Paragraph;
use crate::layout::TextLine;
use crate::layout::measure::Failure;
use crate::layout::measure::Measurer;

#[derive(Clone)]
pub(super) struct MeasuredMarker {
    pub owner: Option<crate::style::PageStyleTarget>,
    pub frame: crate::layout::writing::Frame,
    pub fragment: TextLine,
    pub image: Option<crate::graphics::BoxPaint>,
    /// The anonymous replaced boxes of `content` images, from the line's left edge and
    /// baseline.
    pub images: Vec<crate::graphics::ImagePaint>,
    pub ascent: f32,
    pub ink_bounds: Option<[f32; 2]>,
    pub decorations: Vec<crate::graphics::DecorationPaint>,
}

pub(super) fn measure(
    paragraph: &Paragraph,
    measurer: &Measurer<'_>,
    index: usize,
    left: f32,
    right: f32,
    owner: Option<crate::style::PageStyleTarget>,
) -> Result<MeasuredMarker, Failure> {
    let (sizes, metrics) = content_image_atomics(paragraph);
    let mut line = measurer
        .text_lines(paragraph, f32::MAX, &metrics)?
        .into_iter()
        .next()
        .expect("a generated marker contains nonempty, unwrapped text");
    let style = paragraph.style();
    let image = paragraph.marker_image().map(|image| {
        let [width, height] = paragraph
            .marker_image_size()
            .expect("image marker has size");
        let mut decoration = crate::graphics::BoxDecoration::default();
        std::sync::Arc::make_mut(&mut decoration.backgrounds).images = vec![image.clone()];
        let x = if style.used_rtl() { line.width } else { 0.0 };
        if !style.used_rtl() {
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
        crate::graphics::BoxPaint::whole(
            crate::graphics::Rect {
                x,
                y: -height,
                width,
                height,
            },
            decoration,
        )
    });
    let images = content_image_paints(paragraph, &line, &sizes);
    for image in &images {
        let [top, bottom] = [image.rect.y, image.rect.y + image.rect.height];
        line.ink_bounds = Some(line.ink_bounds.map_or([top, bottom], |bounds| {
            [bounds[0].min(top), bounds[1].max(bottom)]
        }));
    }
    Ok(MeasuredMarker {
        owner,
        frame: crate::layout::writing::Frame::for_style(style),
        image,
        images,
        ascent: line.ascent,
        ink_bounds: line.ink_bounds,
        decorations: line.decorations,
        fragment: TextLine {
            paragraph: index,
            x: if style.used_rtl() {
                right
            } else {
                left - line.width
            },
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

/// The sizes and line metrics of a generated paragraph's atomic inlines, which are the
/// anonymous replaced boxes of its `content` images (CSS Content 3 §2.2): sized as replaced
/// elements with automatic sizes, their baseline is their bottom edge.
pub(super) fn content_image_atomics(
    paragraph: &Paragraph,
) -> (Vec<[f32; 2]>, Vec<crate::text::AtomicMetrics>) {
    let sizes: Vec<[f32; 2]> = paragraph
        .atomics()
        .iter()
        .map(|atomic| crate::layout::images::size(atomic.block(), 0.0, 0.0, None, None, None))
        .collect();
    let metrics = sizes
        .iter()
        .map(|&[width, height]| crate::text::AtomicMetrics {
            width,
            height,
            baseline: height,
            synthesized_baseline: false,
        })
        .collect();
    (sizes, metrics)
}

/// The `content` images on `line` of a generated paragraph, sized by
/// [`content_image_atomics`], from the line's left edge and baseline.
pub(super) fn content_image_paints(
    paragraph: &Paragraph,
    line: &crate::text::Line,
    sizes: &[[f32; 2]],
) -> Vec<crate::graphics::ImagePaint> {
    line.atomics
        .iter()
        .map(|fragment| {
            let atomic = &paragraph.atomics()[fragment.index];
            let [width, height] = sizes[fragment.index];
            crate::graphics::ImagePaint {
                image: atomic
                    .block()
                    .image()
                    .expect("generated atomics are content images")
                    .clone(),
                rect: crate::graphics::Rect {
                    x: fragment.x,
                    y: fragment.baseline - height,
                    width,
                    height,
                },
                clip: None,
                source_scale: atomic.block().style().effective_zoom,
                rendering: atomic.block().style().image_rendering(),
            }
        })
        .collect()
}
