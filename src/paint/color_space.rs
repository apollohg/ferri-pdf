//! CSS Color 4 colour spaces: conversion through CIE XYZ (§11), CSS gamut mapping to the sRGB
//! output (§14.2.2), and interpolation with missing components, premultiplied alpha and hue
//! fix-up (§13). Matrices and transfer functions are the §19 sample code.
use crate::paint::Color;
use serde::Serialize;

type Vector = [f64; 3];
type Matrix = [[f64; 3]; 3];

const FULL_TURN_DEG: f64 = 360.0;
const HALF_TURN_DEG: f64 = 180.0;
const PERCENT: f64 = 100.0;
const SRGB_LINEAR_LIMIT: f64 = 0.040_45;
const SRGB_GAMMA_LIMIT: f64 = 0.003_130_8;
const SRGB_LINEAR_SLOPE: f64 = 12.92;
const SRGB_OFFSET: f64 = 0.055;
const SRGB_GAMMA: f64 = 2.4;
const PROPHOTO_LINEAR_LIMIT: f64 = 16.0 / 512.0;
const PROPHOTO_GAMMA_LIMIT: f64 = 1.0 / 512.0;
const PROPHOTO_LINEAR_SLOPE: f64 = 16.0;
const PROPHOTO_GAMMA: f64 = 1.8;
const A98_GAMMA: f64 = 563.0 / 256.0;
const REC2020_GAMMA: f64 = 2.4;
const LAB_EPSILON: f64 = 216.0 / 24389.0;
const LAB_KAPPA: f64 = 24389.0 / 27.0;
const LAB_LIGHTNESS_SCALE: f64 = 116.0;
const LAB_LIGHTNESS_OFFSET: f64 = 16.0;
const LAB_A_SCALE: f64 = 500.0;
const LAB_B_SCALE: f64 = 200.0;
/// Powerless-hue thresholds (§7, §8, §9.3, §9.4), in each space's reference units.
const HSL_SATURATION_EPSILON: f64 = 0.001;
const HWB_ACHROMATIC_EPSILON: f64 = 0.001;
const LCH_CHROMA_EPSILON: f64 = 0.0015;
const OKLCH_CHROMA_EPSILON: f64 = 0.000_004;
/// HSL hue sectors of 30 degrees over a 12-sector circle (§7.1).
const HSL_SECTOR_DEG: f64 = 30.0;
const HSL_SECTORS: f64 = 12.0;
/// §14.2.2 binary search: one just-noticeable ΔEOK and the chroma search precision.
const GAMUT_JND: f64 = 0.02;
const GAMUT_EPSILON: f64 = 0.0001;

