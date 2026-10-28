use crate::text::{GlyphRun, PositionedGlyph, TextError};
use ferri_pdf_fonts::{FontId, FontSynthesis, FontVariation};
use ferri_pdf_style::{FontVariantCaps, Style};
use icu_casemap::{CaseMapper, ContextCase};
use icu_properties::{
    CodePointMapData, CodePointSetData,
    props::{DefaultIgnorableCodePoint, GeneralCategory, VariationSelector},
};
use rustybuzz::{BufferFlags, Direction, Face, Feature, UnicodeBuffer, ttf_parser::Tag};
use std::ops::Range;
use unicode_script::{Script, UnicodeScript};
use unicode_segmentation::UnicodeSegmentation;

pub(super) const NARROW_NO_BREAK_SPACE: char = '\u{202f}';
pub(super) const NARROW_SPACE_FACTOR: f32 = 0.5;
const MAX_LIGATURE_TRACKING_EM: f32 = 0.025;
const WORD_SPACE: char = ' ';
const KERNING: Tag = Tag::from_bytes(b"kern");
const LIGATURES: Tag = Tag::from_bytes(b"liga");
const CONTEXTUAL_LIGATURES: Tag = Tag::from_bytes(b"clig");
const SMALL_CAPITALS: Tag = Tag::from_bytes(b"smcp");
/// UA synthesis policy; CSS Fonts leaves the reduction factor to implementations.
const SMALL_CAPS_SCALE: f32 = 0.8;
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum MissingGlyphs {
    Reject,
    Substitute,
}

pub(super) fn private_use(ch: char) -> bool {
    !ch.is_ascii()
        && CodePointMapData::<GeneralCategory>::new().get(ch) == GeneralCategory::PrivateUse
}

pub(super) fn variation_selector(ch: char) -> bool {
    !ch.is_ascii() && CodePointSetData::new::<VariationSelector>().contains(ch)
}

pub(super) fn supports_variations(face: &Face<'_>, text: &str) -> bool {
    let mut previous = None;
    for ch in text.chars() {
        if variation_selector(ch)
            // Mongolian free selectors are handled by the script shaper through GSUB.
            && ch.script() != Script::Mongolian
            && let Some(base) = previous
            && !variation_selector(base)
            && face.glyph_variation_index(base, ch).is_none()
        {
            return false;
        }
        previous = Some(ch);
    }
    true
}

#[derive(Clone, Copy)]
pub(super) struct ShapingFont<'f, 'data> {
    pub face: &'f Face<'data>,
    pub font_id: FontId,
    pub synthesis: FontSynthesis,
    pub oblique_skew: f32,
    pub variations: &'f [FontVariation],
    pub narrow_space_advance: Option<f32>,
    pub missing: MissingGlyphs,
}

pub(super) fn shape_runs(
    font: ShapingFont<'_, '_>,
    paragraph: &str,
    range: Range<usize>,
    rtl: bool,
    style: &Style,
) -> Result<Vec<GlyphRun>, TextError> {
    if style.font_variant_caps != FontVariantCaps::SmallCaps
        || !style.font_synthesis.small_caps
        || font
            .face
            .tables()
            .gsub
            .is_some_and(|table| table.features.find(SMALL_CAPITALS).is_some())
    {
        return shape_run(font, paragraph, range, rtl, style, false).map(|run| vec![run]);
    }
    let mut pieces: Vec<(Range<usize>, bool)> = Vec::new();
    for (offset, cluster) in paragraph[range.clone()].grapheme_indices(true) {
        let start = range.start + offset;
        let small = cluster.chars().any(char::is_lowercase);
        if let Some((previous, same)) = pieces.last_mut()
            && *same == small
        {
            previous.end = start + cluster.len();
        } else {
            pieces.push((start..start + cluster.len(), small));
        }
    }
    if rtl {
        pieces.reverse();
    }
    pieces
        .into_iter()
        .map(|(range, small)| shape_run(font, paragraph, range, rtl, style, small))
        .collect()
}

