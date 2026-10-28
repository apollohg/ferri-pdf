use crate::{
    LayoutError,
    geometry::{HorizontalBox, SizeConstraints},
    margin::MarginStrut,
    measure::{FlowItem, MeasuredBlock, Measurer},
};
use ferri_pdf_document::BlockBox;
use ferri_pdf_graphics::{BoxPaint, ImagePaint, Rect};
use ferri_pdf_style::POINTS_PER_PIXEL;
use ferri_pdf_text::IntrinsicWidths;

/// Content-box width and height constraints, percentages resolved against the containing block.
pub(super) fn constraints(
    block: &BlockBox,
    containing_width: f32,
    containing_height: Option<f32>,
) -> [SizeConstraints; 2] {
    let style = block.style();
    let padding = style
        .padding
        .each_ref()
        .map(|p| p.resolve(containing_width));
    let border = style
        .decoration
        .borders
        .map(ferri_pdf_graphics::Border::used_width);
    let mut constraints = [
        SizeConstraints::new(
            [&style.width, &style.min_width, &style.max_width],
            Some(containing_width),
            style.box_sizing,
            padding[1] + padding[3] + border[1] + border[3],
        ),
        SizeConstraints::new(
            [&style.height, &style.min_height, &style.max_height],
            containing_height,
            style.box_sizing,
            padding[0] + padding[2] + border[0] + border[2],
        ),
    ];
    let natural = block
        .image()
        .unwrap()
        .pixel_size(containing_width / POINTS_PER_PIXEL)
        .map(|value| value * POINTS_PER_PIXEL);
    let width = constraints[1]
        .specified
        .map_or(natural[0], |height| height * natural[0] / natural[1]);
    let height = constraints[0]
        .specified
        .map_or(natural[1], |width| width * natural[1] / natural[0]);
    let [widths, heights] = constraints;
    constraints = [
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
    ];
    constraints
}

/// Used content size; an image with only an aspect ratio stretches into `stretch_width`.
pub(super) fn size(
    block: &BlockBox,
    containing_width: f32,
    stretch_width: f32,
    containing_height: Option<f32>,
    used_width: Option<f32>,
    used_height: Option<f32>,
) -> [f32; 2] {
    let intrinsic = block
        .image()
        .unwrap()
        .pixel_size(stretch_width / POINTS_PER_PIXEL)
        .map(|v| v * POINTS_PER_PIXEL);
    let [width, height] = constraints(block, containing_width, containing_height);
    let specified_width = used_width.or(width.specified);
    let specified_height = used_height.or(height.specified);
    let constrain_width = |v| used_width.unwrap_or_else(|| width.constrain(v));
    let constrain_height = |v| used_height.unwrap_or_else(|| height.constrain(v));
    match (specified_width, specified_height) {
        (Some(w), Some(h)) => [constrain_width(w), constrain_height(h)],
        (Some(w), None) => {
            let w = constrain_width(w);
            [w, constrain_height(w * intrinsic[1] / intrinsic[0])]
        }
        (None, Some(h)) => {
            let h = constrain_height(h);
            [constrain_width(h * intrinsic[0] / intrinsic[1]), h]
        }
        (None, None) => {
            let minimum = (width.minimum / intrinsic[0]).max(height.minimum / intrinsic[1]);
            let maximum = (width.maximum.unwrap_or(f32::INFINITY).max(width.minimum)
                / intrinsic[0])
                .min(height.maximum.unwrap_or(f32::INFINITY).max(height.minimum) / intrinsic[1]);
            let scale = if minimum <= maximum {
                1.0f32.clamp(minimum, maximum)
            } else {
                1.0
            };
            [
                width.constrain(intrinsic[0] * scale),
                height.constrain(intrinsic[1] * scale),
            ]
        }
    }
}

/// Min- and max-content widths (CSS Sizing 3 §5.1): with only an aspect ratio the minimum is
/// zero and the maximum stretches into `available`, the containing block width.
pub(super) fn intrinsic(block: &BlockBox, available: f32) -> IntrinsicWidths {
    let max = size(block, 0.0, available, None, None, None)[0];
    let min = if block.image().unwrap().natural_size().ratio_only() {
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
    let intrinsic = block
        .image()
        .unwrap()
        .pixel_size(width / POINTS_PER_PIXEL)
        .map(|v| v * POINTS_PER_PIXEL);
    let [paint_width, paint_height] = style.object_fit.size(intrinsic, [width, height]);
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
        image: block.image().unwrap().clone(),
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
        items: vec![FlowItem::Image {
            paint,
            height,
            visible: style.visibility.is_visible(),
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
