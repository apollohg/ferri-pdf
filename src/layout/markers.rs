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
    // A marker's atomic inlines are the anonymous replaced boxes of its content images, sized
    // as replaced elements with automatic sizes; their baseline is their bottom edge.
    let sizes: Vec<[f32; 2]> = paragraph
        .atomics()
        .iter()
        .map(|atomic| crate::layout::images::size(&atomic.block, 0.0, 0.0, None, None, None))
        .collect();
    let metrics: Vec<_> = sizes
        .iter()
        .map(|&[width, height]| crate::text::AtomicMetrics {
            width,
            height,
            baseline: height,
            synthesized_baseline: false,
        })
        .collect();
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
    let images = line
        .atomics
        .iter()
        .map(|fragment| {
            let atomic = &paragraph.atomics()[fragment.index];
            let [width, height] = sizes[fragment.index];
            let top = fragment.baseline - height;
            line.ink_bounds = Some(line.ink_bounds.map_or([top, fragment.baseline], |bounds| {
                [bounds[0].min(top), bounds[1].max(fragment.baseline)]
            }));
            crate::graphics::ImagePaint {
                image: atomic
                    .block
                    .image()
                    .expect("marker atomics are content images")
                    .clone(),
                rect: crate::graphics::Rect {
                    x: fragment.x,
                    y: top,
                    width,
                    height,
                },
                clip: None,
                source_scale: atomic.block.style().effective_zoom,
            }
        })
        .collect();
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
