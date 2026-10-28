use crate::paint::{Easing, LinearPoint, StepPosition};
use crate::style::{
    StyleError,
    calc::Kind,
    component::Component,
    grammar::{Keyword, Property},
    matcher::{ValueContext, match_value},
    units::Relative,
    values,
};

const EASE: [f32; 4] = [0.25, 0.1, 0.25, 1.0];
const EASE_IN: [f32; 4] = [0.42, 0.0, 1.0, 1.0];
const EASE_OUT: [f32; 4] = [0.0, 0.0, 0.58, 1.0];
const EASE_IN_OUT: [f32; 4] = [0.42, 0.0, 0.58, 1.0];
const MIN_LINEAR_STOPS: usize = 2;
const MIN_STEPS: i32 = 1;
const MIN_JUMP_NONE_STEPS: i32 = 2;

pub(super) fn parse(source: &str, relative: &Relative<'_>) -> Result<Option<Easing>, StyleError> {
    let before = relative.work.steps();
    let mut steps = before;
    let result = match_value(
        Property::AnimationTimingFunction.data().root,
        source,
        &mut steps,
        ValueContext::Nested(relative),
    )?;
    relative.work.charge(steps.saturating_sub(before))?;
    let Some(value) = result else {
        return Ok(None);
    };
    let [value] = value.items() else {
        return Ok(None);
    };
    Ok(match value.unwrap() {
        Component::Keyword(keyword) => Some(match keyword {
            Keyword::Linear => Easing::Linear,
            Keyword::Ease => Easing::CubicBezier(EASE),
            Keyword::EaseIn => Easing::CubicBezier(EASE_IN),
            Keyword::EaseOut => Easing::CubicBezier(EASE_OUT),
            Keyword::EaseInOut => Easing::CubicBezier(EASE_IN_OUT),
            Keyword::StepStart => Easing::Steps {
                count: 1,
                position: StepPosition::Start,
            },
            Keyword::StepEnd => Easing::Steps {
                count: 1,
                position: StepPosition::End,
            },
            _ => return Ok(None),
        }),
        Component::Function(Keyword::CubicBezier, _) => {
            let mut components = Vec::new();
            leaves(value.arguments(), &mut components);
            let coordinates = components
                .iter()
                .map(|value| values::number(value, relative))
                .collect::<Result<Vec<_>, _>>()?;
            let Ok(coordinates) = <[f32; 4]>::try_from(coordinates) else {
                return Ok(None);
            };
            Some(Easing::CubicBezier(coordinates))
        }
        Component::Function(Keyword::Steps, _) => {
            let mut components = Vec::new();
            leaves(value.arguments(), &mut components);
            let Some(count) = components.first() else {
                return Ok(None);
            };
            let position = match components.get(1).and_then(|value| value.keyword()) {
                None | Some(Keyword::End | Keyword::JumpEnd) => StepPosition::End,
                Some(Keyword::Start | Keyword::JumpStart) => StepPosition::Start,
                Some(Keyword::JumpNone) => StepPosition::None,
                Some(Keyword::JumpBoth) => StepPosition::Both,
                _ => return Ok(None),
            };
            let minimum = if position == StepPosition::None {
                MIN_JUMP_NONE_STEPS
            } else {
                MIN_STEPS
            };
            let count = match count.unwrap() {
                Component::Math(source, Kind::Number, range) => {
                    values::math(source, relative, Kind::Number, None)?.integer(
                        crate::style::calc::Range {
                            min: range.min.max(minimum as f32),
                            ..*range
                        },
                    )
                }
                count => values::integer(count, relative)?,
            };
            if count < minimum {
                return Ok(None);
            }
            Some(Easing::Steps {
                count: count as u32,
                position,
            })
        }
        Component::Function(Keyword::Linear, _) => linear(value.arguments(), relative)?,
        _ => None,
    })
}

fn leaves<'a>(values: &'a [Component], output: &mut Vec<&'a Component>) {
    for value in values {
        match value.unwrap() {
            Component::Group(values) => leaves(values, output),
            Component::Slots(slots) => {
                for value in slots.iter().flatten() {
                    leaves(std::slice::from_ref(value), output);
                }
            }
            Component::Delim(_) => {}
            value => output.push(value),
        }
    }
}

fn linear(stops: &[Component], relative: &Relative<'_>) -> Result<Option<Easing>, StyleError> {
    if stops.len() < MIN_LINEAR_STOPS {
        return Ok(None);
    }
    let mut points = Vec::new();
    let mut largest = f32::NEG_INFINITY;
    for (index, stop) in stops.iter().enumerate() {
        let mut components = Vec::new();
        leaves(std::slice::from_ref(stop), &mut components);
        let mut output = None;
        let mut positions = Vec::new();
        for component in components {
            match component {
                Component::Percentage(value) => positions.push(*value),
                Component::Math(source, Kind::Percentage, range) => positions.push(
                    values::math(source, relative, Kind::Percentage, None)?.resolve(0.0, *range),
                ),
                value => {
                    if output.is_some() {
                        return Ok(None);
                    }
                    output = Some(values::number(value, relative)?);
                }
            }
        }
        let Some(output) = output else {
            return Ok(None);
        };
        if positions.is_empty() {
            let input = if index == 0 {
                largest = 0.0;
                Some(0.0)
            } else if index + 1 == stops.len() {
                Some(largest.max(1.0))
            } else {
                None
            };
            points.push((input, output));
        } else {
            for position in positions {
                largest = largest.max(position);
                points.push((Some(largest), output));
            }
        }
    }
    let mut start = 0;
    while start + 1 < points.len() {
        let end = start
            + 1
            + points[start + 1..]
                .iter()
                .position(|(input, _)| input.is_some())
                .expect("the last stop has an input");
        let first = f64::from(points[start].0.expect("the first stop has an input"));
        let last = f64::from(points[end].0.unwrap());
        let step = (last - first) / (end - start) as f64;
        for (offset, point) in points[start + 1..end].iter_mut().enumerate() {
            point.0 = Some((first + step * (offset + 1) as f64) as f32);
        }
        start = end;
    }
    Ok(Some(Easing::Piecewise(
        points
            .into_iter()
            .map(|(input, output)| LinearPoint {
                input: input.unwrap(),
                output,
            })
            .collect(),
    )))
}
