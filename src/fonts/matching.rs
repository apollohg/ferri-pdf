use crate::fonts::{
    FontData, FontFace, FontFaceStyle, FontId, FontNamespace, FontSelection, FontSlant, FontStyle,
    FontWeight, FontWidth,
};
use std::cmp::Ordering;

pub(super) fn names_match(left: &str, right: &str) -> bool {
    if left.is_ascii() && right.is_ascii() {
        left.eq_ignore_ascii_case(right)
    } else {
        let mapper = icu_casemap::CaseMapper::new();
        mapper.fold_string(left) == mapper.fold_string(right)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Rank(u8, f32);
impl Eq for Rank {}
impl Ord for Rank {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.cmp(&other.0).then(self.1.total_cmp(&other.1))
    }
}
impl PartialOrd for Rank {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

pub(super) fn candidates(
    faces: &[FontFace],
    namespace: FontNamespace,
    family: &str,
    requested: FontSelection,
    installed_only: bool,
) -> Vec<FontId> {
    let family_match = |face: &FontFace| {
        face.namespace == namespace
            && names_match(&face.descriptor.family, family)
            && (!installed_only || !matches!(face.data, FontData::Lazy { .. }))
    };
    let override_bundled = faces
        .iter()
        .any(|face| family_match(face) && !matches!(face.data, FontData::Bundled(_)));
    let eligible = |face: &FontFace| {
        family_match(face) && (!override_bundled || !matches!(face.data, FontData::Bundled(_)))
    };
    let rank = |face: &FontFace| {
        let descriptor = &face.descriptor;
        let (min, max) = descriptor.width.bounds(FontWidth::NORMAL);
        let point = requested.width.value().clamp(min.value(), max.value());
        let target = requested.width.value();
        let preferred = if target <= FontWidth::NORMAL.value() {
            point <= target
        } else {
            point >= target
        };
        let width = Rank(u8::from(!preferred), (point - target).abs());
        let (min, max) = descriptor.weight.bounds(FontWeight::NORMAL);
        let point = requested.weight.value().clamp(min.value(), max.value());
        (
            width,
            style_match(descriptor.style, requested).0,
            weight_rank(point, requested.weight.value()),
        )
    };
    let best = faces.iter().filter(|face| eligible(face)).map(rank).min();
    faces
        .iter()
        .enumerate()
        .rev()
        .filter(|(_, face)| eligible(face) && Some(rank(face)) == best)
        .map(|(index, _)| FontId(index))
        .collect()
}

pub(super) fn style_match(face: FontFaceStyle, requested: FontSelection) -> (Rank, FontSlant) {
    let italic = face.is_italic();
    let (min, max) = match face {
        FontFaceStyle::Oblique(range) => range.bounds(FontSlant::DEFAULT),
        _ => (FontSlant::NORMAL, FontSlant::NORMAL),
    };
    let (mut min, mut max) = (min.value(), max.value());
    let normal = requested.style == FontStyle::Normal
        || (requested.style == FontStyle::Oblique && requested.slant == FontSlant::NORMAL);
    let (rank, angle) = if let Some(sign) = requested.style.italic_direction() {
        // CSS Fonts 4 §2.4: `left` and `right` match an italic of their slant direction, else
        // an oblique slanted the same way, searched as `italic` searches obliques.
        let face_sign = match face {
            FontFaceStyle::Left => Some(1.0),
            FontFaceStyle::Right => Some(-1.0),
            _ => None,
        };
        if sign < 0.0 {
            (min, max) = (-max, -min);
        }
        let target = FontSlant::ITALIC_MATCH.value();
        let (rank, point) = if face_sign == Some(sign) {
            (Rank(0, 0.0), 0.0)
        } else if face_sign.is_some() {
            (Rank(6, 0.0), 0.0)
        } else if italic {
            (Rank(3, 0.0), 0.0)
        } else if max >= target {
            let point = min.max(target);
            (Rank(1, point - target), point)
        } else if max > 0.0 {
            (Rank(2, target - max), max)
        } else if max == 0.0 {
            (Rank(4, 0.0), 0.0)
        } else {
            (Rank(5, -max), max)
        };
        (rank, point * sign)
    } else if normal {
        if italic {
            (Rank(1, 0.0), 0.0)
        } else if max >= 0.0 {
            let point = min.max(0.0);
            (Rank(0, point), point)
        } else {
            (Rank(2, -max), max)
        }
    } else if requested.style == FontStyle::Italic {
        let target = FontSlant::ITALIC_MATCH.value();
        if italic {
            (Rank(0, 0.0), 0.0)
        } else if max >= target {
            let point = min.max(target);
            (Rank(1, point - target), point)
        } else if max > 0.0 {
            (Rank(2, target - max), max)
        } else if max == 0.0 {
            (Rank(3, 0.0), 0.0)
        } else {
            (Rank(4, -max), max)
        }
    } else {
        let sign = requested.slant.value().signum();
        if sign < 0.0 {
            (min, max) = (-max, -min);
        }
        let target = requested.slant.value().abs();
        let (rank, point) = if italic {
            (Rank(4, 0.0), 0.0)
        } else if max > 0.0 {
            // A subnormal maximum bounds the positive slants below the smallest normal one.
            let point = target.clamp(min.max(f32::MIN_POSITIVE).min(max), max);
            let group = if point == target {
                0
            } else if (target >= FontSlant::ITALIC_MATCH.value()) == (point > target) {
                1
            } else {
                2
            };
            (Rank(group, (point - target).abs()), point)
        } else if max == 0.0 && requested.synthesis.oblique {
            (Rank(3, 0.0), 0.0)
        } else {
            (Rank(5, -max), max)
        };
        (rank, point * sign)
    };
    (
        rank,
        FontSlant::new(angle).expect("matching stays within the validated slant bounds"),
    )
}

fn weight_rank(face: f32, target: f32) -> Rank {
    let normal = FontWeight::NORMAL.value();
    let medium = FontWeight::MEDIUM.value();
    if (normal..=medium).contains(&target) {
        if (target..=medium).contains(&face) {
            Rank(0, face - target)
        } else if face < target {
            Rank(1, target - face)
        } else {
            Rank(2, face - medium)
        }
    } else if target < normal {
        if face <= target {
            Rank(0, target - face)
        } else {
            Rank(1, face - target)
        }
    } else if face >= target {
        Rank(0, face - target)
    } else {
        Rank(1, target - face)
    }
}
