//! Manual and automatic hyphenation (CSS Text 4 §6.3). Automatic opportunities come from the
//! hyphenation patterns of the content language; languages without patterns hyphenate only
//! at conditional hyphens.
use crate::document::Paragraph;
use crate::style::{Hyphens, LineBreak, Style, WordBreak};
use crate::text::shaping::MissingGlyphs;
use crate::text::{GlyphRun, Piece, TextContext, TextError, line_boxes};
use icu_properties::{
    CodePointMapData,
    props::{GeneralCategory, GeneralCategoryGroup},
};
use unicode_segmentation::UnicodeSegmentation;

pub(crate) const SOFT_HYPHEN: char = '\u{00ad}';
const HYPHEN: &str = "\u{2010}";
const ARMENIAN_HYPHEN: &str = "\u{058a}";
const ARMENIAN: &str = "hy";
const HYPHEN_FALLBACK: &str = "-";
/// No visible hyphen, as for scripts that do not mark hyphenation.
const NO_HYPHEN: &str = "";
/// CSS Text 4 §6.3.4: the suggested `auto` minimum length of a hyphenated word.
const AUTO_WORD_MINIMUM: usize = 5;
/// The pattern resources hyphenate between letters, so at least one counts on each side.
const MINIMUM_SIDE: usize = 1;

/// CSS Text 4 §6.1: `auto-phrase` suppresses hyphenation as `none` does; `break-all` and
/// `line-break: anywhere` apply no automatic hyphenation.
pub(super) fn used(style: &Style) -> Hyphens {
    let breaking = &style.line_breaking;
    match style.hyphens {
        _ if breaking.word_break == WordBreak::AutoPhrase => Hyphens::None,
        Hyphens::Auto
            if breaking.word_break == WordBreak::BreakAll
                || breaking.line_break == LineBreak::Anywhere =>
        {
            Hyphens::Manual
        }
        hyphens => hyphens,
    }
}

/// The hyphenation resource of a content language, by its primary subtag.
fn resource(language: &str) -> Option<hypher::Lang> {
    let primary = language.split('-').next()?.to_ascii_lowercase();
    let code: [u8; 2] = primary.as_bytes().try_into().ok()?;
    hypher::Lang::from_iso(code)
}

/// CSS Text 4 §6.3.4: nonspacing marks and punctuation do not count towards the limits.
fn counted(text: &str) -> usize {
    let categories = CodePointMapData::<GeneralCategory>::new();
    text.chars()
        .filter(|&ch| {
            let category = categories.get(ch);
            category != GeneralCategory::NonspacingMark
                && !GeneralCategoryGroup::Punctuation.contains(category)
        })
        .count()
}

/// Automatic hyphenation opportunities in offset order. Words with a conditional hyphen
/// keep only their explicit opportunities; both sides of an opportunity must hyphenate
/// automatically. Inline box boundaries do not delimit words.
pub(super) fn automatic(paragraph: &Paragraph) -> Vec<usize> {
    if !paragraph
        .spans()
        .iter()
        .any(|span| used(&span.style) == Hyphens::Auto)
    {
        return Vec::new();
    }
    let text = paragraph.text();
    let mut points = Vec::new();
    for (start, word) in text.split_word_bound_indices() {
        if word.contains(SOFT_HYPHEN) || !word.chars().any(char::is_alphabetic) {
            continue;
        }
        let style = paragraph.style_at(start);
        let Some(language) = resource(&style.language) else {
            continue;
        };
        let limits = style.line_breaking.hyphenate_limit_chars;
        let (left, right) = language.bounds();
        let before = limits.before.unwrap_or(left);
        let after = limits.after.unwrap_or(right);
        let total = counted(word);
        if total < limits.word.unwrap_or(AUTO_WORD_MINIMUM) {
            continue;
        }
        let mut offset = start;
        let mut syllables =
            hypher::hyphenate_bounded(word, language, MINIMUM_SIDE, MINIMUM_SIDE).peekable();
        while let Some(syllable) = syllables.next() {
            offset += syllable.len();
            if syllables.peek().is_none() {
                break;
            }
            let leading = counted(&text[start..offset]);
            if leading >= before
                && total - leading >= after
                && used(paragraph.style_at(offset - 1)) == Hyphens::Auto
                && used(paragraph.style_at(offset)) == Hyphens::Auto
            {
                points.push(offset);
            }
        }
    }
    points
}

/// Whether the opportunity at `end` hyphenates: after a conditional hyphen, or automatic.
pub(super) fn is_soft_break(paragraph: &Paragraph, automatic: &[usize], end: usize) -> bool {
    automatic.binary_search(&end).is_ok()
        || (end < paragraph.text().len() && paragraph.text()[..end].ends_with(SOFT_HYPHEN))
}

