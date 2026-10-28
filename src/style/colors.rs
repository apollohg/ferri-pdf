use crate::style::{
    StyleError,
    calc::{Context, Kind, MathError, math_function},
    component::Component,
    grammar::Keyword,
    units::Relative,
};
use cssparser::{ParseError, ParseErrorKind, Parser, ParserInput};
use cssparser_color::{
    AngleOrNumber, Color as ParsedColor, ColorParser, DefaultColorParser, NumberOrPercentage,
    RgbaLegacy,
};
use ferri_pdf_paint::Color;
use std::cell::Cell;

/// CSS Color 4 §6.2 system colours for a white page with black text.
const CANVAS: Color = Color {
    rgb: [u8::MAX; 3],
    alpha: 1.0,
};
const CANVAS_TEXT: Color = Color::BLACK;

/// A math-function channel: its kind and value.
type Channel<'i> = Result<(Option<Kind>, f32), ParseError<'i, ColorError>>;

#[derive(Debug)]
enum ColorError {
    InvalidComponent,
    Math(MathError),
}
impl From<()> for ColorError {
    fn from((): ()) -> Self {
        Self::InvalidComponent
    }
}

pub(super) struct ParsedColorValue {
    pub color: ParsedColor,
    pub has_math: bool,
}

/// Colour syntax whose components also accept math functions.
struct MathColorParser<'r> {
    relative: &'r Relative<'r>,
    has_math: Cell<bool>,
}
impl MathColorParser<'_> {
    fn math<'i>(&self, input: &mut Parser<'i, '_>, kinds: &[Kind]) -> Option<Channel<'i>> {
        let context = Context {
            relative: self.relative,
            percentage: kinds
                .contains(&Kind::Percentage)
                .then_some(Kind::Percentage),
        };
        let math = math_function(input, context)?;
        self.has_math.set(true);
        Some(
            math.and_then(|math| math.expect(kinds))
                .map(|math| {
                    (
                        math.kind(),
                        math.resolve(0.0, crate::style::calc::Range::ALL),
                    )
                })
                .map_err(|error| input.new_custom_error(ColorError::Math(error))),
        )
    }
}
impl<'i> ColorParser<'i> for MathColorParser<'_> {
    type Output = ParsedColor;
    type Error = ColorError;
    fn parse_angle_or_number<'t>(
        &self,
        input: &mut Parser<'i, 't>,
    ) -> Result<AngleOrNumber, ParseError<'i, ColorError>> {
        match self.math(input, &[Kind::Angle, Kind::Number]) {
            Some(result) => result.map(|(kind, value)| match kind {
                Some(Kind::Angle) => AngleOrNumber::Angle { degrees: value },
                _ => AngleOrNumber::Number { value },
            }),
            None => DefaultColorParser
                .parse_angle_or_number(input)
                .map_err(ParseError::into),
        }
    }
    fn parse_percentage<'t>(
        &self,
        input: &mut Parser<'i, 't>,
    ) -> Result<f32, ParseError<'i, ColorError>> {
        match self.math(input, &[Kind::Percentage]) {
            Some(result) => result.map(|(_, value)| value),
            None => DefaultColorParser
                .parse_percentage(input)
                .map_err(ParseError::into),
        }
    }
    fn parse_number<'t>(
        &self,
        input: &mut Parser<'i, 't>,
    ) -> Result<f32, ParseError<'i, ColorError>> {
        match self.math(input, &[Kind::Number]) {
            Some(result) => result.map(|(_, value)| value),
            None => DefaultColorParser
                .parse_number(input)
                .map_err(ParseError::into),
        }
    }
    fn parse_number_or_percentage<'t>(
        &self,
        input: &mut Parser<'i, 't>,
    ) -> Result<NumberOrPercentage, ParseError<'i, ColorError>> {
        match self.math(input, &[Kind::Number, Kind::Percentage]) {
            Some(result) => result.map(|(kind, value)| match kind {
                Some(Kind::Percentage) => NumberOrPercentage::Percentage { unit_value: value },
                _ => NumberOrPercentage::Number { value },
            }),
            None => DefaultColorParser
                .parse_number_or_percentage(input)
                .map_err(ParseError::into),
        }
    }
}

/// One complete colour value, if cssparser-color can represent it.
pub(super) fn parse(value: &str, relative: &Relative<'_>) -> Result<ParsedColorValue, StyleError> {
    let mut input = ParserInput::new(value);
    let mut parser = Parser::new(&mut input);
    let color_parser = MathColorParser {
        relative,
        has_math: Cell::new(false),
    };
    let color =
        cssparser_color::parse_color_with(&color_parser, &mut parser).map_err(
            |error| match error.kind {
                ParseErrorKind::Custom(ColorError::Math(MathError::Resolution(error))) => error,
                kind => StyleError::Invalid(format!("invalid colour {value}: {kind:?}")),
            },
        )?;
    parser
        .expect_exhausted()
        .map_err(|error| StyleError::Invalid(format!("colour: {error:?}")))?;
    Ok(ParsedColorValue {
        color,
        has_math: color_parser.has_math.get(),
    })
}

/// Computes a matched `<color>`; `currentcolor` is `current`.
pub(super) fn color(
    value: &Component,
    relative: &Relative<'_>,
    current: Color,
) -> Result<Color, StyleError> {
    let computed;
    let parsed = match value.unwrap() {
        Component::Color(parsed) => parsed,
        Component::MathColor(source) => {
            computed = parse(source, relative)?.color;
            &computed
        }
        other => {
            return match other.keyword() {
                Some(Keyword::Canvas) => Ok(CANVAS),
                Some(Keyword::Canvastext) => Ok(CANVAS_TEXT),
                _ => Err(StyleError::Unsupported(format!("colour {value}"))),
            };
        }
    };
    let rgba = match parsed {
        ParsedColor::CurrentColor => return Ok(current),
        ParsedColor::Rgba(rgba) => *rgba,
        ParsedColor::Hsl(hsl) => {
            let (r, g, b) = cssparser_color::hsl_to_rgb(
                hsl.hue.unwrap_or(0.0) / 360.0,
                hsl.saturation.unwrap_or(0.0),
                hsl.lightness.unwrap_or(0.0),
            );
            RgbaLegacy::from_floats(r, g, b, hsl.alpha.unwrap_or(1.0))
        }
        ParsedColor::Hwb(hwb) => {
            let (r, g, b) = cssparser_color::hwb_to_rgb(
                hwb.hue.unwrap_or(0.0) / 360.0,
                hwb.whiteness.unwrap_or(0.0),
                hwb.blackness.unwrap_or(0.0),
            );
            RgbaLegacy::from_floats(r, g, b, hwb.alpha.unwrap_or(1.0))
        }
        _ => return Err(StyleError::Unsupported(format!("colour space of {value}"))),
    };
    if !rgba.alpha.is_finite() {
        return Err("non-finite colour alpha".into());
    }
    Ok(Color {
        rgb: [rgba.red, rgba.green, rgba.blue],
        alpha: rgba.alpha,
    })
}
