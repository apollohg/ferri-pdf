//! CSS Color 4 and 5 `<color>` values: every colour function with math functions, `none`
//! and relative colour channel keywords, `color-mix()`, `light-dark()` and system colours.
use crate::paint::{
    AbsoluteColor, Color, ColorSpace, HueInterpolation, Interpolation, normalize_hue,
};
use crate::style::{
    ColorScheme, StyleError,
    calc::{Context, Kind, MathError, Range, math_function},
    component::Component,
    parser::MAX_COMPONENT_DEPTH,
    units::{Dimension, Relative},
};
use cssparser::{ParseError, ParseErrorKind, Parser, ParserInput, Token};
use std::cell::Cell;

const NONE: &str = "none";
const FROM: &str = "from";
const IN: &str = "in";
const HUE: &str = "hue";
const ALPHA: &str = "alpha";
pub(super) const CURRENT_COLOR: &str = "currentcolor";
const ACCENT_COLOR: &str = "accentcolor";
const CANVAS: &str = "canvas";
const ACCENT_COLOR: &str = "accentcolor";
const ACCENT_COLOR_TEXT: &str = "accentcolortext";
/// The ambient flare WCAG 2 adds to relative luminances in contrast ratios.
const LUMINANCE_FLARE: f32 = 0.05;
const TRANSPARENT: &str = "transparent";
const ALPHA_SEPARATOR: char = '/';
const CUSTOM_SPACE_PREFIX: &str = "--";
const RGB_NUMBER_SCALE: f32 = 1.0 / 255.0;
const PERCENT_SCALE: f32 = 100.0;
const LAB_OPPONENT_SCALE: f32 = 125.0;
const LCH_CHROMA_SCALE: f32 = 150.0;
const OKLAB_OPPONENT_SCALE: f32 = 0.4;
const OKLCH_CHROMA_SCALE: f32 = 0.4;
/// An omitted absolute alpha (CSS Color 4 §4.2).
const OPAQUE: f32 = 1.0;
/// Mix percentages are fractions of the whole.
const WHOLE: f32 = 1.0;
const EVEN_MIX: f32 = 0.5;
const DEFAULT_MIX: Interpolation = Interpolation {
    space: ColorSpace::Oklab,
    hue: HueInterpolation::Shorter,
};

