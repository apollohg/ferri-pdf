//! Support of filter results through their inputs and primitive subregions (Filter Effects 1 §9).
use super::{
    ColorMatrixKind, CompositeOperator, EdgeMode, Filter, Input, Kind, MorphologyOperator,
};
use crate::svg::usvg::{Rect, Transform};

const ALPHA_ROW: usize = 15;
const ALPHA_OFFSET: usize = 19;

fn union(a: Option<Rect>, b: Option<Rect>) -> Option<Rect> {
    match (a, b) {
        (Some(a), Some(b)) => a.join(&b),
        (a, b) => a.or(b),
    }
}
fn intersection(a: Option<Rect>, b: Option<Rect>) -> Option<Rect> {
    a?.intersect(&b?).filter(|rect| !rect.is_empty())
}
fn translated(rect: Rect, x: f32, y: f32) -> Option<Rect> {
    rect.transform(Transform::from_translate(x, y))
}

fn convolved(rect: Rect, region: Rect, dx: f32, dy: f32, edge: EdgeMode) -> Option<Rect> {
    let shifted = translated(rect, dx, dy)?;
    if edge == EdgeMode::Wrap {
        let axis = |start: f32, length: f32, origin: f32, extent: f32| {
            let start = origin + (start - origin).rem_euclid(extent);
            if length >= extent || start + length > origin + extent {
                (origin, extent)
            } else {
                (start, length)
            }
        };
        let (x, width) = axis(shifted.x(), shifted.width(), region.x(), region.width());
        let (y, height) = axis(shifted.y(), shifted.height(), region.y(), region.height());
        Rect::from_xywh(x, y, width, height)
    } else if edge == EdgeMode::Duplicate {
        Rect::from_ltrb(
            if rect.left() == region.left() && dx > 0.0 {
                region.left()
            } else {
                shifted.left()
            },
            if rect.top() == region.top() && dy > 0.0 {
                region.top()
            } else {
                shifted.top()
            },
            if rect.right() == region.right() && dx < 0.0 {
                region.right()
            } else {
                shifted.right()
            },
            if rect.bottom() == region.bottom() && dy < 0.0 {
                region.bottom()
            } else {
                shifted.bottom()
            },
        )
    } else {
        Some(shifted)
    }
}