const D50: Vector = [0.3457 / 0.3585, 1.0, (1.0 - 0.3457 - 0.3585) / 0.3585];
const LIN_SRGB_TO_XYZ: Matrix = [
    [
        506_752.0 / 1_228_815.0,
        87_881.0 / 245_763.0,
        12_673.0 / 70_218.0,
    ],
    [
        87_098.0 / 409_605.0,
        175_762.0 / 245_763.0,
        12_673.0 / 175_545.0,
    ],
    [
        7_918.0 / 409_605.0,
        87_881.0 / 737_289.0,
        1_001_167.0 / 1_053_270.0,
    ],
];
const XYZ_TO_LIN_SRGB: Matrix = [
    [12_831.0 / 3_959.0, -329.0 / 214.0, -1_974.0 / 3_959.0],
    [
        -851_781.0 / 878_810.0,
        1_648_619.0 / 878_810.0,
        36_519.0 / 878_810.0,
    ],
    [705.0 / 12_673.0, -2_585.0 / 12_673.0, 705.0 / 667.0],
];
const LIN_P3_TO_XYZ: Matrix = [
    [
        608_311.0 / 1_250_200.0,
        189_793.0 / 714_400.0,
        198_249.0 / 1_000_160.0,
    ],
    [
        35_783.0 / 156_275.0,
        247_089.0 / 357_200.0,
        198_249.0 / 2_500_400.0,
    ],
    [0.0, 32_229.0 / 714_400.0, 5_220_557.0 / 5_000_800.0],
];
const XYZ_TO_LIN_P3: Matrix = [
    [
        446_124.0 / 178_915.0,
        -333_277.0 / 357_830.0,
        -72_051.0 / 178_915.0,
    ],
    [-14_852.0 / 17_905.0, 63_121.0 / 35_810.0, 423.0 / 17_905.0],
    [
        11_844.0 / 330_415.0,
        -50_337.0 / 660_830.0,
        316_169.0 / 330_415.0,
    ],
];
const LIN_PROPHOTO_TO_XYZ_D50: Matrix = [
    [
        0.797_766_644_900_642_3,
        0.135_181_297_400_533_08,
        0.031_347_734_128_392_2,
    ],
    [
        0.288_074_828_819_401_3,
        0.711_835_234_241_873,
        0.000_089_936_938_725_64,
    ],
    [0.0, 0.0, 0.825_104_602_510_460_2],
];
const XYZ_D50_TO_LIN_PROPHOTO: Matrix = [
    [
        1.345_786_881_647_158_3,
        -0.255_572_087_379_794_64,
        -0.051_101_864_975_545_26,
    ],
    [
        -0.544_630_705_124_901_9,
        1.508_247_742_845_146_8,
        0.020_527_447_436_421_39,
    ],
    [0.0, 0.0, 1.211_967_545_638_945_2],
];
const LIN_A98_TO_XYZ: Matrix = [
    [
        573_536.0 / 994_567.0,
        263_643.0 / 1_420_810.0,
        187_206.0 / 994_567.0,
    ],
    [
        591_459.0 / 1_989_134.0,
        6_239_551.0 / 9_945_670.0,
        374_412.0 / 4_972_835.0,
    ],
    [
        53_769.0 / 1_989_134.0,
        351_524.0 / 4_972_835.0,
        4_929_758.0 / 4_972_835.0,
    ],
];
const XYZ_TO_LIN_A98: Matrix = [
    [
        1_829_569.0 / 896_150.0,
        -506_331.0 / 896_150.0,
        -308_931.0 / 896_150.0,
    ],
    [
        -851_781.0 / 878_810.0,
        1_648_619.0 / 878_810.0,
        36_519.0 / 878_810.0,
    ],
    [
        16_779.0 / 1_248_040.0,
        -147_721.0 / 1_248_040.0,
        1_266_979.0 / 1_248_040.0,
    ],
];
const LIN_REC2020_TO_XYZ: Matrix = [
    [
        63_426_534.0 / 99_577_255.0,
        20_160_776.0 / 139_408_157.0,
        47_086_771.0 / 278_816_314.0,
    ],
    [
        26_158_966.0 / 99_577_255.0,
        472_592_308.0 / 697_040_785.0,
        8_267_143.0 / 139_408_157.0,
    ],
    [
        0.0,
        19_567_812.0 / 697_040_785.0,
        295_819_943.0 / 278_816_314.0,
    ],
];
const XYZ_TO_LIN_REC2020: Matrix = [
    [
        30_757_411.0 / 17_917_100.0,
        -6_372_589.0 / 17_917_100.0,
        -4_539_589.0 / 17_917_100.0,
    ],
    [
        -19_765_991.0 / 29_648_200.0,
        47_925_759.0 / 29_648_200.0,
        467_509.0 / 29_648_200.0,
    ],
    [
        792_561.0 / 44_930_125.0,
        -1_921_689.0 / 44_930_125.0,
        42_328_811.0 / 44_930_125.0,
    ],
];
const D65_TO_D50: Matrix = [
    [
        1.047_929_792_544_996_9,
        0.022_946_870_601_609_652,
        -0.050_192_266_289_205_24,
    ],
    [
        0.029_627_808_770_055_99,
        0.990_434_426_753_879_9,
        -0.017_073_799_063_418_826,
    ],
    [
        -0.009_243_040_646_204_504,
        0.015_055_191_490_298_152,
        0.751_874_281_428_137_1,
    ],
];
const D50_TO_D65: Matrix = [
    [
        0.955_473_421_488_075,
        -0.023_098_454_948_764_71,
        0.063_259_243_200_570_72,
    ],
    [
        -0.028_369_709_333_863_7,
        1.009_995_398_081_304_1,
        0.021_041_441_191_917_323,
    ],
    [
        0.012_314_014_864_481_998,
        -0.020_507_649_298_898_964,
        1.330_365_926_242_124,
    ],
];
const XYZ_TO_LMS: Matrix = [
    [
        0.819_022_437_996_703,
        0.361_906_260_052_890_4,
        -0.128_873_781_520_987_9,
    ],
    [
        0.032_983_653_932_388_5,
        0.929_286_861_586_343_4,
        0.036_144_666_350_642_4,
    ],
    [
        0.048_177_189_359_624_2,
        0.264_239_531_752_730_8,
        0.633_547_828_469_430_9,
    ],
];
const LMS_TO_OKLAB: Matrix = [
    [
        0.210_454_268_309_314,
        0.793_617_774_702_305_4,
        -0.004_072_043_011_619_3,
    ],
    [
        1.977_998_532_431_168_4,
        -2.428_592_242_048_58,
        0.450_593_709_617_411,
    ],
    [
        0.025_904_042_465_547_8,
        0.782_771_712_457_529_6,
        -0.808_675_754_923_077_4,
    ],
];
const LMS_TO_XYZ: Matrix = [
    [
        1.226_879_875_845_924_3,
        -0.557_814_994_460_217_1,
        0.281_391_045_665_964_7,
    ],
    [
        -0.040_575_745_214_800_8,
        1.112_286_803_280_317,
        -0.071_711_058_065_516_4,
    ],
    [
        -0.076_372_936_674_660_1,
        -0.421_493_332_402_243_2,
        1.586_924_019_836_781_6,
    ],
];
const OKLAB_TO_LMS: Matrix = [
    [1.0, 0.396_337_777_376_174_9, 0.215_803_757_309_913_6],
    [1.0, -0.105_561_345_815_658_6, -0.063_854_172_825_813_3],
    [1.0, -0.089_484_177_529_811_9, -1.291_485_548_019_409_2],
];