fn shape_run(
    font: ShapingFont<'_, '_>,
    paragraph: &str,
    range: Range<usize>,
    rtl: bool,
    style: &Style,
    synthetic_caps: bool,
) -> Result<GlyphRun, TextError> {
    let ShapingFont {
        face,
        font_id,
        mut synthesis,
        oblique_skew,
        variations,
        narrow_space_advance,
        missing,
    } = font;
    let text = &paragraph[range.clone()];
    let size = style.size
        * if synthetic_caps {
            SMALL_CAPS_SCALE
        } else {
            1.0
        };
    synthesis.small_caps = synthetic_caps;
    let mut buffer = UnicodeBuffer::new();
    let ignorables = CodePointSetData::new::<DefaultIgnorableCodePoint>();
    let display_character = |ch| {
        if missing == MissingGlyphs::Substitute
            && !ignorables.contains(ch)
            && (private_use(ch) || face.glyph_index(ch).is_none())
        {
            char::REPLACEMENT_CHARACTER
        } else {
            ch
        }
    };
    if synthetic_caps {
        let mapper = CaseMapper::new();
        let mut context = mapper.context(paragraph);
        let language = style
            .language
            .parse::<icu_locale_core::Locale>()
            .map_or(icu_locale_core::LanguageIdentifier::UNKNOWN, |locale| {
                locale.id
            });
        let mut bytes = 0;
        let mut mapped = String::new();
        for (start, cluster) in text.grapheme_indices(true) {
            mapped.clear();
            for (offset, _) in cluster.char_indices() {
                context
                    .map_char(
                        range.start + start + offset,
                        &language,
                        ContextCase::Upper,
                        &mut mapped,
                    )
                    .expect("valid source offsets and String writes cannot fail");
                if mapped.len() > crate::document::MAX_INLINE_TEXT_BYTES {
                    return Err(TextError::CaseExpansionLimit);
                }
            }
            bytes += mapped.len();
            if bytes > crate::document::MAX_INLINE_TEXT_BYTES {
                return Err(TextError::CaseExpansionLimit);
            }
            for ch in mapped.chars() {
                buffer.add(display_character(ch), start as u32);
            }
        }
    } else if missing == MissingGlyphs::Substitute {
        for (start, ch) in text.char_indices() {
            buffer.add(display_character(ch), start as u32);
        }
    } else {
        buffer.push_str(text);
    }
    buffer.set_flags(BufferFlags::REMOVE_DEFAULT_IGNORABLES);
    buffer.set_pre_context(&paragraph[..range.start]);
    buffer.set_post_context(&paragraph[range.end..]);
    buffer.set_direction(if rtl {
        Direction::RightToLeft
    } else {
        Direction::LeftToRight
    });
    if let Ok(language) = style.language.parse() {
        buffer.set_language(language);
    }
    buffer.guess_segment_properties();
    // Words shape independently, so no glyph kerns against a word space. Fields are set directly
    // because `Feature::new` stores an inclusive end that its cluster masking treats as exclusive.
    let mut features: Vec<Feature> = text
        .match_indices(WORD_SPACE)
        .map(|(start, _)| Feature {
            tag: KERNING,
            value: 0,
            start: start as u32,
            end: (start + WORD_SPACE.len_utf8()) as u32,
        })
        .collect();
    if style.font_variant_caps == FontVariantCaps::SmallCaps && !synthetic_caps {
        features.push(Feature::new(SMALL_CAPITALS, 1, ..));
    }
    if style.letter_spacing.abs() > size * MAX_LIGATURE_TRACKING_EM {
        features.extend([LIGATURES, CONTEXTUAL_LIGATURES].map(|tag| Feature::new(tag, 0, ..)));
    }
    let shaped = rustybuzz::shape(face, &features, buffer);
    let boundaries = ferri_pdf_fonts::cluster_boundaries(
        shaped
            .glyph_infos()
            .iter()
            .map(|glyph| glyph.cluster as usize),
        text.len(),
    );
    let upem = face.units_per_em() as f32;
    let mut width = 0.0;
    let mut glyphs = Vec::new();
    let bounds = face.global_bounding_box();
    let mut ink_bounds: Option<[f32; 2]> = None;
    let mut pen_y = 0.0;
    let mut cluster_advance = 0.0;
    let word_spacing = style.word_spacing.resolve(size);
    for (index, (info, pos)) in shaped
        .glyph_infos()
        .iter()
        .zip(shaped.glyph_positions())
        .enumerate()
    {
        if info.glyph_id == 0 && size != 0.0 && missing == MissingGlyphs::Reject {
            return Err(TextError::MissingGlyph { text: text.into() });
        }
        let start = info.cluster as usize;
        let end = boundaries[boundaries.binary_search(&start).unwrap() + 1];
        let last_in_cluster = shaped
            .glyph_infos()
            .get(index + 1)
            .is_none_or(|next| next.cluster != info.cluster);
        let advance = if last_in_cluster && text[start..end].starts_with(NARROW_NO_BREAK_SPACE) {
            narrow_space_advance
                .map_or(pos.x_advance as f32 / upem, |width| width - cluster_advance)
        } else {
            pos.x_advance as f32 / upem
        };
        cluster_advance += advance;
        if last_in_cluster {
            cluster_advance = 0.0;
        }
        let top = pen_y - (f32::from(bounds.y_max) + pos.y_offset as f32) / upem * size;
        let bottom = pen_y - (f32::from(bounds.y_min) + pos.y_offset as f32) / upem * size;
        ink_bounds = Some(match ink_bounds {
            Some([a, b]) => [a.min(top), b.max(bottom)],
            None => [top, bottom],
        });
        pen_y -= pos.y_advance as f32 / upem * size;
        let source = &text[start..end];
        let text_range = if source.is_ascii() {
            start..end
        } else {
            let trim = |ch| ignorables.contains(ch) && !variation_selector(ch);
            let mapped = source.trim_start_matches(trim);
            let mapped_start = start + source.len() - mapped.len();
            let mapped = mapped.trim_end_matches(trim);
            if mapped.is_empty() {
                start..end
            } else {
                mapped_start..mapped_start + mapped.len()
            }
        };
        let x_spacing = if last_in_cluster {
            style.letter_spacing
                + if crate::text::spacing::word_space_text(source) {
                    word_spacing
                } else {
                    0.0
                }
        } else {
            0.0
        };
        width += advance * size + x_spacing;
        if !width.is_finite() {
            return Err(TextError::NonFiniteGeometry);
        }
        let glyph = PositionedGlyph {
            id: info.glyph_id,
            x_advance: advance,
            x_spacing,
            x_offset: pos.x_offset as f32 / upem,
            y_offset: -(pos.y_offset as f32) / upem,
            y_advance: -(pos.y_advance as f32) / upem,
            text_range,
        };
        if size != 0.0 && glyph.normalized_advance(size).is_none() {
            return Err(TextError::NonFiniteGeometry);
        }
        glyphs.push(glyph);
    }
    Ok(GlyphRun {
        leader: false,
        text: text.into(),
        source_range: range.clone(),
        source_span: 0,
        rtl,
        bidi_scoped: false,
        x: 0.0,
        width,
        baseline_shift: 0.0,
        font_size: size,
        metric_size: style.size,
        font_id,
        synthesis,
        oblique_skew,
        variations: variations.to_vec(),
        color: style.color,
        leading: style.line_height.resolve(style.size),
        white_space: style.white_space,
        ink_bounds,
        metrics: ferri_pdf_fonts::FaceMetrics::of(face),
        glyphs,
    })
}