/// UA system colours (CSS Color 4 §6.2) as 8-bit sRGB for the light and dark schemes; the
/// spec's legible foreground and background pairs meet WCAG AA contrast.
const SYSTEM_COLORS: [(&str, [u8; 3], [u8; 3]); 19] = [
    (ACCENT_COLOR, [0x00, 0x60, 0xdf], [0x99, 0xc8, 0xff]),
    ("accentcolortext", [0xff, 0xff, 0xff], [0x00, 0x00, 0x00]),
    ("activetext", [0xee, 0x00, 0x00], [0xff, 0x9e, 0x9e]),
    ("buttonborder", [0x76, 0x76, 0x76], [0x8e, 0x8e, 0x8e]),
    ("buttonface", [0xef, 0xef, 0xef], [0x3b, 0x3b, 0x3b]),
    ("buttontext", [0x00, 0x00, 0x00], [0xff, 0xff, 0xff]),
    ("canvas", [0xff, 0xff, 0xff], [0x12, 0x12, 0x12]),
    ("canvastext", [0x00, 0x00, 0x00], [0xff, 0xff, 0xff]),
    ("field", [0xff, 0xff, 0xff], [0x3b, 0x3b, 0x3b]),
    ("fieldtext", [0x00, 0x00, 0x00], [0xff, 0xff, 0xff]),
    ("graytext", [0x6d, 0x6d, 0x6d], [0x9e, 0x9e, 0x9e]),
    ("highlight", [0xb5, 0xd5, 0xff], [0x3f, 0x63, 0x8b]),
    ("highlighttext", [0x00, 0x00, 0x00], [0xff, 0xff, 0xff]),
    ("linktext", [0x00, 0x00, 0xee], [0x9e, 0x9e, 0xff]),
    ("mark", [0xff, 0xff, 0x00], [0x66, 0x56, 0x1f]),
    ("marktext", [0x00, 0x00, 0x00], [0xff, 0xff, 0xff]),
    ("selecteditem", [0x00, 0x60, 0xdf], [0x99, 0xc8, 0xff]),
    ("selecteditemtext", [0xff, 0xff, 0xff], [0x00, 0x00, 0x00]),
    ("visitedtext", [0x55, 0x1a, 0x8b], [0xd0, 0xad, 0xf0]),
];
/// Deprecated system colours and the system colour each computes to (CSS Color 4 Appendix A).
const DEPRECATED_SYSTEM_COLORS: [(&str, &str); 23] = [
    ("activeborder", "buttonborder"),
    ("activecaption", "canvas"),
    ("appworkspace", "canvas"),
    ("background", "canvas"),
    ("buttonhighlight", "buttonface"),
    ("buttonshadow", "buttonface"),
    ("captiontext", "canvastext"),
    ("inactiveborder", "buttonborder"),
    ("inactivecaption", "canvas"),
    ("inactivecaptiontext", "graytext"),
    ("infobackground", "canvas"),
    ("infotext", "canvastext"),
    ("menu", "canvas"),
    ("menutext", "canvastext"),
    ("scrollbar", "canvas"),
    ("threeddarkshadow", "buttonborder"),
    ("threedface", "buttonface"),
    ("threedhighlight", "buttonborder"),
    ("threedlightshadow", "buttonborder"),
    ("threedshadow", "buttonborder"),
    ("window", "canvas"),
    ("windowframe", "buttonborder"),
    ("windowtext", "canvastext"),
];
/// `color()` spaces (CSS Color 4 §10.1); `xyz` computes to `xyz-d65` (§15.4).
const PREDEFINED_SPACES: [(&str, ColorSpace); 10] = [
    ("srgb", ColorSpace::Srgb),
    ("srgb-linear", ColorSpace::SrgbLinear),
    ("display-p3", ColorSpace::DisplayP3),
    ("display-p3-linear", ColorSpace::DisplayP3Linear),
    ("a98-rgb", ColorSpace::A98Rgb),
    ("prophoto-rgb", ColorSpace::ProphotoRgb),
    ("rec2020", ColorSpace::Rec2020),
    ("xyz", ColorSpace::XyzD65),
    ("xyz-d50", ColorSpace::XyzD50),
    ("xyz-d65", ColorSpace::XyzD65),
];
/// `<color-space>` names beyond the predefined spaces (CSS Color 4 §13.2).
const INTERPOLATION_SPACES: [(&str, ColorSpace); 6] = [
    ("lab", ColorSpace::Lab),
    ("oklab", ColorSpace::Oklab),
    ("hsl", ColorSpace::Hsl),
    ("hwb", ColorSpace::Hwb),
    ("lch", ColorSpace::Lch),
    ("oklch", ColorSpace::Oklch),
];
/// Serializes a computed colour (CSS Color 4 §15): legacy sRGB colours as `rgb()`/`rgba()`,
/// everything else in its own notation, with missing components as `none`.
pub(crate) fn serialize(
    color: &AbsoluteColor,
    f: &mut std::fmt::Formatter<'_>,
) -> std::fmt::Result {
    if color.legacy || matches!(color.space, ColorSpace::Hsl | ColorSpace::Hwb) {
        let Color {
            rgb: [red, green, blue],
            alpha,
        } = color.used();
        return if alpha < 1.0 {
            write!(f, "rgba({red}, {green}, {blue}, {alpha})")
        } else {
            write!(f, "rgb({red}, {green}, {blue})")
        };
    }
    // `xyz` and `xyz-d65` share a space; the reverse search finds the canonical `xyz-d65`.
    if let Some((name, _)) = PREDEFINED_SPACES
        .iter()
        .rev()
        .find(|(_, space)| *space == color.space)
    {
        write!(f, "color({name} ")?;
    } else if let Some((name, _)) = INTERPOLATION_SPACES
        .iter()
        .find(|(_, space)| *space == color.space)
    {
        write!(f, "{name}(")?;
    }
    for (index, component) in color.components.iter().enumerate() {
        if index > 0 {
            f.write_str(" ")?;
        }
        write_component(*component, f)?;
    }
    if color.alpha != Some(1.0) {
        f.write_str(" / ")?;
        write_component(color.alpha, f)?;
    }
    f.write_str(")")
}

fn write_component(component: Option<f32>, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    match component {
        Some(value) => write!(f, "{value}"),
        None => f.write_str("none"),
    }
}

const HUE_METHODS: [(&str, HueInterpolation); 4] = [
    ("shorter", HueInterpolation::Shorter),
    ("longer", HueInterpolation::Longer),
    ("increasing", HueInterpolation::Increasing),
    ("decreasing", HueInterpolation::Decreasing),
];

/// One argument of a colour function: its keyword, what 100% means in the space's reference
/// units, and the parsed-value clamp of absolute colours.
#[derive(Clone, Copy)]
struct Channel {
    name: &'static str,
    hue: bool,
    percent: f32,
    min: f32,
    max: f32,
}
const fn linear(name: &'static str, percent: f32, min: f32, max: f32) -> Channel {
    Channel {
        name,
        hue: false,
        percent,
        min,
        max,
    }
}
const fn unbounded(name: &'static str, percent: f32) -> Channel {
    linear(name, percent, f32::NEG_INFINITY, f32::INFINITY)
}
const HUE_CHANNEL: Channel = Channel {
    name: "h",
    hue: true,
    percent: 0.0,
    min: f32::NEG_INFINITY,
    max: f32::INFINITY,
};