impl Filter {
    /// Bounds of possibly non-transparent output, in the filter's user coordinates.
    pub(crate) fn support_bounds(&self, source: Option<Rect>) -> Option<Rect> {
        let region = Some(self.rect().to_rect());
        let source = intersection(source, region);
        let mut results: Vec<(&str, Option<Rect>)> = Vec::new();
        for primitive in self.primitives() {
            let subregion = intersection(Some(primitive.rect().to_rect()), region);
            let input = |input: &Input| match input {
                Input::SourceGraphic | Input::SourceAlpha => source,
                Input::Reference(name) => results
                    .iter()
                    .rev()
                    .find_map(|(result, rect)| (*result == name).then_some(*rect))
                    .unwrap_or(source),
            };
            let support = match primitive.kind() {
                Kind::Blend(fe) => union(input(fe.input1()), input(fe.input2())),
                Kind::ColorMatrix(fe) => match fe.kind() {
                    ColorMatrixKind::Matrix(values) if values[ALPHA_OFFSET] > 0.0 => subregion,
                    ColorMatrixKind::Matrix(values)
                        if values[ALPHA_ROW..ALPHA_OFFSET].iter().all(|v| *v <= 0.0) =>
                    {
                        None
                    }
                    _ => input(fe.input()),
                },
                Kind::ComponentTransfer(fe) => {
                    if fe.func_a().evaluate(0.0) > 0.0 {
                        subregion
                    } else if !fe.func_a().has_positive_output() {
                        None
                    } else {
                        input(fe.input())
                    }
                }
                Kind::Composite(fe) => {
                    let (a, b) = (input(fe.input1()), input(fe.input2()));
                    match fe.operator() {
                        CompositeOperator::In => intersection(a, b),
                        CompositeOperator::Out => a,
                        CompositeOperator::Atop => b,
                        CompositeOperator::Over | CompositeOperator::Xor => union(a, b),
                        CompositeOperator::Arithmetic { k1, k2, k3, k4 } => {
                            let [k1, k2, k3, k4] = [k1, k2, k3, k4].map(f64::from);
                            // A bilinear alpha polynomial attains its maximum at a unit-square corner.
                            let generated = (k4 > 0.0).then_some(subregion).flatten();
                            let product = (k1 + k2 + k3 + k4 > 0.0)
                                .then(|| intersection(a, b))
                                .flatten();
                            let first = (k2 + k4 > 0.0).then_some(a).flatten();
                            let second = (k3 + k4 > 0.0).then_some(b).flatten();
                            union(union(generated, product), union(first, second))
                        }
                    }
                }
                Kind::ConvolveMatrix(fe) => {
                    let original = input(fe.input());
                    if fe.preserve_alpha() {
                        original
                    } else if fe.bias() > 0.0 {
                        subregion
                    } else {
                        let matrix = fe.matrix();
                        let mut result = None;
                        for row in 0..matrix.rows() {
                            for column in 0..matrix.columns() {
                                if matrix.get(column, row) / fe.divisor().get() > 0.0 {
                                    result = union(
                                        result,
                                        original.and_then(|rect| {
                                            convolved(
                                                rect,
                                                self.rect().to_rect(),
                                                matrix.target_x() as f32 + column as f32 + 1.0
                                                    - matrix.columns() as f32,
                                                matrix.target_y() as f32 + row as f32 + 1.0
                                                    - matrix.rows() as f32,
                                                fe.edge_mode(),
                                            )
                                        }),
                                    );
                                }
                            }
                        }
                        result
                    }
                }
                Kind::DiffuseLighting(_) => subregion,
                Kind::SpecularLighting(fe) => (fe.specular_constant() > 0.0
                    && fe.lighting_color() != crate::svg::usvg::Color::black())
                .then_some(subregion)
                .flatten(),
                Kind::DisplacementMap(fe) => input(fe.input1()).and_then(|rect| {
                    if input(fe.input2()).is_none() {
                        translated(rect, fe.scale() / 2.0, fe.scale() / 2.0)
                    } else {
                        rect.outset(fe.scale().abs() / 2.0, fe.scale().abs() / 2.0)
                    }
                }),
                Kind::DropShadow(fe) => {
                    let original = input(fe.input());
                    let shadow = if fe.opacity().get() > 0.0 {
                        original
                            .and_then(|rect| {
                                rect.outset(
                                    crate::paint::BLUR_EXTENT * fe.std_dev_x().get(),
                                    crate::paint::BLUR_EXTENT * fe.std_dev_y().get(),
                                )
                            })
                            .and_then(|rect| translated(rect, fe.dx(), fe.dy()))
                    } else {
                        None
                    };
                    union(original, shadow)
                }
                Kind::Flood(fe) => (fe.opacity().get() > 0.0).then_some(subregion).flatten(),
                Kind::GaussianBlur(fe) => input(fe.input()).and_then(|rect| {
                    rect.outset(
                        crate::paint::BLUR_EXTENT * fe.std_dev_x().get(),
                        crate::paint::BLUR_EXTENT * fe.std_dev_y().get(),
                    )
                }),
                Kind::Image(fe) => fe
                    .root()
                    .has_children()
                    .then(|| fe.root().layer_bounding_box().to_rect())
                    .and_then(|rect| translated(rect, primitive.rect().x(), primitive.rect().y())),
                Kind::Merge(fe) => fe
                    .inputs()
                    .iter()
                    .fold(None, |result, item| union(result, input(item))),
                Kind::Morphology(fe) => input(fe.input()).and_then(|rect| {
                    if fe.radius_x().get() == 0.0 || fe.radius_y().get() == 0.0 {
                        return Some(rect);
                    }
                    let direction = if fe.operator() == MorphologyOperator::Dilate {
                        1.0
                    } else {
                        -1.0
                    };
                    rect.outset(
                        direction * fe.radius_x().get(),
                        direction * fe.radius_y().get(),
                    )
                }),
                Kind::Offset(fe) => {
                    input(fe.input()).and_then(|rect| translated(rect, fe.dx(), fe.dy()))
                }
                Kind::Tile(fe) => input(fe.input()).and(subregion),
                Kind::Turbulence(fe) => {
                    if fe.kind() == super::TurbulenceKind::Turbulence
                        && (fe.num_octaves() == 0
                            || (fe.base_frequency_x().get() == 0.0
                                && fe.base_frequency_y().get() == 0.0))
                    {
                        None
                    } else {
                        subregion
                    }
                }
            };
            results.push((primitive.result(), intersection(support, subregion)));
        }
        results.last().and_then(|(_, rect)| *rect)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primitive_support_tracks_named_results_generated_alpha_and_geometric_operations() {
        let source = Rect::from_xywh(50.0, 20.0, 10.0, 10.0).unwrap();
        for (primitives, expected) in [
            (
                "<feOffset dx='-20' result='shift'/><feMerge><feMergeNode in='shift'/><feMergeNode in='SourceGraphic'/></feMerge>",
                Some([30.0, 20.0, 30.0, 10.0]),
            ),
            (
                "<feFlood x='52' y='20' width='5' height='10' result='band'/><feComposite in='SourceGraphic' in2='band' operator='in'/>",
                Some([52.0, 20.0, 5.0, 10.0]),
            ),
            (
                "<feComponentTransfer><feFuncA type='linear' slope='0' intercept='1'/></feComponentTransfer>",
                Some([0.0, 0.0, 200.0, 200.0]),
            ),
            (
                "<feMorphology operator='dilate' radius='2'/>",
                Some([48.0, 18.0, 14.0, 14.0]),
            ),
            (
                "<feMorphology operator='erode' radius='2'/>",
                Some([52.0, 22.0, 6.0, 6.0]),
            ),
            (
                "<feConvolveMatrix order='3' kernelMatrix='0 0 0 1 0 0 0 0 0' edgeMode='none'/>",
                Some([49.0, 20.0, 10.0, 10.0]),
            ),
            (
                "<feConvolveMatrix order='3' kernelMatrix='0 0 0 1 0 0 0 0 0' edgeMode='wrap'/>",
                Some([49.0, 20.0, 10.0, 10.0]),
            ),
            (
                "<feComponentTransfer><feFuncA type='linear' slope='0' intercept='0'/></feComponentTransfer>",
                None,
            ),
            (
                "<feComposite in='SourceGraphic' in2='SourceGraphic' operator='arithmetic' k1='-1'/>",
                None,
            ),
            (
                "<feComposite in='SourceGraphic' in2='SourceGraphic' operator='arithmetic' k1='1' k2='-1'/>",
                None,
            ),
            (
                "<feComposite in='SourceGraphic' in2='SourceGraphic' operator='arithmetic' k2='1' k4='-2'/>",
                None,
            ),
            (
                "<feSpecularLighting lighting-color='black' specularConstant='1'><feDistantLight elevation='90'/></feSpecularLighting>",
                None,
            ),
            (
                "<feDiffuseLighting lighting-color='black' diffuseConstant='1'><feDistantLight elevation='90'/></feDiffuseLighting>",
                Some([0.0, 0.0, 200.0, 200.0]),
            ),
            (
                "<feMorphology operator='dilate' radius='0 2'/>",
                Some([50.0, 20.0, 10.0, 10.0]),
            ),
            ("<feTurbulence/>", None),
            (
                "<feTurbulence type='fractalNoise'/>",
                Some([0.0, 0.0, 200.0, 200.0]),
            ),
            ("<feTurbulence baseFrequency='0.1' numOctaves='0'/>", None),
            (
                "<feMorphology operator='dilate' radius='2 broken'/>",
                Some([50.0, 20.0, 10.0, 10.0]),
            ),
            (
                "<feMorphology operator='dilate' radius='2 3 broken'/>",
                Some([50.0, 20.0, 10.0, 10.0]),
            ),
            (
                "<feGaussianBlur stdDeviation='2 broken'/>",
                Some([50.0, 20.0, 10.0, 10.0]),
            ),
            (
                "<feDiffuseLighting diffuseConstant='0'><feDistantLight elevation='90'/></feDiffuseLighting>",
                Some([0.0, 0.0, 200.0, 200.0]),
            ),
            (
                "<feOffset dx='-20' x='35' y='20' width='2' height='10'/>",
                Some([35.0, 20.0, 2.0, 10.0]),
            ),
            (
                "<feOffset dx='-20' x='45' y='20' width='2' height='10'/>",
                None,
            ),
        ] {
            let xml = format!(
                "<svg xmlns='http://www.w3.org/2000/svg'><defs><filter id='ink' filterUnits='userSpaceOnUse' x='0' y='0' width='200' height='200'>{primitives}</filter></defs><rect x='50' y='20' width='10' height='10' filter='url(#ink)'/></svg>"
            );
            let tree =
                crate::svg::usvg::Tree::from_str(&xml, &crate::svg::usvg::Options::default())
                    .unwrap();
            let filter =
                crate::svg::first_filter(tree.root()).expect("fixture has a referenced SVG filter");
            let actual = filter
                .support_bounds(Some(source))
                .map(|rect| [rect.x(), rect.y(), rect.width(), rect.height()]);
            assert_eq!(actual, expected, "support of SVG primitives {primitives}");
        }
    }
}
