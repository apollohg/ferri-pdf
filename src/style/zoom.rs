use crate::paint::{self, BackgroundImage, BasicShape, LengthPercentage};
use crate::style::grid_values::{TrackBreadth, TrackSize};
use crate::style::{LineHeight, Style, TabSize};
use std::sync::Arc;

pub(crate) const INITIAL_ZOOM: f32 = 1.0;

pub(crate) fn accumulate_zoom(parent: f32, own: f32) -> Result<f32, crate::style::StyleError> {
    let effective = parent * own;
    if !effective.is_finite() || effective <= 0.0 {
        return Err("non-finite or underflowed effective zoom".into());
    }
    Ok(effective)
}

impl Style {
    pub(crate) fn canonical(&self) -> &Self {
        self.canonical.as_deref().unwrap_or(self)
    }

    pub(crate) fn used(&self) -> Self {
        if self.canonical.is_some() {
            return self.clone();
        }
        let factor = self.effective_zoom;
        if factor == INITIAL_ZOOM {
            return self.clone();
        }
        let mut style = self.clone();
        style.canonical = Some(Arc::new(self.clone()));
        style.size *= factor;
        if let LineHeight::Length(height) = &mut style.line_height {
            *height *= factor;
        }
        style.line_height_step *= factor;
        style.letter_spacing *= factor;
        style.word_spacing = style.word_spacing.zoomed(factor);
        if let TabSize::Length(width) = &mut style.tab_size {
            *width *= factor;
        }
        for value in style.margin.iter_mut().chain(&mut style.inset).chain([
            &mut style.width,
            &mut style.min_width,
            &mut style.max_width,
            &mut style.height,
            &mut style.min_height,
            &mut style.max_height,
        ]) {
            *value = value.zoomed(factor);
        }
        for value in &mut style.padding {
            *value = value.zoomed(factor);
        }
        for border in &mut style.decoration.borders {
            border.width *= factor;
        }
        if style.decoration.outline.style == paint::OutlineStyle::Auto {
            style.decoration.outline.width = style.decoration.outline.used_width();
            style.decoration.outline.style = paint::OutlineStyle::Line(paint::BorderStyle::Solid);
        }
        style.decoration.outline.width *= factor;
        style.decoration.outline.offset *= factor;
        for corner in Arc::make_mut(&mut style.decoration.radii) {
            for radius in corner {
                *radius = radius.zoomed(factor);
            }
        }
        for spacing in &mut style.table.spacing {
            *spacing *= factor;
        }
        for gap in &mut style.flex.gap {
            if let crate::style::Gap::Length(value) = gap {
                *value = value.zoomed(factor);
            }
        }
        if let crate::style::FlexBasis::Size(value) = &mut style.flex.basis {
            *value = value.zoomed(factor);
        }
        for template in &mut style.grid.templates {
            let template = Arc::make_mut(template);
            for track in &mut template.tracks {
                zoom_track(track, factor);
            }
            if let Some(repeat) = &mut template.repeat {
                for track in &mut repeat.tracks {
                    zoom_track(track, factor);
                }
            }
        }
        for tracks in &mut style.grid.auto_tracks {
            for track in Arc::make_mut(tracks) {
                zoom_track(track, factor);
            }
        }
        if let Some(width) = &mut style.multicol.width {
            *width *= factor;
        }
        style.multicol.rule.width *= factor;
        style.text_indent.amount = style.text_indent.amount.zoomed(factor);
        style.shape_margin = style.shape_margin.zoomed(factor);
        match Arc::make_mut(&mut style.shape_outside) {
            paint::ShapeOutside::Shape { shape, .. } => zoom_shape(shape, factor),
            paint::ShapeOutside::Image(image) => zoom_image(image, factor),
            _ => {}
        }
        if let Some(edges) = &mut style.clip {
            for edge in edges.iter_mut().flatten() {
                *edge *= factor;
            }
        }
        for edge in &mut style.overflow_clip_margin {
            edge.length *= factor;
        }
        for value in &mut style.transform {
            value.zoom(factor);
        }
        for value in &mut style.transform_origin {
            *value = value.zoomed(factor);
        }
        if let Some(offsets) = &mut style.translate {
            for value in offsets {
                *value = value.zoomed(factor);
            }
        }
        if let Some(value) = &mut style.perspective {
            *value *= factor;
        }
        zoom_position(&mut style.perspective_origin, factor);
        zoom_position(&mut style.object_position, factor);
        for filter in [&mut style.filter, &mut style.backdrop_filter] {
            for function in Arc::make_mut(filter) {
                match function {
                    paint::FilterFunction::Blur(value) => *value *= factor,
                    paint::FilterFunction::DropShadow {
                        offset, deviation, ..
                    } => {
                        *offset = offset.map(|value| value * factor);
                        *deviation *= factor;
                    }
                    _ => {}
                }
            }
        }
        for shadow in Arc::make_mut(&mut style.decoration.shadows) {
            shadow.offset = shadow.offset.map(|value| value * factor);
            shadow.blur *= factor;
            shadow.spread *= factor;
        }
        for shadow in Arc::make_mut(&mut style.text_paint.shadows) {
            shadow.offset = shadow.offset.map(|value| value * factor);
            shadow.blur *= factor;
        }
        style.text_paint.stroke_width *= factor;
        if let Some(width) = &mut style.text_decorations.declared.thickness {
            *width *= factor;
        }
        if let Some(value) = &mut style.text_decorations.underline_offset {
            *value = value.zoomed(factor);
        }
        if let crate::style::BaselineShift::Raise(value) = &mut style.baseline.shift {
            *value = value.zoomed(factor);
        }
        if let crate::style::VerticalAlign::Length(value) = &mut style.table.vertical_align {
            *value = value.zoomed(factor);
        }
        for size in &mut style.containment.intrinsic {
            if let Some(value) = &mut size.length {
                *value *= factor;
            }
        }
        zoom_backgrounds(Arc::make_mut(&mut style.decoration.backgrounds), factor);
        zoom_border_image(Arc::make_mut(&mut style.decoration.border_image), factor);
        zoom_image(&mut style.list_style_image, factor);
        let masks = Arc::make_mut(&mut style.decoration.masks);
        zoom_backgrounds(Arc::make_mut(&mut masks.layers), factor);
        zoom_border_image(Arc::make_mut(&mut masks.border), factor);
        if let Some(path) = &mut masks.clip_path
            && let Some(shape) = &mut Arc::make_mut(path).shape
        {
            zoom_shape(shape, factor);
        }
        let motion = Arc::make_mut(&mut style.motion);
        motion.distance = motion.distance.zoomed(factor);
        if let crate::style::motion::OffsetPosition::Position(value) = &mut motion.position {
            zoom_position(value, factor);
        }
        if let Some(value) = &mut motion.anchor {
            zoom_position(value, factor);
        }
        if let crate::style::motion::OffsetPath::Path { curve, .. } =
            Arc::make_mut(&mut motion.path)
        {
            match curve {
                crate::style::motion::Curve::Shape { shape, .. } => zoom_shape(shape, factor),
                crate::style::motion::Curve::Ray {
                    at: Some(value), ..
                } => zoom_position(value, factor),
                _ => {}
            }
        }
        for option in Arc::make_mut(&mut style.anchor.options) {
            *option = option.used();
        }
        for (_, page) in Arc::make_mut(&mut style.anchor.pages) {
            *page = page.used();
        }
        style
    }
}

