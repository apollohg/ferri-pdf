use crate::fonts::{FontFeature, FontId, FontSynthesis, FontVariation};
use crate::style::{FontVariantEmoji, FontVariantPosition, POINTS_PER_PIXEL, Style};
use crate::text::features::{self, CapsPlan, FaceFeatures};
use crate::text::{GlyphRun, PositionedGlyph, TextError};
use icu_casemap::{CaseMapper, ContextCase};
use icu_properties::{
    CodePointMapData, CodePointSetData,
    props::{
        Cased, DefaultIgnorableCodePoint, EmojiPresentation, GeneralCategory, VariationSelector,
    },
};
use rustybuzz::{BufferFlags, Direction, Face, Feature, Language, UnicodeBuffer, ttf_parser::Tag};
use std::collections::BTreeMap;
use std::ops::Range;
use std::sync::Arc;
use unicode_script::{Script, UnicodeScript};
use unicode_segmentation::UnicodeSegmentation;

pub(super) const NARROW_NO_BREAK_SPACE: char = '\u{202f}';
pub(super) const NARROW_SPACE_FACTOR: f32 = 0.5;
const MAX_LIGATURE_TRACKING_EM: f32 = 0.025;
const WORD_SPACE: char = ' ';
/// UA synthesis policy; CSS Fonts leaves the reduction factor to implementations.
const SMALL_CAPS_SCALE: f32 = 0.8;
/// UA synthesis policy for `font-variant-position` (§6.5): sizes up to the small size keep
/// more of their size than sizes from the large size, with linear interpolation between.
const POSITION_SMALL_SIZE: f32 = 20.0 * POINTS_PER_PIXEL;
const POSITION_LARGE_SIZE: f32 = 45.0 * POINTS_PER_PIXEL;
const POSITION_SMALL_SCALE: f32 = 0.82;
const POSITION_LARGE_SCALE: f32 = 2.0 / 3.0;
/// Synthesized superscripts rise and subscripts drop by these fractions of the unreduced em.
const SUPERSCRIPT_RISE_EM: f32 = 0.34;
const SUBSCRIPT_DROP_EM: f32 = 0.2;
/// Text and emoji presentation selectors (UTS #51 §1.4.3).
const TEXT_PRESENTATION: char = '\u{fe0e}';
const EMOJI_PRESENTATION: char = '\u{fe0f}';

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

/// The presentation selector `font-variant-emoji` appends after `ch`, which text supplies
/// itself when a presentation selector follows (§9.3).
pub(super) fn implied_presentation(
    emoji: FontVariantEmoji,
    ch: char,
    next: Option<char>,
) -> Option<char> {
    if emoji == FontVariantEmoji::Normal
        || matches!(next, Some(TEXT_PRESENTATION | EMOJI_PRESENTATION))
        || crate::spec_data::emoji::VARIATION_BASES
            .binary_search(&u32::from(ch))
            .is_err()
    {
        return None;
    }
    let emoji_default = || CodePointSetData::new::<EmojiPresentation>().contains(ch);
    Some(match emoji {
        FontVariantEmoji::Emoji => EMOJI_PRESENTATION,
        FontVariantEmoji::Unicode if emoji_default() => EMOJI_PRESENTATION,
        FontVariantEmoji::Normal | FontVariantEmoji::Text | FontVariantEmoji::Unicode => {
            TEXT_PRESENTATION
        }
    })
}

/// Whether the face has every variation sequence in `text`, including the presentation
/// selectors `emoji` implies.
pub(super) fn supports_variations(
    face: &Face<'_>,
    text: &str,
    emoji: Option<FontVariantEmoji>,
) -> bool {
    let mut previous = None;
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if variation_selector(ch)
            // Mongolian free selectors are handled by the script shaper through GSUB.
            && ch.script() != Script::Mongolian
            && let Some(base) = previous
            && !variation_selector(base)
            && face.glyph_variation_index(base, ch).is_none()
        {
            return false;
        }
        if let Some(selector) =
            emoji.and_then(|emoji| implied_presentation(emoji, ch, chars.peek().copied()))
            && face.glyph_variation_index(ch, selector).is_none()
        {
            return false;
        }
        previous = Some(ch);
    }
    true
}

/// Whether the face's substitution or positioning features include `tag`.
pub(super) fn has_feature(face: &Face<'_>, tag: [u8; 4]) -> bool {
    let tag = Tag::from_bytes(&tag);
    [face.tables().gsub, face.tables().gpos]
        .into_iter()
        .flatten()
        .any(|table| table.features.find(tag).is_some())
}