/// The colour spaces of CSS Color 4 functions and `color()`; `xyz` is stored as `XyzD65`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum ColorSpace {
    Srgb,
    SrgbLinear,
    DisplayP3,
    DisplayP3Linear,
    A98Rgb,
    ProphotoRgb,
    Rec2020,
    XyzD50,
    XyzD65,
    Lab,
    Lch,
    Oklab,
    Oklch,
    Hsl,
    Hwb,
}

/// §13.3 analogous component categories; HWB whiteness and blackness have none.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Category {
    Red,
    Green,
    Blue,
    Lightness,
    Colorfulness,
    Hue,
    OpponentA,
    OpponentB,
}

impl ColorSpace {
    /// The hue's component index in cylindrical polar spaces.
    pub fn hue_index(self) -> Option<usize> {
        match self {
            Self::Hsl | Self::Hwb => Some(0),
            Self::Lch | Self::Oklch => Some(2),
            _ => None,
        }
    }
    fn categories(self) -> [Option<Category>; 3] {
        use Category::{Blue, Colorfulness, Green, Hue, Lightness, OpponentA, OpponentB, Red};
        match self {
            Self::Lab | Self::Oklab => [Some(Lightness), Some(OpponentA), Some(OpponentB)],
            Self::Lch | Self::Oklch => [Some(Lightness), Some(Colorfulness), Some(Hue)],
            Self::Hsl => [Some(Hue), Some(Colorfulness), Some(Lightness)],
            Self::Hwb => [Some(Hue), None, None],
            _ => [Some(Red), Some(Green), Some(Blue)],
        }
    }
    /// The rectangular orthogonal space a polar representation is defined over.
    fn rectangular(self) -> Self {
        match self {
            Self::Lch => Self::Lab,
            Self::Oklch => Self::Oklab,
            Self::Hsl | Self::Hwb => Self::Srgb,
            other => other,
        }
    }
}

/// A colour with a colorimetric interpretation (CSS Color 5 §2), in its own space and
/// reference units: RGB and XYZ components in `0..=1`, HSL and HWB percentages in `0..=100`,
/// hues in degrees. `None` is a missing component (`none`).
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct AbsoluteColor {
    pub space: ColorSpace,
    pub components: [Option<f32>; 3],
    pub alpha: Option<f32>,
    /// Hex, named, `rgb()`, `hsl()` and `hwb()` colours, which interpolate in gamma-encoded
    /// sRGB unless a method is specified (CSS Color 4 §13.2).
    pub legacy: bool,
}

impl AbsoluteColor {
    pub const BLACK: Self = Self::srgb([0.0; 3], 1.0);
    pub const TRANSPARENT: Self = Self::srgb([0.0; 3], 0.0);

    pub const fn srgb([red, green, blue]: [f32; 3], alpha: f32) -> Self {
        Self {
            space: ColorSpace::Srgb,
            components: [Some(red), Some(green), Some(blue)],
            alpha: Some(alpha),
            legacy: true,
        }
    }
    /// 8-bit sRGB, as in hex and named colours.
    pub fn from_rgb8([red, green, blue]: [u8; 3], alpha: f32) -> Self {
        let unit = |channel: u8| f32::from(channel) / f32::from(u8::MAX);
        Self::srgb([unit(red), unit(green), unit(blue)], alpha)
    }

    /// Converts with missing components treated as zero, setting powerless hues of the
    /// result to missing (CSS Color 4 §4.4, §11.2).
    #[must_use]
    pub fn convert(&self, space: ColorSpace) -> Self {
        if self.space == space {
            return *self;
        }
        let values = self.components.map(|value| f64::from(value.unwrap_or(0.0)));
        let source = self.space.rectangular();
        let target = space.rectangular();
        let rectangular = to_rectangular(self.space, values);
        let converted = if source == target {
            rectangular
        } else {
            from_xyz_d65(target, to_xyz_d65(source, rectangular))
        };
        Self {
            space,
            components: to_polar(space, converted).map(|value| value.map(|value| value as f32)),
            alpha: self.alpha,
            legacy: false,
        }
    }

