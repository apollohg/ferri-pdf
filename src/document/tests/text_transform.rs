use crate::document::{BlockContent, Paragraph, StyledDocument};
use crate::resources::{FileResolver, ResourceBase, Url};
use std::fmt::Write;

fn document(css: &str, body: &str) -> StyledDocument {
    let base: ResourceBase = Url::parse("https://example.org/transform.html")
        .unwrap()
        .into();
    StyledDocument::parse(
        &format!("<style>{css}</style>{body}"),
        &base,
        &[],
        &FileResolver::default(),
    )
    .unwrap_or_else(|error| panic!("transform fixture: {error}; CSS={css}; body={body}"))
}

fn paragraphs(document: &StyledDocument) -> Vec<&Paragraph> {
    let mut pending: Vec<_> = document.root().children().iter().rev().collect();
    let mut result = Vec::new();
    while let Some(child) = pending.pop() {
        match child {
            BlockContent::Inline(paragraph) => result.push(paragraph),
            BlockContent::Block(block) | BlockContent::Positioned(block) => {
                pending.extend(block.children().iter().rev());
            }
            BlockContent::Events(_) => {}
        }
    }
    result
}

#[test]
fn width_and_kana_follow_casing_without_normalizing_unrelated_characters() {
    let document = document(
        "p{text-transform:full-size-kana uppercase full-width}",
        "<p>straße ｧｶﾞ ぁㇰ ① ﬁ</p>",
    );
    assert_eq!(
        paragraphs(&document)[0].text(),
        "ＳＴＲＡＳＳＥ アカ\u{3099} あク ① ＦＩ"
    );
}

#[test]
fn only_preserved_spaces_receive_ideographic_width() {
    for (mode, expected) in [
        ("normal", "Ａ Ｂ"),
        ("nowrap", "Ａ Ｂ"),
        ("pre-line", "Ａ Ｂ"),
        ("pre", "Ａ\u{3000}\u{3000}Ｂ"),
        ("pre-wrap", "Ａ\u{3000}\u{3000}Ｂ"),
        ("break-spaces", "Ａ\u{3000}\u{3000}Ｂ"),
    ] {
        let document = document(
            &format!("p{{white-space:{mode};text-transform:full-width}}"),
            "<p>A  B</p>",
        );
        assert_eq!(
            paragraphs(&document)[0].text(),
            expected,
            "white-space:{mode}"
        );
    }
}

#[test]
fn transformed_offsets_still_cover_links_boxes_breaks_and_atomics() {
    let document = document(
        "p{text-transform:uppercase full-width}a{display:contents}span{border:1px solid}i{display:inline-block;text-transform:none}",
        "<p>ß<a href='#end'><span>a</span></a><wbr><i>X</i><br clear=all>b</p><div id=end></div>",
    );
    let paragraph = paragraphs(&document)[0];
    let text = paragraph.text();
    assert!(text.starts_with("ＳＳＡ"), "{text:?}");
    let link = &paragraph.links()[0];
    assert_eq!(&text[link.range.clone()], "Ａ");
    assert!(
        paragraph
            .boxes()
            .iter()
            .any(|item| &text[item.range.clone()] == "Ａ")
    );
    let after_letters = "ＳＳＡ".len();
    assert_eq!(paragraph.optional_breaks(), &[after_letters]);
    assert_eq!(paragraph.atomics()[0].offset, after_letters);
    let (offset, ch) = text
        .char_indices()
        .find(|(_, ch)| matches!(ch, '\u{2028}' | '\n'))
        .unwrap();
    let after_break = offset + ch.len_utf8();
    assert_eq!(
        paragraph.clearance_at(after_break).0,
        crate::style::Clear::Both
    );
}

#[test]
fn every_unicode_width_mapping_matches_its_original_decomposition_record() {
    let mut html = String::from("<p>");
    let mut expected = String::new();
    const DECOMPOSITION_FIELD: usize = 5;
    for record in include_str!("fixtures/width-decompositions.txt").lines() {
        let fields: Vec<_> = record.split(';').collect();
        let mapping: Vec<_> = fields[DECOMPOSITION_FIELD].split_whitespace().collect();
        let codepoint = u32::from_str_radix(fields[0], 16).unwrap();
        let decomposition = u32::from_str_radix(mapping[1], 16).unwrap();
        let (source, target) = match mapping[0] {
            "<wide>" => (decomposition, codepoint),
            "<narrow>" => (codepoint, decomposition),
            tag => panic!("unexpected width record tag: {tag}"),
        };
        let _ = write!(html, "&#x{source:X};");
        expected.push(char::from_u32(target).unwrap());
    }
    html.push_str("</p>");
    let document = document("p{white-space:pre;text-transform:full-width}", &html);
    assert_eq!(paragraphs(&document)[0].text(), expected);
}

#[test]
fn every_pinned_small_kana_mapping_is_applied() {
    let mut html = String::from("<p>");
    let mut expected = String::new();
    for record in include_str!("fixtures/small-kana.txt").lines() {
        let (source, target) = record.split_once(';').unwrap();
        let _ = write!(html, "&#x{source};");
        expected.push(char::from_u32(u32::from_str_radix(target, 16).unwrap()).unwrap());
    }
    html.push_str("</p>");
    let document = document("p{text-transform:full-size-kana}", &html);
    assert_eq!(paragraphs(&document)[0].text(), expected);
}

#[test]
fn inherited_flags_and_initial_reset_keep_case_context_and_generated_text() {
    let document = document(
        "section{text-transform:lowercase full-width full-size-kana}p::before{content:'ｧA'}i{text-transform:initial}",
        "<section lang=tr><p>I<span>̇</span> I <i>Ａｧ</i> ｧ</p></section>",
    );
    assert_eq!(paragraphs(&document)[0].text(), "アａｉ ı Ａｧ ア");
}

#[test]
fn full_width_does_not_widen_expanded_tabs() {
    for mode in ["pre", "break-spaces"] {
        let document = document(
            &format!("p{{white-space:{mode};text-transform:uppercase full-width;tab-size:3}}"),
            "<p>ß\t B</p>",
        );
        let paragraph = paragraphs(&document)[0];
        assert_eq!(paragraph.text(), "ＳＳ   \u{3000}Ｂ");
        assert_eq!(paragraph.expanded_tabs().len(), 1);
        assert_eq!(paragraph.expanded_tabs()[0], "ＳＳ".len().."ＳＳ   ".len());
    }
}

#[test]
fn math_auto_maps_single_text_nodes_and_preserves_long_identifiers() {
    let document = document(
        "span{text-transform:math-auto}",
        "<p><span>x</span><span>exp</span><span>h</span><span>α</span><span>∞</span></p>",
    );
    assert_eq!(
        paragraphs(&document)[0].text(),
        "𝑥expℎ𝛼∞",
        "MathML Core §4.2 maps a single source text node, including non-BMP mappings and the Planck constant exception"
    );
}
