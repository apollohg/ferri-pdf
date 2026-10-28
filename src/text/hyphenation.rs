use crate::document::Paragraph;
use crate::style::Hyphens;
use crate::text::shaping::MissingGlyphs;
use crate::text::{GlyphRun, Piece, TextContext, TextError, line_boxes};

pub(super) const SOFT_HYPHEN: char = '\u{00ad}';
const HYPHEN: &str = "\u{2010}";
const ARMENIAN_HYPHEN: &str = "\u{058a}";
const HYPHEN_FALLBACK: &str = "-";

pub(super) fn is_soft_break(paragraph: &Paragraph, end: usize) -> bool {
    end < paragraph.text().len() && paragraph.text()[..end].ends_with(SOFT_HYPHEN)
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
        .is_none_or(|span| span.style.hyphens != Hyphens::None)
}

/// The conditional glyph belongs to the soft hyphen's own span, including its bidi level.
pub(super) fn decorate(
    context: &TextContext<'_>,
    paragraph: &Paragraph,
    bidi: &crate::text::bidi::ResolvedParagraph<'_>,
    end: usize,
    pieces: &mut Vec<Piece>,
) -> Result<Option<&'static str>, TextError> {
    if !is_soft_break(paragraph, end) || !allowed(paragraph, end) {
        return Ok(None);
    }
    let offset = end - SOFT_HYPHEN.len_utf8();
    let span_index = paragraph
        .spans()
        .partition_point(|span| span.range.end <= offset);
    let style = &paragraph.spans()[span_index].style;
    let rtl = bidi.level(offset).is_rtl();
    let preferred = if style
        .language
        .split('-')
        .next()
        .is_some_and(|language| language.eq_ignore_ascii_case("hy"))
    {
        ARMENIAN_HYPHEN
    } else {
        HYPHEN
    };
    let (mut runs, text): (Vec<GlyphRun>, _) = match context.shape(
        preferred,
        0..preferred.len(),
        rtl,
        style,
        MissingGlyphs::Reject,
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
        run.source_range = offset..end;
        run.source_span = span_index;
    }
    let insertion = line_boxes::insertion(
        pieces.iter().enumerate().map(|(index, piece)| {
            let (range, rtl) = piece.source();
            (index, range, rtl)
        }),
        offset,
    );
    pieces.splice(insertion..insertion, runs.into_iter().map(Piece::Text));
    Ok(Some(text))
}

pub(super) fn text(source: &str, hyphen: Option<&str>) -> String {
    if !source.contains(SOFT_HYPHEN) {
        return source.to_owned();
    }
    let mut text: String = source.chars().filter(|&ch| ch != SOFT_HYPHEN).collect();
    if let Some(hyphen) = hyphen {
        text.push_str(hyphen);
    }
    text
}