fn zoom_position(position: &mut paint::Position, factor: f32) {
    for component in position {
        component.offset = component.offset.zoomed(factor);
    }
}

fn zoom_lengths(values: &mut [LengthPercentage], factor: f32) {
    for value in values {
        *value = value.zoomed(factor);
    }
}

fn zoom_shape(shape: &mut BasicShape, factor: f32) {
    match shape {
        BasicShape::Inset {
            insets: edges,
            radii,
        }
        | BasicShape::Rect { edges, radii } => {
            zoom_lengths(edges, factor);
            for corner in radii {
                zoom_lengths(corner, factor);
            }
        }
        BasicShape::Circle { radius, center } => {
            if let paint::CircleRadius::Length(value) = radius {
                *value = value.zoomed(factor);
            }
            zoom_position(center, factor);
        }
        BasicShape::Ellipse { radii, center } => {
            if let paint::EllipseRadii::Axes(axes) = radii {
                for axis in axes {
                    if let paint::AxisRadius::Length(value) = axis {
                        *value = value.zoomed(factor);
                    }
                }
            }
            zoom_position(center, factor);
        }
        BasicShape::Polygon { points, .. } => {
            for point in points {
                zoom_lengths(point, factor);
            }
        }
        BasicShape::Path { contours, .. } => {
            for contour in contours {
                *contour = contour.map(|point| point.map(|value| value * factor));
            }
        }
    }
}