    /// Converts for interpolation or relative colour syntax, carrying missing analogous
    /// components and analogous sets forward (CSS Color 4 §13.3).
    #[must_use]
    pub fn convert_carrying(&self, space: ColorSpace) -> Self {
        let mut converted = self.convert(space);
        if self.space == space {
            return converted;
        }
        let source = self.space.categories();
        let target = space.categories();
        let mut source_matched = [false; 3];
        let mut target_matched = [false; 3];
        for (index, category) in target.iter().enumerate() {
            let Some(found) = category.and_then(|category| {
                source
                    .iter()
                    .position(|candidate| *candidate == Some(category))
            }) else {
                continue;
            };
            target_matched[index] = true;
            source_matched[found] = true;
            if self.components[found].is_none() {
                converted.components[index] = None;
            }
        }
        let mut remaining = (0..3).filter(|&index| !source_matched[index]).peekable();
        if remaining.peek().is_some() && remaining.all(|index| self.components[index].is_none()) {
            for (component, matched) in converted.components.iter_mut().zip(target_matched) {
                if !matched {
                    *component = None;
                }
            }
        }
        converted
    }

    /// The used sRGB colour: CSS gamut mapped and rounded to the 8-bit output precision.
    pub fn used(&self) -> Color {
        Color {
            rgb: quantize(self.mapped_srgb()),
            alpha: self.used_alpha(),
        }
    }
    /// Missing alpha behaves as zero (CSS Color 4 §4.4).
    pub fn used_alpha(&self) -> f32 {
        self.alpha.unwrap_or(0.0).clamp(0.0, 1.0)
    }

    /// Gamma-encoded sRGB within `0..=1`, by binary search gamut mapping with local MINDE
    /// (CSS Color 4 §14.2.2).
    pub fn mapped_srgb(&self) -> [f64; 3] {
        let srgb = self.convert(ColorSpace::Srgb).values();
        if in_unit_gamut(srgb) {
            return srgb;
        }
        let origin = self.convert(ColorSpace::Oklch).values();
        if origin[0] >= 1.0 {
            return clip(oklab_to_srgb([1.0, 0.0, 0.0]));
        }
        if origin[0] <= 0.0 {
            return clip(oklab_to_srgb([0.0; 3]));
        }
        let oklch_srgb = |oklch: Vector| oklab_to_srgb(lch_to_lab(oklch));
        let delta = |clipped: Vector, current: Vector| {
            delta_eok(srgb_to_oklab(clipped), lch_to_lab(current))
        };
        let mut current = origin;
        let mut clipped = clip(oklch_srgb(current));
        if delta(clipped, current) < GAMUT_JND {
            return clipped;
        }
        let mut min = 0.0;
        let mut max = origin[1];
        let mut min_in_gamut = true;
        while max - min > GAMUT_EPSILON {
            let chroma = f64::midpoint(min, max);
            current[1] = chroma;
            if min_in_gamut && in_unit_gamut(oklch_srgb(current)) {
                min = chroma;
                continue;
            }
            clipped = clip(oklch_srgb(current));
            let error = delta(clipped, current);
            if error < GAMUT_JND {
                if GAMUT_JND - error < GAMUT_EPSILON {
                    return clipped;
                }
                min_in_gamut = false;
                min = chroma;
            } else {
                max = chroma;
            }
        }
        clipped
    }

    fn values(&self) -> Vector {
        self.components.map(|value| f64::from(value.unwrap_or(0.0)))
    }
}

/// Percentages are fractions of the whole; an even mix weighs both colours equally.
const WHOLE: f32 = 1.0;
const EVEN_MIX: f32 = 0.5;

/// A `color-mix()` colour and its percentage, as a fraction (CSS Color 5 §3.1).
pub(crate) struct MixItem {
    pub color: AbsoluteColor,
    pub percentage: Option<f32>,
}

/// Mix percentages with the omitted ones sharing what the specified ones leave below 100%,
/// floored at 0% (CSS Color 5 §3.2, CSS Images 4 §2.6).
pub(crate) fn distribute_mix(percentages: &[Option<f32>]) -> Vec<f32> {
    let specified: f32 = percentages.iter().flatten().sum();
    let omitted = percentages.iter().filter(|value| value.is_none()).count();
    let share = if omitted == 0 {
        0.0
    } else {
        ((WHOLE - specified) / omitted as f32).max(0.0)
    };
    percentages
        .iter()
        .map(|value| value.unwrap_or(share))
        .collect()
}

/// Mix percentages scaled to sum to 100% when they sum past it or, under forced
/// normalization, to any positive total, with the leftover they leave below 100%.
pub(crate) fn normalize_mix(percentages: &[f32], forced: bool) -> (Vec<f32>, f32) {
    let total: f32 = percentages.iter().sum();
    let leftover = (WHOLE - total).max(0.0);
    let scale = if total > WHOLE || (forced && total > 0.0) {
        total.recip()
    } else {
        WHOLE
    };
    (
        percentages.iter().map(|value| value * scale).collect(),
        leftover,
    )
}