pub(super) fn allowed(paragraph: &Paragraph, end: usize) -> bool {
    if !paragraph.text()[..end].ends_with(SOFT_HYPHEN) {
        return true;
    }
    let offset = end - SOFT_HYPHEN.len_utf8();
    paragraph
        .spans()
        .iter()
        .find(|span| span.range.contains(&offset))
        .is_none_or(|span| used(&span.style) != Hyphens::None)
}

/// CSS Text 4 §6.3.2: `hyphenate-character`, or the content language's convention.
fn character(style: &Style) -> &str {
    if let Some(text) = &style.line_breaking.hyphenate_character {
        return text;
    }
    let primary = style.language.split('-').next().unwrap_or_default();
    if primary.eq_ignore_ascii_case(ARMENIAN) {
        ARMENIAN_HYPHEN
    } else if resource(&style.language)
        .is_some_and(|language| language.hyphenation_character().is_none())
    {
        NO_HYPHEN
    } else {
        HYPHEN
    }
}

/// Where a hyphenating break at `end` takes its hyphenation string's style, and the source the
/// string stands for: a conditional hyphen itself, or the end of the character before an
/// automatic break. `None` where the break does not hyphenate.
fn hyphenating(
    paragraph: &Paragraph,
    automatic: &[usize],
    end: usize,
) -> Option<(usize, std::ops::Range<usize>)> {
    if !is_soft_break(paragraph, automatic, end) || !allowed(paragraph, end) {
        return None;
    }
    if paragraph.text()[..end].ends_with(SOFT_HYPHEN) {
        let offset = end - SOFT_HYPHEN.len_utf8();
        return Some((offset, offset..end));
    }
    let previous = paragraph.text()[..end]
        .char_indices()
        .next_back()
        .map_or(0, |(offset, _)| offset);
    Some((previous, end..end))
}

/// The hyphenation string a line ending with a break at `end` shows, where it hyphenates.
pub(crate) fn string<'p>(
    paragraph: &'p Paragraph,
    automatic: &[usize],
    end: usize,
) -> Option<&'p str> {
    let (offset, _) = hyphenating(paragraph, automatic, end)?;
    Some(character(paragraph.style_at(offset)))
}

/// Inserts the hyphenation string of a hyphenating break at `end`. A conditional hyphen's
/// string belongs to its own span, including its bidi level; an automatic one belongs to
/// the character before the break.
pub(super) fn decorate<'p>(
    context: &TextContext<'_>,
    paragraph: &'p Paragraph,
    automatic: &[usize],
    bidi: &crate::text::bidi::ResolvedParagraph<'_>,
    end: usize,
    pieces: &mut Vec<Piece>,
) -> Result<Option<&'p str>, TextError> {
    let Some((offset, source)) = hyphenating(paragraph, automatic, end) else {
        return Ok(None);
    };
    let span_index = paragraph
        .spans()
        .partition_point(|span| span.range.end <= offset);
    let style = &paragraph.spans()[span_index].style;
    let rtl = bidi.level(offset).is_rtl();
    let preferred = character(style);
    if preferred.is_empty() {
        return Ok(Some(preferred));
    }
    let custom = style.line_breaking.hyphenate_character.is_some();
    let (mut runs, text): (Vec<GlyphRun>, _) = match context.shape(
        preferred,
        0..preferred.len(),
        rtl,
        style,
        if custom {
            MissingGlyphs::Substitute
        } else {
            MissingGlyphs::Reject
        },
    ) {
        Err(TextError::MissingGlyph { .. }) => (
            context.shape(
                HYPHEN_FALLBACK,
                0..HYPHEN_FALLBACK.len(),
                rtl,
                style,
                MissingGlyphs::Substitute,
            )?,
            HYPHEN_FALLBACK,
        ),
        result => (result?, preferred),
    };
    for run in &mut runs {
        run.source_range = source.clone();
        run.source_span = span_index;
    }
    let insertion = line_boxes::insertion(
        pieces.iter().enumerate().map(|(index, piece)| {
            let (range, rtl) = piece.source();
            (index, range, rtl)
        }),
        source.start,
    );
    pieces.splice(
        insertion..insertion,
        runs.into_iter().map(|run| Piece::Text(Box::new(run))),
    );
    Ok(Some(text))
}

pub(crate) fn text(source: &str, hyphen: Option<&str>) -> String {
    if !source.contains(SOFT_HYPHEN) && hyphen.is_none_or(str::is_empty) {
        return source.to_owned();
    }
    let mut text: String = source.chars().filter(|&ch| ch != SOFT_HYPHEN).collect();
    if let Some(hyphen) = hyphen {
        text.push_str(hyphen);
    }
    text
}