/// A colour function's space, arguments, and the scale from its `<number>`s to reference units;
/// `legacy` colours interpolate in sRGB by default and `commas` allows the legacy syntax.
struct Notation {
    space: ColorSpace,
    channels: [Channel; 3],
    number_scale: f32,
    legacy: bool,
    commas: bool,
}
const RGB: Notation = Notation {
    space: ColorSpace::Srgb,
    channels: [
        linear("r", 1.0, 0.0, 1.0),
        linear("g", 1.0, 0.0, 1.0),
        linear("b", 1.0, 0.0, 1.0),
    ],
    number_scale: RGB_NUMBER_SCALE,
    legacy: true,
    commas: true,
};
const HSL: Notation = Notation {
    space: ColorSpace::Hsl,
    channels: [
        HUE_CHANNEL,
        linear("s", PERCENT_SCALE, 0.0, f32::INFINITY),
        unbounded("l", PERCENT_SCALE),
    ],
    number_scale: 1.0,
    legacy: true,
    commas: true,
};
const HWB: Notation = Notation {
    space: ColorSpace::Hwb,
    channels: [
        HUE_CHANNEL,
        unbounded("w", PERCENT_SCALE),
        unbounded("b", PERCENT_SCALE),
    ],
    number_scale: 1.0,
    legacy: true,
    commas: false,
};
const LAB: Notation = Notation {
    space: ColorSpace::Lab,
    channels: [
        linear("l", PERCENT_SCALE, 0.0, PERCENT_SCALE),
        unbounded("a", LAB_OPPONENT_SCALE),
        unbounded("b", LAB_OPPONENT_SCALE),
    ],
    number_scale: 1.0,
    legacy: false,
    commas: false,
};
const LCH: Notation = Notation {
    space: ColorSpace::Lch,
    channels: [
        linear("l", PERCENT_SCALE, 0.0, PERCENT_SCALE),
        linear("c", LCH_CHROMA_SCALE, 0.0, f32::INFINITY),
        HUE_CHANNEL,
    ],
    number_scale: 1.0,
    legacy: false,
    commas: false,
};
const OKLAB: Notation = Notation {
    space: ColorSpace::Oklab,
    channels: [
        linear("l", 1.0, 0.0, 1.0),
        unbounded("a", OKLAB_OPPONENT_SCALE),
        unbounded("b", OKLAB_OPPONENT_SCALE),
    ],
    number_scale: 1.0,
    legacy: false,
    commas: false,
};
const OKLCH: Notation = Notation {
    space: ColorSpace::Oklch,
    channels: [
        linear("l", 1.0, 0.0, 1.0),
        linear("c", OKLCH_CHROMA_SCALE, 0.0, f32::INFINITY),
        HUE_CHANNEL,
    ],
    number_scale: 1.0,
    legacy: false,
    commas: false,
};
const RGB_PARAMS: [Channel; 3] = [
    unbounded("r", 1.0),
    unbounded("g", 1.0),
    unbounded("b", 1.0),
];
const XYZ_PARAMS: [Channel; 3] = [
    unbounded("x", 1.0),
    unbounded("y", 1.0),
    unbounded("z", 1.0),
];

#[derive(Debug)]
enum ColorError {
    Invalid,
    Depth,
    Math(MathError),
}
impl From<()> for ColorError {
    fn from((): ()) -> Self {
        Self::Invalid
    }
}
type Outcome<'i, T> = Result<T, ParseError<'i, ColorError>>;

/// A parsed argument before conversion to reference units.
#[derive(Clone, Copy)]
enum Value {
    Missing,
    Number(f32),
    Percentage(f32),
    Angle(f32),
}

/// Channel keywords of an origin colour and their values, `None` when missing.
type Channels = Vec<(&'static str, Option<f32>)>;

struct ColorParser<'r> {
    relative: &'r Relative<'r>,
    current: &'r AbsoluteColor,
    dependent: Cell<bool>,
    depth: Cell<usize>,
}

/// A colour and whether it depends on the element through `currentcolor`, the colour
/// scheme or a math function, which defers it to computed-value time.
pub(super) struct Parsed {
    pub color: AbsoluteColor,
    pub dependent: bool,
}