/// Whether the face defines `tag` as a language system of any script.
pub(super) fn has_language_system(face: &Face<'_>, tag: [u8; 4]) -> bool {
    let tag = Tag::from_bytes(&tag);
    [face.tables().gsub, face.tables().gpos]
        .into_iter()
        .flatten()
        .any(|table| {
            table
                .scripts
                .into_iter()
                .any(|script| script.languages.into_iter().any(|system| system.tag == tag))
        })
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
    /// The used font size, after `font-size-adjust`.
    pub size: f32,
    pub features: FaceFeatures<'f>,
    pub language: Option<&'f Language>,
    pub palette: Option<&'f Arc<[crate::graphics::Color]>>,
}

/// How a run renders `font-variant-position` (§6.5).
#[derive(Clone, Copy, Debug, PartialEq)]
enum Position {
    Normal,
    Native(FontFeature),
    /// Reduced glyphs shifted by the offset below the baseline, in points.
    Synthetic {
        scale: f32,
        offset: f32,
    },
}

/// The reduction of synthesized superscripts and subscripts at `size`.
fn position_scale(size: f32) -> f32 {
    let progress = ((size - POSITION_SMALL_SIZE) / (POSITION_LARGE_SIZE - POSITION_SMALL_SIZE))
        .clamp(0.0, 1.0);
    POSITION_SMALL_SCALE + (POSITION_LARGE_SCALE - POSITION_SMALL_SCALE) * progress
}

/// The glyphs of each cluster of `text`.
fn clusters(
    face: &Face<'_>,
    text: &str,
    rtl: bool,
    features: &[Feature],
) -> BTreeMap<u32, Vec<u32>> {
    let mut buffer = UnicodeBuffer::new();
    buffer.push_str(text);
    buffer.set_direction(if rtl {
        Direction::RightToLeft
    } else {
        Direction::LeftToRight
    });
    buffer.guess_segment_properties();
    let shaped = rustybuzz::shape(face, features, buffer);
    let mut clusters = BTreeMap::<u32, Vec<u32>>::new();
    for info in shaped.glyph_infos() {
        clusters
            .entry(info.cluster)
            .or_default()
            .push(info.glyph_id);
    }
    clusters
}

fn shaper_features(features: &[FontFeature]) -> Vec<Feature> {
    features
        .iter()
        .map(|feature| Feature::new(Tag::from_bytes(&feature.tag), feature.value, ..))
        .collect()
}

/// Native variants apply only where the face has them for every character of the run other
/// than spaces; otherwise the whole run is synthesized, if synthesis is allowed (§6.5, §2.8.4).
fn position_form(
    font: ShapingFont<'_, '_>,
    text: &str,
    rtl: bool,
    style: &Style,
    caps: CapsPlan,
) -> Position {
    let position = style.font_features.position;
    let Some(feature) = features::position_feature(position) else {
        return Position::Normal;
    };
    if !style.font_synthesis.position {
        return Position::Native(feature);
    }
    let face_features = FaceFeatures {
        caps: caps.features,
        ..font.features
    };
    let plain = clusters(
        font.face,
        text,
        rtl,
        &shaper_features(&features::run_features(style, face_features, false)),
    );
    let variant = clusters(
        font.face,
        text,
        rtl,
        &shaper_features(&features::run_features(
            style,
            FaceFeatures {
                position: Some(feature),
                ..face_features
            },
            false,
        )),
    );
    let ignorables = CodePointSetData::new::<DefaultIgnorableCodePoint>();
    let boundaries =
        crate::fonts::cluster_boundaries(plain.keys().map(|cluster| *cluster as usize), text.len());
    let complete = boundaries.windows(2).all(|bounds| {
        let source = &text[bounds[0]..bounds[1]];
        let cluster = u32::try_from(bounds[0]).expect("text offsets fit the shaper's clusters");
        source
            .chars()
            .all(|ch| ch.is_whitespace() || ignorables.contains(ch))
            || plain.get(&cluster) != variant.get(&cluster)
    });
    if complete {
        return Position::Native(feature);
    }
    let em = font.size;
    Position::Synthetic {
        scale: position_scale(em),
        offset: match position {
            FontVariantPosition::Super => -SUPERSCRIPT_RISE_EM * em,
            FontVariantPosition::Sub | FontVariantPosition::Normal => SUBSCRIPT_DROP_EM * em,
        },
    }
}