fn zoom_image(image: &mut BackgroundImage, factor: f32) {
    if let BackgroundImage::Url { scale, .. } | BackgroundImage::Image { scale, .. } = image {
        *scale *= factor;
    }
    if let BackgroundImage::Gradient(gradient) = image {
        let conic = matches!(gradient.shape, paint::GradientShape::Conic { .. });
        match &mut gradient.shape {
            paint::GradientShape::Radial { ending, center } => {
                zoom_position(center, factor);
                match ending {
                    paint::RadialEnding::Circle(paint::CircleSize::Radius(value)) => {
                        *value *= factor;
                    }
                    paint::RadialEnding::Ellipse(paint::EllipseSize::Radii(values)) => {
                        zoom_lengths(values, factor);
                    }
                    _ => {}
                }
            }
            paint::GradientShape::Conic { center, .. } => zoom_position(center, factor),
            paint::GradientShape::Linear(_) => {}
        }
        if !conic {
            for item in &mut gradient.items {
                match item {
                    paint::GradientItem::Stop {
                        position: Some(value),
                        ..
                    }
                    | paint::GradientItem::Hint(value) => *value = value.zoomed(factor),
                    paint::GradientItem::Stop { .. } => {}
                }
            }
        }
    }
}

fn zoom_backgrounds(backgrounds: &mut paint::Backgrounds, factor: f32) {
    for image in &mut backgrounds.images {
        zoom_image(image, factor);
    }
    for positions in &mut backgrounds.positions {
        for position in positions {
            position.offset = position.offset.zoomed(factor);
        }
    }
    for size in &mut backgrounds.sizes {
        if let paint::BackgroundSize::Explicit(axes) = size {
            for axis in axes {
                if let paint::SizeComponent::Value(value) = axis {
                    *value = value.zoomed(factor);
                }
            }
        }
    }
}

fn zoom_border_image(border: &mut paint::BorderImage, factor: f32) {
    zoom_image(&mut border.source, factor);
    for slice in &mut border.slice {
        if let paint::ImageSlice::Number(value) = slice {
            *value *= factor;
        }
    }
    for width in &mut border.widths {
        if let paint::BorderImageWidth::Length(value) = width {
            *value = value.zoomed(factor);
        }
    }
    for outset in &mut border.outset {
        if let paint::BorderImageOutset::Length(value) = outset {
            *value *= factor;
        }
    }
}

fn zoom_track(track: &mut TrackSize, factor: f32) {
    let breadth = |breadth: &mut TrackBreadth| {
        if let TrackBreadth::Length(value) = breadth {
            *value = value.zoomed(factor);
        }
    };
    match track {
        TrackSize::Breadth(value) => breadth(value),
        TrackSize::MinMax(min, max) => {
            breadth(min);
            breadth(max);
        }
        TrackSize::FitContent(value) => *value = value.zoomed(factor),
    }
}