/// One complete colour value; `current` stands for `currentcolor`.
pub(super) fn parse(
    value: &str,
    relative: &Relative<'_>,
    current: &AbsoluteColor,
) -> Result<Parsed, StyleError> {
    let mut input = ParserInput::new(value);
    let mut input = Parser::new(&mut input);
    let parser = ColorParser {
        relative,
        current,
        dependent: Cell::new(false),
        depth: Cell::new(0),
    };
    let color = parser
        .color(&mut input)
        .and_then(|color| {
            input.expect_exhausted()?;
            Ok(color)
        })
        .map_err(|error| match error.kind {
            ParseErrorKind::Custom(ColorError::Math(MathError::Resolution(error))) => error,
            kind => StyleError::Invalid(format!("invalid colour {value}: {kind:?}")),
        })?;
    Ok(Parsed {
        color,
        dependent: parser.dependent.get(),
    })
}

/// Computes a matched `<color>`; `current` is the element's computed `color`.
pub(super) fn absolute(
    value: &Component,
    relative: &Relative<'_>,
    current: &AbsoluteColor,
) -> Result<AbsoluteColor, StyleError> {
    match value.unwrap() {
        Component::Color(color) => Ok(*color),
        Component::MathColor(source) => Ok(parse(source, relative, current)?.color),
        other => match other.keyword() {
            Some(keyword) => Ok(parse(keyword.name(), relative, current)?.color),
            None => Err(StyleError::Unsupported(format!("colour {value}"))),
        },
    }
}

/// The used sRGB colour of a matched `<color>`.
pub(super) fn color(
    value: &Component,
    relative: &Relative<'_>,
    current: &AbsoluteColor,
) -> Result<Color, StyleError> {
    absolute(value, relative, current).map(|color| color.used())
}

/// `<color-interpolation-method>` from the words after `in` (CSS Color 4 §13.2); custom
/// spaces need an `@color-profile`, which the pinned grammar lacks, so they are invalid.
pub(super) fn interpolation_method(words: &[&str]) -> Option<Interpolation> {
    let (name, hue) = match words {
        [name] => (*name, None),
        [name, method, keyword] if keyword.eq_ignore_ascii_case(HUE) => (*name, Some(*method)),
        _ => return None,
    };
    let space = lookup(&PREDEFINED_SPACES, name).or_else(|| lookup(&INTERPOLATION_SPACES, name))?;
    let hue = match hue {
        None => HueInterpolation::Shorter,
        Some(_) if space.hue_index().is_none() => return None,
        Some(method) => lookup(&HUE_METHODS, method)?,
    };
    Some(Interpolation { space, hue })
}

/// The `Canvas` system colour in `scheme`.
pub(super) fn canvas(scheme: ColorScheme) -> Color {
    system_color(CANVAS, scheme)
        .expect("Canvas is a system colour")
        .used()
}

/// The `AccentColor` system colour in `scheme`.
pub(super) fn accent(scheme: ColorScheme) -> Color {
    system_color(ACCENT_COLOR, scheme)
        .expect("AccentColor is a system colour")
        .used()
}

/// A system colour in `scheme`, with deprecated names mapped (CSS Color 4 §6.2, Appendix A).
fn system_color(name: &str, scheme: ColorScheme) -> Option<AbsoluteColor> {
    let name = lookup(&DEPRECATED_SYSTEM_COLORS, name).unwrap_or(name);
    let (_, light, dark) = SYSTEM_COLORS
        .iter()
        .find(|(candidate, ..)| name.eq_ignore_ascii_case(candidate))?;
    let rgb = match scheme {
        ColorScheme::Light => *light,
        ColorScheme::Dark => *dark,
    };
    Some(AbsoluteColor::from_rgb8(rgb, 1.0))
}

/// `AccentColor` takes an author `accent-color`, pre-composed over `Canvas` (CSS Color 4
/// §6.2, CSS UI 4 §7.1), and `AccentColorText` the more contrasting of black and white on it.
fn author_accent(name: &str, accent: AbsoluteColor, scheme: ColorScheme) -> Option<AbsoluteColor> {
    let text = name.eq_ignore_ascii_case(ACCENT_COLOR_TEXT);
    if !text && !name.eq_ignore_ascii_case(ACCENT_COLOR) {
        return None;
    }
    let (accent, canvas) = (accent.used(), canvas(scheme));
    let rgb = std::array::from_fn(|index| {
        (f32::from(accent.rgb[index]) * accent.alpha
            + f32::from(canvas.rgb[index]) * (OPAQUE - accent.alpha))
            .round() as u8
    });
    let composed = AbsoluteColor::from_rgb8(rgb, OPAQUE);
    if !text {
        return Some(composed);
    }
    // WCAG 2 contrast ratios against white and black, whose luminances are 1 and 0.
    let luminance = composed.convert(ColorSpace::XyzD65).components[1].unwrap_or_default();
    let against_white = (OPAQUE + LUMINANCE_FLARE) / (luminance + LUMINANCE_FLARE);
    let against_black = (luminance + LUMINANCE_FLARE) / LUMINANCE_FLARE;
    Some(AbsoluteColor::from_rgb8(
        if against_white >= against_black {
            [u8::MAX; 3]
        } else {
            [0; 3]
        },
        OPAQUE,
    ))
}