pub(super) fn shape_runs(
    font: ShapingFont<'_, '_>,
    paragraph: &str,
    range: Range<usize>,
    rtl: bool,
    style: &Style,
) -> Result<Vec<GlyphRun>, TextError> {
    let caps = features::caps_plan(
        style.font_variant_caps,
        |tag| has_feature(font.face, tag),
        style.font_synthesis.small_caps,
    );
    let position = position_form(font, &paragraph[range.clone()], rtl, style, caps);
    if !caps.synthetic() {
        return shape_run(font, paragraph, range, rtl, style, caps, position, false)
            .map(|run| vec![run]);
    }
    let cased = CodePointSetData::new::<Cased>();
    let mut pieces: Vec<(Range<usize>, bool)> = Vec::new();
    for (offset, cluster) in paragraph[range.clone()].grapheme_indices(true) {
        let start = range.start + offset;
        let small = cluster.chars().any(|ch| {
            if ch.is_lowercase() {
                caps.synthetic_lower
            } else {
                caps.synthetic_upper && cased.contains(ch)
            }
        });
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
        .map(|(range, small)| shape_run(font, paragraph, range, rtl, style, caps, position, small))
        .collect()
}

#[expect(
    clippy::too_many_arguments,
    reason = "a run's text, direction, style and its caps and position forms"
)]
fn shape_run(
    font: ShapingFont<'_, '_>,
    paragraph: &str,
    range: Range<usize>,
    rtl: bool,
    style: &Style,
    caps: CapsPlan,
    position: Position,
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
        size: used_size,
        features: face_features,
        language,
        palette,
    } = font;
    let text = &paragraph[range.clone()];
    let (position_feature, synthetic_position) = match position {
        Position::Normal => (None, None),
        Position::Native(feature) => (Some(feature), None),
        Position::Synthetic { scale, offset } => (None, Some((scale, offset))),
    };
    let (position_scale, baseline_offset) = synthetic_position.unwrap_or((1.0, 0.0));
    let size = used_size
        * position_scale
        * if synthetic_caps {
            SMALL_CAPS_SCALE
        } else {
            1.0
        };
    synthesis.small_caps = synthetic_caps;
    synthesis.position = synthetic_position.is_some();
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
    let emoji = style.font_features.emoji;
    let add = |buffer: &mut UnicodeBuffer, ch, next, cluster| {
        buffer.add(display_character(ch), cluster);
        if let Some(selector) = implied_presentation(emoji, ch, next) {
            buffer.add(selector, cluster);
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
            let mut chars = mapped.chars().peekable();
            while let Some(ch) = chars.next() {
                add(&mut buffer, ch, chars.peek().copied(), start as u32);
            }
        }
    } else if missing == MissingGlyphs::Substitute || emoji != FontVariantEmoji::Normal {
        let mut chars = text.char_indices().peekable();
        while let Some((start, ch)) = chars.next() {
            add(
                &mut buffer,
                ch,
                chars.peek().map(|(_, next)| *next),
                start as u32,
            );
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
    if let Some(language) = language {
        buffer.set_language(language.clone());
    }
    buffer.guess_segment_properties();
    // Words shape independently, so no glyph kerns against a word space. Fields are set directly
    // because `Feature::new` stores an inclusive end that its cluster masking treats as exclusive.
    let mut shaper: Vec<Feature> = text
        .match_indices(WORD_SPACE)
        .map(|(start, _)| Feature {
            tag: Tag::from_bytes(&features::KERNING),
            value: 0,
            start: start as u32,
            end: (start + WORD_SPACE.len_utf8()) as u32,
        })
        .collect();
    shaper.extend(shaper_features(&features::run_features(
        style,
        FaceFeatures {
            caps: caps.features,
            position: position_feature,
            ..face_features
        },
        style.letter_spacing.abs() > size * MAX_LIGATURE_TRACKING_EM,
    )));
    let shaped = rustybuzz::shape(face, &shaper, buffer);
    let boundaries = crate::fonts::cluster_boundaries(
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
    let mut pen_y = baseline_offset;
    let mut cluster_advance = 0.0;
    let word_spacing = style.word_spacing.resolve(size);
    let em_offset = if size == 0.0 {
        0.0
    } else {
        baseline_offset / size
    };
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
            y_offset: em_offset - (pos.y_offset as f32) / upem,
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
        metric_size: used_size,
        font_id,
        synthesis,
        oblique_skew,
        variations: variations.to_vec(),
        palette: palette.cloned(),
        color: style.text_paint.fill.used,
        stroke: crate::text::TextStroke::of(style),
        marks: Vec::new(),
        mark_shapes: Vec::new(),
        shadows: style.text_paint.shadows.clone(),
        leading: style.line_height.resolve(style.size),
        white_space: style.white_space,
        ink_bounds,
        ink_sides: [bounds.x_min, bounds.x_max].map(|side| f32::from(side) / upem),
        metrics: crate::fonts::FaceMetrics::of(face),
        glyphs,
        removed_spaces: Vec::new(),
    })
}
