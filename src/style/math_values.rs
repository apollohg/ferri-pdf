//! `MathML` Core §4.3–§4.5, computed before font-size.
use crate::style::{Style, StyleError, component::Component, grammar::Keyword, units::Relative};
use serde::Serialize;

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize)]
pub(crate) struct Math {
    pub depth: i32,
    pub compact: bool,
    pub shift: bool,
}
pub(super) fn depth(
    value: &Component,
    parent: &Style,
    relative: &Relative<'_>,
) -> Result<i32, StyleError> {
    let add = match value.unwrap() {
        Component::Keyword(Keyword::AutoAdd) => Some(i32::from(parent.math.compact)),
        Component::Function(Keyword::Add, Some(inner)) => {
            Some(crate::style::values::integer(inner, relative)?)
        }
        _ => None,
    };
    if let Some(add) = add {
        parent
            .math
            .depth
            .checked_add(add)
            .ok_or_else(|| "math-depth overflow".into())
    } else {
        crate::style::values::integer(value, relative)
    }
}
pub(super) fn scale(
    mut inherited: i32,
    mut computed: i32,
    font: Option<[f32; 2]>,
) -> Result<f32, StyleError> {
    const FALLBACK: f32 = 0.71;
    let invert = computed < inherited;
    if invert {
        std::mem::swap(&mut inherited, &mut computed);
    }
    let mut exponent = i64::from(computed) - i64::from(inherited);
    let mut factor = 1.0;
    if exponent > 0
        && let Some([script, script_script]) =
            font.filter(|values| values.iter().all(|value| *value > 0.0))
    {
        if inherited <= 0 && computed >= 2 {
            factor *= script_script;
            exponent -= 2;
        } else if inherited == 1 {
            factor *= script_script / script;
            exponent -= 1;
        } else if computed == 1 {
            factor *= script;
            exponent -= 1;
        }
    }
    factor *= f64::from(FALLBACK).powf(exponent as f64) as f32;
    if invert {
        factor = 1.0 / factor;
    }
    if factor.is_finite() && factor > 0.0 {
        Ok(factor)
    } else {
        Err("non-finite math font scaling".into())
    }
}

impl crate::style::Stylesheet {
    pub(crate) fn math_attribute_length(
        &self,
        source: &str,
        style: &Style,
        resolver: &dyn crate::resources::ResourceResolver,
    ) -> Result<Option<crate::style::LengthPercentage>, StyleError> {
        let Some(value) = crate::style::declaration::value(
            crate::style::grammar::Property::MarginTop,
            source,
            &mut 0,
        )?
        else {
            return Ok(None);
        };
        let relative = self.inherited(style, self.page_area(), resolver);
        let Ok(value) = crate::style::box_values::length_percentage(&value, &relative) else {
            return Ok(None);
        };
        Ok(Some(value))
    }
}