fn lookup<T: Copy>(table: &[(&str, T)], name: &str) -> Option<T> {
    table
        .iter()
        .find(|(candidate, _)| name.eq_ignore_ascii_case(candidate))
        .map(|(_, value)| *value)
}

/// The origin colour's components in `space` as channel keywords, carrying missing
/// analogous components forward (CSS Color 5 §4.1).
fn origin_channels(
    origin: &AbsoluteColor,
    space: ColorSpace,
    channels: &[Channel; 3],
    number_scale: f32,
) -> (AbsoluteColor, Channels) {
    let converted = origin.convert_carrying(space);
    let mut keywords: Channels = channels
        .iter()
        .zip(converted.components)
        .map(|(channel, value)| {
            (
                channel.name,
                value.map(|value| {
                    if channel.hue {
                        value
                    } else {
                        value / number_scale
                    }
                }),
            )
        })
        .collect();
    keywords.push((ALPHA, converted.alpha));
    (converted, keywords)
}

impl ColorParser<'_> {
    fn color<'i>(&self, input: &mut Parser<'i, '_>) -> Outcome<'i, AbsoluteColor> {
        if self.depth.get() >= MAX_COMPONENT_DEPTH {
            return Err(input.new_custom_error(ColorError::Depth));
        }
        self.depth.set(self.depth.get() + 1);
        let result = self.color_term(input);
        self.depth.set(self.depth.get() - 1);
        result
    }

    fn color_term<'i>(&self, input: &mut Parser<'i, '_>) -> Outcome<'i, AbsoluteColor> {
        let location = input.current_source_location();
        let token = input.next()?.clone();
        match &token {
            Token::Hash(value) | Token::IDHash(value) => {
                cssparser::color::parse_hash_color(value.as_bytes())
                    .map(|(red, green, blue, alpha)| {
                        AbsoluteColor::from_rgb8([red, green, blue], alpha)
                    })
                    .map_err(|()| location.new_unexpected_token_error(token.clone()))
            }
            Token::Ident(name) => {
                if name.eq_ignore_ascii_case(CURRENT_COLOR) {
                    self.dependent.set(true);
                    return Ok(*self.current);
                }
                if name.eq_ignore_ascii_case(TRANSPARENT) {
                    return Ok(AbsoluteColor::TRANSPARENT);
                }
                if let Ok(rgb) = cssparser::color::parse_named_color(name) {
                    return Ok(AbsoluteColor::from_rgb8(rgb.into(), 1.0));
                }
                let color = self
                    .relative
                    .accent_color
                    .and_then(|accent| author_accent(name, accent, self.relative.color_scheme))
                    .or_else(|| system_color(name, self.relative.color_scheme))
                    .ok_or_else(|| location.new_unexpected_token_error(token.clone()))?;
                self.dependent.set(true);
                Ok(color)
            }
            Token::Function(name) => {
                let name = name.to_ascii_lowercase();
                input.parse_nested_block(|arguments| {
                    let color = match name.as_str() {
                        "rgb" | "rgba" => self.notation(arguments, &RGB),
                        "hsl" | "hsla" => self.notation(arguments, &HSL),
                        "hwb" => self.notation(arguments, &HWB),
                        "lab" => self.notation(arguments, &LAB),
                        "lch" => self.notation(arguments, &LCH),
                        "oklab" => self.notation(arguments, &OKLAB),
                        "oklch" => self.notation(arguments, &OKLCH),
                        "color" => self.color_function(arguments),
                        "color-mix" => self.mix(arguments),
                        "light-dark" => self.light_dark(arguments),
                        "alpha" => self.alpha_function(arguments),
                        _ => Err(arguments.new_custom_error(ColorError::Invalid)),
                    }?;
                    arguments.expect_exhausted()?;
                    Ok(color)
                })
            }
            _ => Err(location.new_unexpected_token_error(token.clone())),
        }
    }

    /// `from <color>`, if present.
    fn origin<'i>(&self, input: &mut Parser<'i, '_>) -> Outcome<'i, Option<AbsoluteColor>> {
        if input
            .try_parse(|input| input.expect_ident_matching(FROM))
            .is_err()
        {
            return Ok(None);
        }
        self.color(input).map(Some)
    }

    /// One argument: `none`, a channel keyword, a number, a percentage, an angle for hues,
    /// or a math function over the channel keywords.
    fn value<'i>(
        &self,
        input: &mut Parser<'i, '_>,
        hue: bool,
        channels: &Channels,
        none: bool,
    ) -> Outcome<'i, Value> {
        let location = input.current_source_location();
        let state = input.state();
        let token = input.next()?.clone();
        Ok(match &token {
            Token::Ident(name) if none && name.eq_ignore_ascii_case(NONE) => Value::Missing,
            Token::Ident(name) => {
                let (_, value) = channels
                    .iter()
                    .find(|(channel, _)| name.eq_ignore_ascii_case(channel))
                    .ok_or_else(|| location.new_unexpected_token_error(token.clone()))?;
                value.map_or(Value::Missing, Value::Number)
            }
            Token::Number { value, .. } => Value::Number(*value),
            Token::Percentage { unit_value, .. } if !hue => Value::Percentage(*unit_value),
            Token::Dimension { value, unit, .. } if hue => {
                match crate::style::units::dimension(unit) {
                    Some(Dimension::Angle(degrees)) => Value::Angle(value * degrees),
                    _ => return Err(location.new_unexpected_token_error(token.clone())),
                }
            }
            Token::Function(_) => {
                input.reset(&state);
                return self.math(input, hue, channels);
            }
            _ => return Err(location.new_unexpected_token_error(token.clone())),
        })
    }

    fn math<'i>(
        &self,
        input: &mut Parser<'i, '_>,
        hue: bool,
        channels: &Channels,
    ) -> Outcome<'i, Value> {
        let numbers: Vec<(&str, f32)> = channels
            .iter()
            .map(|(name, value)| (*name, value.unwrap_or(0.0)))
            .collect();
        let (kinds, percentage) = if hue {
            ([Kind::Number, Kind::Angle], None)
        } else {
            ([Kind::Number, Kind::Percentage], Some(Kind::Percentage))
        };
        let context = Context {
            relative: self.relative,
            percentage,
            channels: &numbers,
        };
        let math = math_function(input, context)
            .ok_or_else(|| input.new_custom_error(ColorError::Invalid))?
            .and_then(|math| math.expect(&kinds))
            .map_err(|error| input.new_custom_error(ColorError::Math(error)))?;
        self.dependent.set(true);
        let value = math.resolve(0.0, Range::ALL);
        Ok(match math.kind() {
            Some(Kind::Percentage) => Value::Percentage(value),
            Some(Kind::Angle) => Value::Angle(value),
            _ => Value::Number(value),
        })
    }

    /// `/ <alpha-value> | none`, if present; numbers and percentages are fractions.
    fn alpha<'i>(
        &self,
        input: &mut Parser<'i, '_>,
        channels: &Channels,
        default: Option<f32>,
    ) -> Outcome<'i, Option<f32>> {
        if input.is_exhausted() {
            return Ok(default);
        }
        input.expect_delim(ALPHA_SEPARATOR)?;
        self.alpha_value(input, channels, true)
    }
    fn alpha_value<'i>(
        &self,
        input: &mut Parser<'i, '_>,
        channels: &Channels,
        none: bool,
    ) -> Outcome<'i, Option<f32>> {
        Ok(match self.value(input, false, channels, none)? {
            Value::Missing => None,
            Value::Number(value) | Value::Percentage(value) | Value::Angle(value) => {
                Some(censor(value).clamp(0.0, 1.0))
            }
        })
    }

    /// The modern and, for sRGB functions, legacy syntaxes of a colour function, absolute or
    /// relative (CSS Color 4 §4.1, CSS Color 5 §4).
    fn notation<'i>(
        &self,
        input: &mut Parser<'i, '_>,
        notation: &Notation,
    ) -> Outcome<'i, AbsoluteColor> {
        let origin = self.origin(input)?;
        let (origin, channels) = match &origin {
            Some(origin) => {
                let (converted, channels) = origin_channels(
                    origin,
                    notation.space,
                    &notation.channels,
                    notation.number_scale,
                );
                (Some(converted), channels)
            }
            None => (None, Vec::new()),
        };
        let first = self.value(input, notation.channels[0].hue, &channels, true)?;
        let legacy = notation.commas
            && origin.is_none()
            && !matches!(first, Value::Missing)
            && input.try_parse(Parser::expect_comma).is_ok();
        let mut values = [first, Value::Missing, Value::Missing];
        let alpha = if legacy {
            for (index, value) in values.iter_mut().enumerate().skip(1) {
                if index > 1 {
                    input.expect_comma()?;
                }
                *value = self.value(input, false, &channels, false)?;
                // Legacy rgb() repeats the first argument's type; legacy hsl() takes percentages.
                let valid = match (notation.space, first, *value) {
                    (ColorSpace::Srgb, Value::Number(_), Value::Number(_)) => true,
                    (ColorSpace::Srgb, Value::Number(_), _) => false,
                    (_, _, value) => matches!(value, Value::Percentage(_)),
                };
                if !valid {
                    return Err(input.new_custom_error(ColorError::Invalid));
                }
            }
            if input.is_exhausted() {
                Some(OPAQUE)
            } else {
                input.expect_comma()?;
                self.alpha_value(input, &channels, false)?
            }
        } else {
            for (index, value) in values.iter_mut().enumerate().skip(1) {
                *value = self.value(input, notation.channels[index].hue, &channels, true)?;
            }
            self.alpha(
                input,
                &channels,
                origin.map_or(Some(OPAQUE), |origin| origin.alpha),
            )?
        };
        let absolute = origin.is_none();
        let components = std::array::from_fn(|index| {
            reference(
                values[index],
                &notation.channels[index],
                notation.number_scale,
                absolute,
            )
        });
        Ok(AbsoluteColor {
            space: notation.space,
            components,
            alpha,
            legacy: notation.legacy && absolute,
        })
    }

    /// `color()` with a predefined space; custom spaces would need an `@color-profile`, so
    /// they name no colour space and are invalid colours, whose used value is opaque black
    /// (CSS Color 4 §10.1, CSS Color 5 §5.1).
    fn color_function<'i>(&self, input: &mut Parser<'i, '_>) -> Outcome<'i, AbsoluteColor> {
        let origin = self.origin(input)?;
        let location = input.current_source_location();
        let name = input.expect_ident()?.clone();
        if name.starts_with(CUSTOM_SPACE_PREFIX) {
            if origin.is_some() {
                return Err(input.new_custom_error(ColorError::Invalid));
            }
            let mut parameters = 0;
            let mut alpha = false;
            while !input.is_exhausted() {
                if input
                    .try_parse(|input| input.expect_delim(ALPHA_SEPARATOR))
                    .is_ok()
                {
                    alpha = true;
                    break;
                }
                self.value(input, false, &Vec::new(), true)?;
                parameters += 1;
            }
            if parameters == 0 {
                return Err(input.new_custom_error(ColorError::Invalid));
            }
            if alpha {
                self.alpha_value(input, &Vec::new(), true)?;
            }
            return Ok(AbsoluteColor {
                legacy: false,
                ..AbsoluteColor::BLACK
            });
        }
        let space = lookup(&PREDEFINED_SPACES, &name)
            .ok_or_else(|| location.new_unexpected_token_error(Token::Ident(name.clone())))?;
        let parameters = if matches!(space, ColorSpace::XyzD50 | ColorSpace::XyzD65) {
            &XYZ_PARAMS
        } else {
            &RGB_PARAMS
        };
        let (origin, channels) = match &origin {
            Some(origin) => {
                let (converted, channels) = origin_channels(origin, space, parameters, 1.0);
                (Some(converted), channels)
            }
            None => (None, Vec::new()),
        };
        let mut values = [Value::Missing; 3];
        for value in &mut values {
            *value = self.value(input, false, &channels, true)?;
        }
        let alpha = self.alpha(
            input,
            &channels,
            origin.map_or(Some(OPAQUE), |origin| origin.alpha),
        )?;
        Ok(AbsoluteColor {
            space,
            components: std::array::from_fn(|index| {
                reference(values[index], &parameters[index], 1.0, false)
            }),
            alpha,
            legacy: false,
        })
    }

    /// `alpha(from <color> / <alpha-value>)` keeps the origin's space and components
    /// (CSS Color 5 §4.10).
    fn alpha_function<'i>(&self, input: &mut Parser<'i, '_>) -> Outcome<'i, AbsoluteColor> {
        let origin = self
            .origin(input)?
            .ok_or_else(|| input.new_custom_error(ColorError::Invalid))?;
        let channels = vec![(ALPHA, origin.alpha)];
        Ok(AbsoluteColor {
            alpha: self.alpha(input, &channels, origin.alpha)?,
            legacy: false,
            ..origin
        })
    }

    /// `light-dark()` computes to the colour for the element's used colour scheme
    /// (CSS Color 5 §7).
    fn light_dark<'i>(&self, input: &mut Parser<'i, '_>) -> Outcome<'i, AbsoluteColor> {
        let light = self.color(input)?;
        input.expect_comma()?;
        let dark = self.color(input)?;
        self.dependent.set(true);
        Ok(match self.relative.color_scheme {
            ColorScheme::Light => light,
            ColorScheme::Dark => dark,
        })
    }

    fn method<'i>(input: &mut Parser<'i, '_>) -> Outcome<'i, Interpolation> {
        input.expect_ident_matching(IN)?;
        let mut words = vec![input.expect_ident()?.clone()];
        if let Ok(method) = input.try_parse(|input| {
            let method = input.expect_ident()?.clone();
            input.expect_ident_matching(HUE)?;
            Ok::<_, ParseError<'i, ColorError>>(method)
        }) {
            words.push(method);
            words.push(HUE.into());
        }
        let words: Vec<&str> = words.iter().map(AsRef::as_ref).collect();
        interpolation_method(&words).ok_or_else(|| input.new_custom_error(ColorError::Invalid))
    }

    /// `<percentage [0,100]>` as a fraction.
    fn mix_percentage<'i>(&self, input: &mut Parser<'i, '_>) -> Outcome<'i, f32> {
        let state = input.state();
        match input.next()?.clone() {
            Token::Percentage { unit_value, .. } if (0.0..=WHOLE).contains(&unit_value) => {
                Ok(unit_value)
            }
            Token::Function(_) => {
                input.reset(&state);
                match self.math(input, false, &Vec::new())? {
                    Value::Percentage(value) => Ok(censor(value).clamp(0.0, WHOLE)),
                    _ => Err(input.new_custom_error(ColorError::Invalid)),
                }
            }
            token => Err(input.new_unexpected_token_error(token)),
        }
    }

    fn mix_item<'i>(&self, input: &mut Parser<'i, '_>) -> Outcome<'i, MixItem> {
        let leading = input.try_parse(|input| self.mix_percentage(input)).ok();
        let color = self.color(input)?;
        let percentage = match leading {
            Some(percentage) => Some(percentage),
            None => input.try_parse(|input| self.mix_percentage(input)).ok(),
        };
        Ok(MixItem { color, percentage })
    }

    /// `color-mix()` (CSS Color 5 §3.3), defaulting to Oklab.
    fn mix<'i>(&self, input: &mut Parser<'i, '_>) -> Outcome<'i, AbsoluteColor> {
        let method = match input.try_parse(Self::method) {
            Ok(method) => {
                input.expect_comma()?;
                method
            }
            Err(_) => DEFAULT_MIX,
        };
        let items = input.parse_comma_separated(|input| self.mix_item(input))?;
        Ok(mix(method, &items))
    }
}

