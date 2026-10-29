use crate::document::BlockBox;
use crate::graphics::{BoxPaint, ImagePaint, Rect};
use crate::layout::{
    LayoutError,
    geometry::{HorizontalBox, SizeConstraints},
    margin::MarginStrut,
    measure::{FlowItem, MeasuredBlock, Measurer},
    ratio::PreferredRatio,
};
use crate::style::POINTS_PER_PIXEL;
use crate::text::IntrinsicWidths;

/// The replaced content's natural size in points, scaled by its selected source's pixel
/// density (HTML §4.8.4.3.7); a ratio-only image takes `available_width`.
fn density_corrected(block: &BlockBox, available_width: f32) -> [f32; 2] {
    let scale = POINTS_PER_PIXEL / block.image_density();
    block
        .image()
        .unwrap()
        .pixel_size(available_width / scale)
        .map(|value| value * scale)
}

/// A replaced element's natural size in points and its preferred aspect ratio (CSS Sizing 4
/// §4.1). Size containment treats the natural size as zero, or as `contain-intrinsic-size`,
/// and drops the natural ratio
/// (CSS Containment 2 §3.1); only an `aspect-ratio` `<ratio>` then relates the axes.
struct Natural {
    size: [f32; 2],
    ratio: Option<PreferredRatio>,
}
impl Natural {
    fn of(block: &BlockBox, available_width: f32, edges: [f32; 2]) -> Self {
        let image = block.image().unwrap();
        let style = block.style();
        let contained = style.containment().size;
        let mut size = density_corrected(block, available_width);
        for (axis, contained) in contained.into_iter().enumerate() {
            if contained {
                // CSS Sizing 4 §5.2: the explicit intrinsic inner size, if any.
                size[axis] = style.containment.intrinsic[axis].explicit().unwrap_or(0.0);
            }
        }
        let natural = (!contained.contains(&true))
            .then(|| image.natural_size().ratio)
            .flatten()
            .filter(|ratio| ratio.is_finite() && *ratio > 0.0);
        Self {
            size,
            ratio: PreferredRatio::of(style, natural, edges),
        }
    }
    fn height(&self, width: f32) -> f32 {
        self.ratio.map_or(self.size[1], |ratio| ratio.height(width))
    }
    fn width(&self, height: f32) -> f32 {
        self.ratio.map_or(self.size[0], |ratio| ratio.width(height))
    }
}

fn edges(block: &BlockBox, containing_width: f32) -> crate::layout::geometry::Edges {
    crate::layout::geometry::Edges::of(block.style(), containing_width)
}

/// Content-box width and height constraints, percentages resolved against the containing block.
pub(super) fn constraints(
    block: &BlockBox,
    containing_width: f32,
    containing_height: Option<f32>,
) -> [SizeConstraints; 2] {
    let style = block.style();
    let edges = edges(block, containing_width);
    let [horizontal, vertical] = crate::layout::ratio::axis_edges(&edges);
    let [widths, heights] = [
        SizeConstraints::new(
            [&style.width, &style.min_width, &style.max_width],
            Some(containing_width),
            style.box_sizing,
            horizontal,
        ),
        SizeConstraints::new(
            [&style.height, &style.min_height, &style.max_height],
            containing_height,
            style.box_sizing,
            vertical,
        ),
    ];
    let natural = Natural::of(block, containing_width, [horizontal, vertical]);
    let width = heights
        .specified
        .map_or(natural.size[0], |height| natural.width(height));
    let height = widths.specified.map_or_else(
        || natural.height(natural.size[0]),
        |width| natural.height(width),
    );
    [
        widths.with_intrinsic(
            [&style.width, &style.min_width, &style.max_width],
            IntrinsicWidths {
                min: width,
                max: width,
            },
            Some(containing_width),
        ),
        heights.with_intrinsic(
            [&style.height, &style.min_height, &style.max_height],
            IntrinsicWidths {
                min: height,
                max: height,
            },
            containing_height,
        ),
    ]
}

/// Used content size; an image with only an aspect ratio stretches into `stretch_width`.
/// With both sizes automatic, the natural width and the height it gives through the ratio
/// meet the minimum and maximum of each axis in turn, each limit transferring through the
/// ratio (CSS 2 §10.4, CSS Sizing 4 §4.4); a size the other axis then violates is clamped.
pub(super) fn size(
    block: &BlockBox,
    containing_width: f32,
    stretch_width: f32,
    containing_height: Option<f32>,
    used_width: Option<f32>,
    used_height: Option<f32>,
) -> [f32; 2] {
    let edges = edges(block, containing_width);
    let natural = Natural::of(
        block,
        stretch_width,
        crate::layout::ratio::axis_edges(&edges),
    );
    let [width, height] = constraints(block, containing_width, containing_height);
    let specified_width = used_width.or(width.specified);
    let specified_height = used_height.or(height.specified);
    let constrain_width = |v| used_width.unwrap_or_else(|| width.constrain(v));
    let constrain_height = |v| used_height.unwrap_or_else(|| height.constrain(v));
    match (specified_width, specified_height) {
        (Some(w), Some(h)) => [constrain_width(w), constrain_height(h)],
        (Some(w), None) => {
            let w = constrain_width(w);
            [w, constrain_height(natural.height(w))]
        }
        (None, Some(h)) => {
            let h = constrain_height(h);
            [constrain_width(natural.width(h)), h]
        }
        (None, None) => {
            let mut w = natural.size[0];
            let mut h = natural.height(w);
            if natural.ratio.is_some() {
                let limited = width.constrain(w);
                if limited != w {
                    w = limited;
                    h = natural.height(w);
                }
                let limited = height.constrain(h);
                if limited != h {
                    h = limited;
                    w = natural.width(h);
                }
            }
            [width.constrain(w), height.constrain(h)]
        }
    }
}