impl Interpolation {
    /// Mixes `items` as `color-mix()` does (CSS Color 5 §3.3).
    pub(crate) fn mix(self, items: &[MixItem]) -> AbsoluteColor {
        let percentages: Vec<_> = items.iter().map(|item| item.percentage).collect();
        let (percentages, leftover) = normalize_mix(&distribute_mix(&percentages), true);
        let mut stack: Vec<(AbsoluteColor, f32)> = items
            .iter()
            .zip(percentages)
            .rev()
            .map(|(item, percentage)| (item.color, percentage))
            .collect();
        let mut color = if let [(only, _)] = stack.as_slice() {
            only.convert_carrying(self.space)
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
                stack.push((self.interpolate(&a, &b, progress), combined));
            }
            stack[0].0
        };
        color.alpha = color.alpha.map(|alpha| alpha * (WHOLE - leftover));
        color.legacy = false;
        color
    }
}

/// The interpolation colour space and hue arc (CSS Color 4 §13.2, §13.5).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Interpolation {
    pub space: ColorSpace,
    pub hue: HueInterpolation,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub enum HueInterpolation {
    #[default]
    Shorter,
    Longer,
    Increasing,
    Decreasing,
}

impl Interpolation {
    /// The default space for hosts without a method: gamma-encoded sRGB when every colour is
    /// legacy, otherwise Oklab (CSS Color 4 §13.2).
    pub fn default_for<'a>(mut colors: impl Iterator<Item = &'a AbsoluteColor>) -> Self {
        Self {
            space: if colors.all(|color| color.legacy) {
                ColorSpace::Srgb
            } else {
                ColorSpace::Oklab
            },
            hue: HueInterpolation::Shorter,
        }
    }

    /// The colour at `progress` from `from` to `to`, unclamped in the interpolation space.
    pub fn interpolate(
        self,
        from: &AbsoluteColor,
        to: &AbsoluteColor,
        progress: f32,
    ) -> AbsoluteColor {
        let mut a = from.convert_carrying(self.space);
        let mut b = to.convert_carrying(self.space);
        for index in 0..3 {
            match (a.components[index], b.components[index]) {
                (None, Some(value)) => a.components[index] = Some(value),
                (Some(value), None) => b.components[index] = Some(value),
                _ => {}
            }
        }
        match (a.alpha, b.alpha) {
            (None, Some(alpha)) => a.alpha = Some(alpha),
            (Some(alpha), None) => b.alpha = Some(alpha),
            _ => {}
        }
        let hue = self.space.hue_index();
        if let Some(index) = hue
            && let (Some(first), Some(second)) = (a.components[index], b.components[index])
        {
            let (first, second) = self.hue.fix_up(f64::from(first), f64::from(second));
            a.components[index] = Some(first as f32);
            b.components[index] = Some(second as f32);
        }
        let progress = f64::from(progress);
        let alpha = match (a.alpha, b.alpha) {
            (Some(first), Some(second)) => {
                Some(lerp(f64::from(first), f64::from(second), progress))
            }
            _ => None,
        };
        let premultiplied = |color: &AbsoluteColor, index: usize| {
            color.components[index].map(|value| {
                let value = f64::from(value);
                match color.alpha {
                    Some(alpha) if hue != Some(index) => value * f64::from(alpha),
                    _ => value,
                }
            })
        };
        let components = std::array::from_fn(|index| {
            let (Some(first), Some(second)) = (premultiplied(&a, index), premultiplied(&b, index))
            else {
                return None;
            };
            let value = lerp(first, second, progress);
            let value = match alpha {
                Some(alpha) if alpha != 0.0 && hue != Some(index) => value / alpha,
                _ => value,
            };
            Some(if hue == Some(index) {
                value.rem_euclid(FULL_TURN_DEG)
            } else {
                value
            } as f32)
        });
        AbsoluteColor {
            space: self.space,
            components,
            alpha: alpha.map(|alpha| alpha as f32),
            legacy: false,
        }
    }
}

impl HueInterpolation {
    /// Adjusts two hues in `0..360` so linear interpolation follows the selected arc.
    fn fix_up(self, mut first: f64, mut second: f64) -> (f64, f64) {
        let difference = second - first;
        match self {
            Self::Shorter if difference > HALF_TURN_DEG => first += FULL_TURN_DEG,
            Self::Shorter if difference < -HALF_TURN_DEG => second += FULL_TURN_DEG,
            Self::Longer if difference > 0.0 && difference < HALF_TURN_DEG => {
                first += FULL_TURN_DEG;
            }
            Self::Longer if difference > -HALF_TURN_DEG && difference <= 0.0 => {
                second += FULL_TURN_DEG;
            }
            Self::Increasing if second < first => second += FULL_TURN_DEG,
            Self::Decreasing if first < second => first += FULL_TURN_DEG,
            _ => {}
        }
        (first, second)
    }
}

