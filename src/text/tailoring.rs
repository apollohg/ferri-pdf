//! CSS Text 4 tailoring of UAX #14 soft wrap opportunities: `word-break` (§6.1) and
//! `line-break` (§6.2). Reclassified characters are replaced by stand-ins of their new line
//! breaking class, so the pair rules of UAX #14 still decide every opportunity.
use crate::document::Paragraph;
use crate::style::{LineBreak, Style, WordBreak};
use icu_properties::{
    CodePointMapData,
    props::{EastAsianWidth, GeneralCategory, GeneralCategoryGroup},
};
use unicode_linebreak::{BreakClass, BreakOpportunity, break_property};
use unicode_segmentation::UnicodeSegmentation;

/// Stand-ins of the classes characters are treated as.
const IDEOGRAPHIC: char = '\u{4e00}';
const COMBINING: char = '\u{0300}';
/// Primary language subtags of the Chinese and Japanese writing systems.
const CHINESE_OR_JAPANESE: [&str; 2] = ["zh", "ja"];
/// §6.2: breaks before these are allowed for `normal` and `loose` in Chinese or Japanese.
const CJK_HYPHENS: [char; 2] = ['\u{301c}', '\u{30a0}'];
/// §6.2: breaks before these are allowed for `loose` after an ideographic character.
const HYPHENS: [char; 2] = ['\u{2010}', '\u{2013}'];
/// §6.2: breaks before these are allowed for `loose`.
const ITERATION_MARKS: [char; 6] = [
    '\u{3005}', '\u{303b}', '\u{309d}', '\u{309e}', '\u{30fd}', '\u{30fe}',
];
/// §6.2: breaks before these are allowed for `loose` in Chinese or Japanese.
const CENTERED_PUNCTUATION: [char; 10] = [
    '\u{30fb}', '\u{ff1a}', '\u{ff1b}', '\u{ff65}', '\u{203c}', '\u{2047}', '\u{2048}', '\u{2049}',
    '\u{ff01}', '\u{ff1f}',
];

/// The soft wrap opportunities of `paragraph` under each character's `word-break` and
/// `line-break`, with UAX #14 mandatory breaks, in offset order.
pub(super) fn linebreaks(paragraph: &Paragraph) -> Vec<(usize, BreakOpportunity)> {
    let text = paragraph.text();
    let categories = CodePointMapData::<GeneralCategory>::new();
    let base =
        |offset: usize, ch: char| base_stand_in(ch, categories.get(ch), paragraph.style_at(offset));
    let mut breaks = stand_in_breaks(text, base)
        .unwrap_or_else(|| unicode_linebreak::linebreaks(text).collect());
    if paragraph.spans().iter().any(|span| {
        span.style.line_breaking.line_break == LineBreak::Loose || chinese_or_japanese(&span.style)
    }) {
        add_strictness_breaks(paragraph, &mut breaks, &base);
    }
    if paragraph
        .spans()
        .iter()
        .any(|span| span.style.line_breaking.word_break == WordBreak::KeepAll)
    {
        breaks.retain(|&(offset, opportunity)| {
            opportunity == BreakOpportunity::Mandatory || !kept_together(paragraph, offset)
        });
    }
    if paragraph
        .spans()
        .iter()
        .any(|span| span.style.line_breaking.line_break == LineBreak::Anywhere)
    {
        breaks.extend(
            text.grapheme_indices(true)
                .skip(1)
                .map(|(offset, _)| offset)
                .filter(|&offset| {
                    anywhere(paragraph.style_at(offset - 1)) || anywhere(paragraph.style_at(offset))
                })
                .map(|offset| (offset, BreakOpportunity::Allowed)),
        );
        breaks.sort_unstable_by_key(|&(offset, opportunity)| {
            (offset, opportunity == BreakOpportunity::Allowed)
        });
        breaks.dedup_by_key(|entry| entry.0);
    }
    breaks
}

/// §6.1: `break-all` treats letters, numbers and SA characters as ideographic. Other values
/// except `manual` break SA text between letters, since no lexical resource is available
/// (§6.1.1.3); SA marks keep attaching to their base.
fn base_stand_in(ch: char, category: GeneralCategory, style: &Style) -> Option<char> {
    let class = break_property(ch as u32);
    let mark = GeneralCategoryGroup::Mark.contains(category);
    match style.line_breaking.word_break {
        WordBreak::Manual => None,
        WordBreak::BreakAll
            if !mark
                && !matches!(
                    class,
                    BreakClass::HangulLJamo | BreakClass::HangulVJamo | BreakClass::HangulTJamo
                )
                && (letter(category)
                    || matches!(
                        class,
                        BreakClass::Numeric
                            | BreakClass::Alphabetic
                            | BreakClass::Ambiguous
                            | BreakClass::ComplexContext
                    )) =>
        {
            Some(IDEOGRAPHIC)
        }
        _ if class == BreakClass::ComplexContext => {
            Some(if mark { COMBINING } else { IDEOGRAPHIC })
        }
        _ => None,
    }
}

/// UAX #14 opportunities of `text` with characters replaced by `stand_in`, or `None` when no
/// character is replaced. Stand-ins may differ in UTF-8 length, so offsets map back.
fn stand_in_breaks(
    text: &str,
    stand_in: impl Fn(usize, char) -> Option<char>,
) -> Option<Vec<(usize, BreakOpportunity)>> {
    let mut proxy = String::with_capacity(text.len());
    let mut origins = Vec::new();
    let mut replaced = false;
    for (offset, ch) in text.char_indices() {
        let replacement = stand_in(offset, ch);
        replaced |= replacement.is_some();
        origins.push((proxy.len(), offset));
        proxy.push(replacement.unwrap_or(ch));
    }
    if !replaced {
        return None;
    }
    origins.push((proxy.len(), text.len()));
    Some(
        unicode_linebreak::linebreaks(&proxy)
            .map(|(offset, opportunity)| {
                let index = origins.partition_point(|&(proxy_offset, _)| proxy_offset < offset);
                (origins[index].1, opportunity)
            })
            .collect(),
    )
}