/// Min- and max-content widths (CSS Sizing 3 §5.1): with only an aspect ratio the minimum is
/// zero and the maximum stretches into `available`, the containing block width.
pub(super) fn intrinsic(block: &BlockBox, available: f32) -> IntrinsicWidths {
    let max = size(block, 0.0, available, None, None, None)[0];
    let min = if block.image().unwrap().natural_size().ratio_only()
        && !block.style().containment().size[0]
    {
        constraints(block, 0.0, None)[0].constrain(0.0)
    } else {
        max
    };
    IntrinsicWidths { min, max }
}

pub(super) fn measure<'a>(
    measurer: &Measurer<'_>,
    block: &'a BlockBox,
    geometry: &HorizontalBox,
    containing_height: Option<f32>,
    used_height: Option<f32>,
) -> Result<MeasuredBlock<'a>, LayoutError> {
    let style = block.style();
    let [width, height] = size(
        block,
        geometry.containing_width,
        geometry.containing_width,
        containing_height,
        Some(geometry.width),
        used_height,
    );
    let top = geometry.padding[0] + geometry.border[0];
    let bottom = geometry.padding[2] + geometry.border[2];
    let bounds = Rect {
        x: geometry.x,
        y: top,
        width,
        height,
    };
    let image = match block.image().unwrap().as_ref() {
        crate::images::Image::Frame(frame, color_scheme) => crate::layout::viewport::view(
            frame,
            [width, height],
            *color_scheme,
            measurer.text.resolver(),
            &measurer.work,
        )?,
        crate::images::Image::Widget(widget) => {
            widget.render([width, height].map(|side| side / POINTS_PER_PIXEL))?
        }
        _ => block.image().unwrap().clone(),
    };
    // A nested navigable's viewport and other content without natural dimensions fill the
    // content box (CSS Images 3 §5.5), as do widgets, which `object-fit` does not apply to
    // (CSS UI 4 §7.2).
    let widget = matches!(
        block.image().unwrap().as_ref(),
        crate::images::Image::Widget(_)
    );
    let [paint_width, paint_height] = if widget || image.natural_size() == Default::default() {
        [width, height]
    } else {
        style
            .object_fit
            .size(density_corrected(block, width), [width, height])
    };
    let border_box = BoxPaint {
        padding: geometry.padding,
        ..BoxPaint::whole(
            Rect {
                x: bounds.x - geometry.padding[3] - geometry.border[3],
                y: 0.0,
                width: width
                    + geometry.padding[1]
                    + geometry.padding[3]
                    + geometry.border[1]
                    + geometry.border[3],
                height: top + height + bottom,
            },
            block.decoration().clone(),
        )
    };
    // Images 3 §4.5 clips every fitted object to the content edge.
    let clip = Some(border_box.inner_clip(geometry.padding, [true; 2]));
    let paint = ImagePaint {
        image,
        clip,
        rect: Rect {
            x: bounds.x + style.object_position[0].resolve(width - paint_width),
            y: top + style.object_position[1].resolve(height - paint_height),
            width: paint_width,
            height: paint_height,
        },
    };
    let height = top + height + bottom;
    if !height.is_finite() {
        return Err(LayoutError::NonFiniteBlockGeometry {
            block: measurer.sources.block(block),
        });
    }
    let edges = measurer.fragmentainer.edges(style);
    Ok(MeasuredBlock {
        grid: None,
        clearance_before: None,
        clearance_target: measurer
            .clearance_targets
            .get(&std::ptr::from_ref::<BlockBox>(block))
            .copied(),
        source: block,
        geometry: geometry.clone(),
        containing_height,
        style,
        canvas_source: block.is_canvas_source(),
        offset: style.relative_offset(
            geometry.containing_width,
            containing_height,
            geometry.containing_rtl,
        ),
        before: MarginStrut::edge(block, 0, geometry.containing_width),
        after: MarginStrut::edge(block, 2, geometry.containing_width),
        through: false,
        height,
        sizing: None,
        constrained: false,
        independent: false,
        overflow_height: height,
        first_fragment: height,
        has_forced: edges.iter().any(|edge| edge.is_forced()),
        break_before: edges[0],
        break_after: edges[1],
        // CSS Containment 2 §4: skipped contents include a replaced element's content.
        items: vec![FlowItem::Image {
            paint,
            height,
            visible: style.visibility.is_visible() && !style.skips_contents(),
        }],
        paint: measurer.box_paint(block, geometry),
        navigation: block.navigation(),
        end_space: 0.0,
        start_space: 0.0,
        repeat_space: 0.0,
        marker: None,
        table: None,
        continues: false,
        spanner_fragment: None,
    })
}