/// Gamma-encoded sRGB rounded to the 8-bit output, halves rounding up (CSS Color 4 §5.1).
pub(crate) fn quantize(srgb: [f64; 3]) -> [u8; 3] {
    srgb.map(|value| (value.clamp(0.0, 1.0) * f64::from(u8::MAX)).round() as u8)
}

/// Normalises a hue in degrees to `0..360`; infinite hues become zero (CSS Color 4 §4.3).
pub(crate) fn normalize_hue(degrees: f32) -> f32 {
    if degrees.is_finite() {
        degrees.rem_euclid(FULL_TURN_DEG as f32)
    } else {
        0.0
    }
}

fn lerp(first: f64, second: f64, progress: f64) -> f64 {
    first + (second - first) * progress
}

fn multiply(matrix: &Matrix, vector: Vector) -> Vector {
    matrix.map(|row| row[0] * vector[0] + row[1] * vector[1] + row[2] * vector[2])
}

/// Applies `transfer` to the magnitude, keeping the sign (the extended transfer functions).
fn signed(value: f64, transfer: impl Fn(f64) -> f64) -> f64 {
    transfer(value.abs()).copysign(value)
}
fn srgb_to_linear(value: f64) -> f64 {
    signed(value, |magnitude| {
        if magnitude <= SRGB_LINEAR_LIMIT {
            magnitude / SRGB_LINEAR_SLOPE
        } else {
            ((magnitude + SRGB_OFFSET) / (1.0 + SRGB_OFFSET)).powf(SRGB_GAMMA)
        }
    })
}
fn srgb_from_linear(value: f64) -> f64 {
    signed(value, |magnitude| {
        if magnitude > SRGB_GAMMA_LIMIT {
            (1.0 + SRGB_OFFSET) * magnitude.powf(1.0 / SRGB_GAMMA) - SRGB_OFFSET
        } else {
            SRGB_LINEAR_SLOPE * magnitude
        }
    })
}
fn prophoto_to_linear(value: f64) -> f64 {
    signed(value, |magnitude| {
        if magnitude <= PROPHOTO_LINEAR_LIMIT {
            magnitude / PROPHOTO_LINEAR_SLOPE
        } else {
            magnitude.powf(PROPHOTO_GAMMA)
        }
    })
}
fn prophoto_from_linear(value: f64) -> f64 {
    signed(value, |magnitude| {
        if magnitude >= PROPHOTO_GAMMA_LIMIT {
            magnitude.powf(1.0 / PROPHOTO_GAMMA)
        } else {
            PROPHOTO_LINEAR_SLOPE * magnitude
        }
    })
}

/// Rectangular coordinates of a polar colour; other colours are returned unchanged.
fn to_rectangular(space: ColorSpace, values: Vector) -> Vector {
    match space {
        ColorSpace::Lch | ColorSpace::Oklch => lch_to_lab(values),
        ColorSpace::Hsl => hsl_to_srgb(values),
        ColorSpace::Hwb => hwb_to_srgb(values),
        _ => values,
    }
}

/// Polar coordinates for polar `space`, with powerless hues missing (CSS Color 4 §4.4.1).
fn to_polar(space: ColorSpace, values: Vector) -> [Option<f64>; 3] {
    match space {
        ColorSpace::Lch => lab_to_lch(values, LCH_CHROMA_EPSILON),
        ColorSpace::Oklch => lab_to_lch(values, OKLCH_CHROMA_EPSILON),
        ColorSpace::Hsl => srgb_to_hsl(values),
        ColorSpace::Hwb => srgb_to_hwb(values),
        _ => values.map(Some),
    }
}

fn to_xyz_d65(space: ColorSpace, values: Vector) -> Vector {
    let linear = |transfer: fn(f64) -> f64, matrix: &Matrix| multiply(matrix, values.map(transfer));
    match space {
        ColorSpace::Srgb => linear(srgb_to_linear, &LIN_SRGB_TO_XYZ),
        ColorSpace::SrgbLinear => multiply(&LIN_SRGB_TO_XYZ, values),
        ColorSpace::DisplayP3 => linear(srgb_to_linear, &LIN_P3_TO_XYZ),
        ColorSpace::DisplayP3Linear => multiply(&LIN_P3_TO_XYZ, values),
        ColorSpace::A98Rgb => linear(
            |value| signed(value, |magnitude| magnitude.powf(A98_GAMMA)),
            &LIN_A98_TO_XYZ,
        ),
        ColorSpace::ProphotoRgb => multiply(
            &D50_TO_D65,
            multiply(&LIN_PROPHOTO_TO_XYZ_D50, values.map(prophoto_to_linear)),
        ),
        ColorSpace::Rec2020 => linear(
            |value| signed(value, |magnitude| magnitude.powf(REC2020_GAMMA)),
            &LIN_REC2020_TO_XYZ,
        ),
        ColorSpace::XyzD50 => multiply(&D50_TO_D65, values),
        ColorSpace::Lab | ColorSpace::Lch => multiply(&D50_TO_D65, lab_to_xyz_d50(values)),
        ColorSpace::Oklab | ColorSpace::Oklch => oklab_to_xyz(values),
        ColorSpace::XyzD65 | ColorSpace::Hsl | ColorSpace::Hwb => values,
    }
}

