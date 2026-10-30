use crate::fonts::{
    FontData, FontDescriptor, FontFace, FontFaceStyle, FontRange, FontStyle, FontWeight, FontWidth,
};

pub(super) const LIBERATION_SANS: &str = "Liberation Sans";
pub(super) const LIBERATION_SERIF: &str = "Liberation Serif";
pub(super) const LIBERATION_MONO: &str = "Liberation Mono";
pub(super) const DEJAVU_SANS: &str = "DejaVu Sans";
pub(super) const DEJAVU_SERIF: &str = "DejaVu Serif";
pub(super) const DEJAVU_SANS_MONO: &str = "DejaVu Sans Mono";

pub(super) fn faces() -> Vec<FontFace> {
    use FontStyle::{Left, Normal, Oblique};
    const NORMAL: FontWeight = FontWeight::NORMAL;
    const BOLD: FontWeight = FontWeight::BOLD;
    macro_rules! face {
        ($family:expr, $weight:expr, $style:expr, $file:literal) => {
            FontFace {
                descriptor: FontDescriptor {
                    family: $family.into(),
                    weight: FontRange::single($weight),
                    width: FontRange::single(FontWidth::NORMAL),
                    style: FontFaceStyle::from($style),
                },
                settings: crate::fonts::FaceSettings::default(),
                data: FontData::Bundled(include_bytes!(concat!(
                    "../../assets/fonts/",
                    $file,
                    ".ttf"
                ))),
                namespace: crate::fonts::FontNamespace::ROOT,
            }
        };
    }
    // Every bundled italic slants clockwise by its `post` italicAngle (CSS Fonts 4 §2.4).
    vec![
        face!(LIBERATION_SANS, NORMAL, Normal, "LiberationSans-Regular"),
        face!(LIBERATION_SANS, BOLD, Normal, "LiberationSans-Bold"),
        face!(LIBERATION_SANS, NORMAL, Left, "LiberationSans-Italic"),
        face!(LIBERATION_SANS, BOLD, Left, "LiberationSans-BoldItalic"),
        face!(LIBERATION_SERIF, NORMAL, Normal, "LiberationSerif-Regular"),
        face!(LIBERATION_SERIF, BOLD, Normal, "LiberationSerif-Bold"),
        face!(LIBERATION_SERIF, NORMAL, Left, "LiberationSerif-Italic"),
        face!(LIBERATION_SERIF, BOLD, Left, "LiberationSerif-BoldItalic"),
        face!(LIBERATION_MONO, NORMAL, Normal, "LiberationMono-Regular"),
        face!(LIBERATION_MONO, BOLD, Normal, "LiberationMono-Bold"),
        face!(LIBERATION_MONO, NORMAL, Left, "LiberationMono-Italic"),
        face!(LIBERATION_MONO, BOLD, Left, "LiberationMono-BoldItalic"),
        face!(DEJAVU_SANS, NORMAL, Normal, "DejaVuSans"),
        face!(DEJAVU_SANS, BOLD, Normal, "DejaVuSans-Bold"),
        face!(DEJAVU_SANS, NORMAL, Oblique, "DejaVuSans-Oblique"),
        face!(DEJAVU_SANS, BOLD, Oblique, "DejaVuSans-BoldOblique"),
        face!(DEJAVU_SERIF, NORMAL, Normal, "DejaVuSerif"),
        face!(DEJAVU_SERIF, BOLD, Normal, "DejaVuSerif-Bold"),
        face!(DEJAVU_SERIF, NORMAL, Left, "DejaVuSerif-Italic"),
        face!(DEJAVU_SERIF, BOLD, Left, "DejaVuSerif-BoldItalic"),
        face!(DEJAVU_SANS_MONO, NORMAL, Normal, "DejaVuSansMono"),
        face!(DEJAVU_SANS_MONO, BOLD, Normal, "DejaVuSansMono-Bold"),
        face!(DEJAVU_SANS_MONO, NORMAL, Oblique, "DejaVuSansMono-Oblique"),
        face!(
            DEJAVU_SANS_MONO,
            BOLD,
            Oblique,
            "DejaVuSansMono-BoldOblique"
        ),
    ]
}