struct MixItem {
    color: AbsoluteColor,
    percentage: Option<f32>,
}

/// Normalizes mix percentages with forced normalization: omitted percentages share what the
/// specified ones leave, and the shortfall below 100% becomes the alpha multiplier's
/// complement (CSS Color 5 §3.2).
fn normalize(items: &[MixItem]) -> (Vec<f32>, f32) {
    let specified: f32 = items.iter().filter_map(|item| item.percentage).sum();
    let omitted = items
        .iter()
        .filter(|item| item.percentage.is_none())
        .count();
    let share = if omitted == 0 {
        0.0
    } else {
        ((WHOLE - specified) / omitted as f32).max(0.0)
    };
    let mut percentages: Vec<f32> = items
        .iter()
        .map(|item| item.percentage.unwrap_or(share))
        .collect();
    let total: f32 = percentages.iter().sum();
    let leftover = (WHOLE - total).max(0.0);
    if total > 0.0 {
        for percentage in &mut percentages {
            *percentage /= total;
        }
    }
    (percentages, leftover)
}

fn mix(method: Interpolation, items: &[MixItem]) -> AbsoluteColor {
    let (percentages, leftover) = normalize(items);
    let mut stack: Vec<(AbsoluteColor, f32)> = items
        .iter()
        .zip(percentages)
        .rev()
        .map(|(item, percentage)| (item.color, percentage))
        .collect();
    let mut color = if let [(only, _)] = stack.as_slice() {
        only.convert_carrying(method.space)
    } else {
        while stack.len() >= 2 {
            let (a, a_percentage) = stack.pop().expect("two items remain");
            let (b, b_percentage) = stack.pop().expect("two items remain");
            let combined = a_percentage + b_percentage;
            let progress = if combined > 0.0 {
                b_percentage / combined
            } else {
                EVEN_MIX
            };
            stack.push((method.interpolate(&a, &b, progress), combined));
        }
        stack[0].0
    };
    color.alpha = color.alpha.map(|alpha| alpha * (WHOLE - leftover));
    color.legacy = false;
    color
}

/// NaN censors to zero and infinities to the largest finite values (CSS Values 4 §10.9).
fn censor(value: f32) -> f32 {
    if value.is_nan() {
        0.0
    } else {
        value.clamp(f32::MIN, f32::MAX)
    }
}

/// An argument in the space's reference units, clamped for absolute colours.
fn reference(value: Value, channel: &Channel, number_scale: f32, absolute: bool) -> Option<f32> {
    let value = match value {
        Value::Missing => return None,
        Value::Number(value) | Value::Angle(value) if channel.hue => {
            return Some(normalize_hue(value));
        }
        Value::Number(value) | Value::Angle(value) => value * number_scale,
        Value::Percentage(fraction) => fraction * channel.percent,
    };
    let value = censor(value);
    Some(if absolute {
        value.clamp(channel.min, channel.max)
    } else {
        value
    })
}