fn from_xyz_d65(space: ColorSpace, xyz: Vector) -> Vector {
    let encoded = |matrix: &Matrix, transfer: fn(f64) -> f64| multiply(matrix, xyz).map(transfer);
    match space {
        ColorSpace::Srgb => encoded(&XYZ_TO_LIN_SRGB, srgb_from_linear),
        ColorSpace::SrgbLinear => multiply(&XYZ_TO_LIN_SRGB, xyz),
        ColorSpace::DisplayP3 => encoded(&XYZ_TO_LIN_P3, srgb_from_linear),
        ColorSpace::DisplayP3Linear => multiply(&XYZ_TO_LIN_P3, xyz),
        ColorSpace::A98Rgb => encoded(&XYZ_TO_LIN_A98, |value| {
            signed(value, |magnitude| magnitude.powf(1.0 / A98_GAMMA))
        }),
        ColorSpace::ProphotoRgb => {
            multiply(&XYZ_D50_TO_LIN_PROPHOTO, multiply(&D65_TO_D50, xyz)).map(prophoto_from_linear)
        }
        ColorSpace::Rec2020 => encoded(&XYZ_TO_LIN_REC2020, |value| {
            signed(value, |magnitude| magnitude.powf(1.0 / REC2020_GAMMA))
        }),
        ColorSpace::XyzD50 => multiply(&D65_TO_D50, xyz),
        ColorSpace::Lab | ColorSpace::Lch => xyz_d50_to_lab(multiply(&D65_TO_D50, xyz)),
        ColorSpace::Oklab | ColorSpace::Oklch => xyz_to_oklab(xyz),
        ColorSpace::XyzD65 | ColorSpace::Hsl | ColorSpace::Hwb => xyz,
    }
}

fn xyz_d50_to_lab(xyz: Vector) -> Vector {
    let f = std::array::from_fn::<f64, 3, _>(|index| {
        let value = xyz[index] / D50[index];
        if value > LAB_EPSILON {
            value.cbrt()
        } else {
            (LAB_KAPPA * value + LAB_LIGHTNESS_OFFSET) / LAB_LIGHTNESS_SCALE
        }
    });
    [
        LAB_LIGHTNESS_SCALE * f[1] - LAB_LIGHTNESS_OFFSET,
        LAB_A_SCALE * (f[0] - f[1]),
        LAB_B_SCALE * (f[1] - f[2]),
    ]
}
fn lab_to_xyz_d50([lightness, a, b]: Vector) -> Vector {
    let lightness116 = lightness / LAB_LIGHTNESS_SCALE;
    let a500 = a / LAB_A_SCALE;
    let b200 = b / LAB_B_SCALE;
    let f1 = lightness116 + LAB_LIGHTNESS_OFFSET / LAB_LIGHTNESS_SCALE;
    let f0 = f1 + a500;
    let f2 = f1 - b200;
    let linear = LAB_LIGHTNESS_SCALE / LAB_KAPPA;
    let xyz = [
        if f0.powi(3) > LAB_EPSILON {
            f0.powi(3)
        } else {
            (lightness116 + a500) * linear
        },
        if lightness > LAB_KAPPA * LAB_EPSILON {
            ((lightness + LAB_LIGHTNESS_OFFSET) / LAB_LIGHTNESS_SCALE).powi(3)
        } else {
            lightness / LAB_KAPPA
        },
        if f2.powi(3) > LAB_EPSILON {
            f2.powi(3)
        } else {
            (lightness116 - b200) * linear
        },
    ];
    std::array::from_fn(|index| xyz[index] * D50[index])
}
fn xyz_to_oklab(xyz: Vector) -> Vector {
    multiply(&LMS_TO_OKLAB, multiply(&XYZ_TO_LMS, xyz).map(f64::cbrt))
}
fn oklab_to_xyz(oklab: Vector) -> Vector {
    multiply(
        &LMS_TO_XYZ,
        multiply(&OKLAB_TO_LMS, oklab).map(|value| value.powi(3)),
    )
}
fn oklab_to_srgb(oklab: Vector) -> Vector {
    from_xyz_d65(ColorSpace::Srgb, oklab_to_xyz(oklab))
}
fn srgb_to_oklab(srgb: Vector) -> Vector {
    xyz_to_oklab(to_xyz_d65(ColorSpace::Srgb, srgb))
}