/// Where a break is decided before or after one character.
#[derive(Clone, Copy, PartialEq)]
enum Side {
    Before,
    After,
}

/// §6.2 requirements for `normal` and `loose`, beyond the strict UAX #14 baseline: each
/// designated character is treated as ideographic, and only the break on its designated
/// side is taken, so opening punctuation and glue still prohibit it.
fn add_strictness_breaks(
    paragraph: &Paragraph,
    breaks: &mut Vec<(usize, BreakOpportunity)>,
    base: &impl Fn(usize, char) -> Option<char>,
) {
    let text = paragraph.text();
    let widths = CodePointMapData::<EastAsianWidth>::new();
    let categories = CodePointMapData::<GeneralCategory>::new();
    let mut designated: Vec<(usize, Side)> = Vec::new();
    let mut previous: Option<(char, Option<char>)> = None;
    for (offset, ch) in text.char_indices() {
        let style = paragraph.style_at(offset);
        let line_break = style.line_breaking.line_break;
        let loose = line_break == LineBreak::Loose;
        let cjk = chinese_or_japanese(style) && line_break != LineBreak::Strict;
        let class = break_property(ch as u32);
        let wide = matches!(
            widths.get(ch),
            EastAsianWidth::Ambiguous | EastAsianWidth::Fullwidth | EastAsianWidth::Wide
        );
        let previous_class =
            previous.map(|(ch, stand_in)| break_property(stand_in.unwrap_or(ch) as u32));
        let before = (cjk && CJK_HYPHENS.contains(&ch))
            || (loose
                && (class == BreakClass::ConditionalJapaneseStarter
                    || ITERATION_MARKS.contains(&ch)
                    || (class == BreakClass::Inseparable
                        && previous_class == Some(BreakClass::Inseparable))
                    || (HYPHENS.contains(&ch) && previous_class == Some(BreakClass::Ideographic))
                    || (cjk
                        && (CENTERED_PUNCTUATION.contains(&ch)
                            || (class == BreakClass::Postfix && wide)))));
        if before {
            designated.push((offset, Side::Before));
        } else if loose && cjk && class == BreakClass::Prefix && wide {
            designated.push((offset, Side::After));
        }
        if !GeneralCategoryGroup::Mark.contains(categories.get(ch)) {
            previous = Some((ch, base(offset, ch)));
        }
    }
    if designated.is_empty() {
        return;
    }
    let Some(tailored) = stand_in_breaks(text, |offset, ch| {
        if designated
            .binary_search_by_key(&offset, |&(start, _)| start)
            .is_ok()
        {
            Some(IDEOGRAPHIC)
        } else {
            base(offset, ch)
        }
    }) else {
        return;
    };
    // Designated characters are in offset order, so their sides are too.
    let targets: Vec<usize> = designated
        .iter()
        .map(|&(offset, side)| match side {
            Side::Before => offset,
            Side::After => offset + text[offset..].chars().next().map_or(0, char::len_utf8),
        })
        .collect();
    breaks.extend(tailored.into_iter().filter(|&(offset, opportunity)| {
        opportunity == BreakOpportunity::Allowed && targets.binary_search(&offset).is_ok()
    }));
    breaks.sort_unstable_by_key(|&(offset, opportunity)| {
        (offset, opportunity == BreakOpportunity::Allowed)
    });
    breaks.dedup_by_key(|entry| entry.0);
}

/// §6.1 `keep-all`: no implicit opportunity between two letters, or characters of the NU,
/// AL, AI or ID classes, unless `line-break: anywhere` requires one.
fn kept_together(paragraph: &Paragraph, offset: usize) -> bool {
    let text = paragraph.text();
    let categories = CodePointMapData::<GeneralCategory>::new();
    let Some((start, before)) = text[..offset]
        .char_indices()
        .rev()
        .find(|&(_, ch)| !GeneralCategoryGroup::Mark.contains(categories.get(ch)))
    else {
        return false;
    };
    let Some(after) = text[offset..].chars().next() else {
        return false;
    };
    let joins = |ch: char| {
        letter(categories.get(ch))
            || matches!(
                break_property(ch as u32),
                BreakClass::Numeric
                    | BreakClass::Alphabetic
                    | BreakClass::Ambiguous
                    | BreakClass::Ideographic
            )
    };
    let [left, right] = [paragraph.style_at(start), paragraph.style_at(offset)];
    joins(before)
        && joins(after)
        && [left, right]
            .iter()
            .any(|style| style.line_breaking.word_break == WordBreak::KeepAll)
        && !anywhere(left)
        && !anywhere(right)
}

/// CSS Text 3 §1.4: typographic letter units belong to the Letter or Number categories.
fn letter(category: GeneralCategory) -> bool {
    GeneralCategoryGroup::Letter.contains(category)
        || GeneralCategoryGroup::Number.contains(category)
}

fn anywhere(style: &Style) -> bool {
    style.line_breaking.line_break == LineBreak::Anywhere
}

fn chinese_or_japanese(style: &Style) -> bool {
    CHINESE_OR_JAPANESE
        .iter()
        .any(|subtag| crate::style::has_primary_language(&style.language, subtag))
}