fn lab_to_lch([lightness, a, b]: Vector, epsilon: f64) -> [Option<f64>; 3] {
    let chroma = a.hypot(b);
    if chroma <= epsilon {
        return [
            Some(lightness),
            Some(if chroma > 0.0 { 0.0 } else { chroma }),
            None,
        ];
    }
    let hue = b.atan2(a).to_degrees().rem_euclid(FULL_TURN_DEG);
    [Some(lightness), Some(chroma), Some(hue)]
}
fn lch_to_lab([lightness, chroma, hue]: Vector) -> Vector {
    let (sin, cos) = hue.to_radians().sin_cos();
    [lightness, chroma * cos, chroma * sin]
}

fn hsl_to_srgb([hue, saturation, lightness]: Vector) -> Vector {
    let saturation = saturation / PERCENT;
    let lightness = lightness / PERCENT;
    let amplitude = saturation * lightness.min(1.0 - lightness);
    let channel = |offset: f64| {
        let sector = (offset + hue / HSL_SECTOR_DEG).rem_euclid(HSL_SECTORS);
        let ramp = (sector - 3.0).min(9.0 - sector).clamp(-1.0, 1.0);
        lightness - amplitude * ramp
    };
    [channel(0.0), channel(8.0), channel(4.0)]
}
fn hwb_to_srgb([hue, whiteness, blackness]: Vector) -> Vector {
    let whiteness = whiteness / PERCENT;
    let blackness = blackness / PERCENT;
    if whiteness + blackness >= 1.0 {
        let gray = whiteness / (whiteness + blackness);
        return [gray; 3];
    }
    hsl_to_srgb([hue, PERCENT, PERCENT / 2.0])
        .map(|channel| channel * (1.0 - whiteness - blackness) + whiteness)
}
/// The hue of an sRGB colour from its largest channel, or `None` for greys.
fn srgb_hue([red, green, blue]: Vector) -> Option<f64> {
    let max = red.max(green).max(blue);
    let min = red.min(green).min(blue);
    let range = max - min;
    if range == 0.0 {
        return None;
    }
    let sector = if max == red {
        (green - blue) / range + if green < blue { 6.0 } else { 0.0 }
    } else if max == green {
        (blue - red) / range + 2.0
    } else {
        (red - green) / range + 4.0
    };
    Some(sector * FULL_TURN_DEG / 6.0)
}
fn srgb_to_hsl(rgb: Vector) -> [Option<f64>; 3] {
    let max = rgb[0].max(rgb[1]).max(rgb[2]);
    let min = rgb[0].min(rgb[1]).min(rgb[2]);
    let lightness = f64::midpoint(min, max);
    let mut hue = srgb_hue(rgb);
    let mut saturation = if hue.is_none() || lightness == 0.0 || lightness == 1.0 {
        0.0
    } else {
        (max - lightness) / lightness.min(1.0 - lightness)
    };
    // Far out-of-gamut colours yield negative saturation: rotate the hue instead.
    if saturation < 0.0 {
        hue = hue.map(|hue| hue + HALF_TURN_DEG);
        saturation = saturation.abs();
    }
    let mut saturation = saturation * PERCENT;
    if saturation <= HSL_SATURATION_EPSILON {
        hue = None;
        if saturation > 0.0 {
            saturation = 0.0;
        }
    }
    [
        hue.map(|hue| hue.rem_euclid(FULL_TURN_DEG)),
        Some(saturation),
        Some(lightness * PERCENT),
    ]
}
fn srgb_to_hwb(rgb: Vector) -> [Option<f64>; 3] {
    let whiteness = rgb[0].min(rgb[1]).min(rgb[2]) * PERCENT;
    let mut blackness = (1.0 - rgb[0].max(rgb[1]).max(rgb[2])) * PERCENT;
    let mut hue = srgb_hue(rgb).map(|hue| hue.rem_euclid(FULL_TURN_DEG));
    if whiteness + blackness >= PERCENT - HWB_ACHROMATIC_EPSILON {
        hue = None;
        if whiteness + blackness < PERCENT {
            blackness = PERCENT - whiteness;
        }
    }
    [hue, Some(whiteness), Some(blackness)]
}

fn in_unit_gamut(values: Vector) -> bool {
    values.iter().all(|value| (0.0..=1.0).contains(value))
}
fn clip(values: Vector) -> Vector {
    values.map(|value| value.clamp(0.0, 1.0))
}
/// ΔEOK: Euclidean distance in Oklab (CSS Color 4 §20.3).
fn delta_eok(reference: Vector, sample: Vector) -> f64 {
    let squares: f64 = (0..3)
        .map(|index| (reference[index] - sample[index]).powi(2))
        .sum();
    squares.sqrt()
}
