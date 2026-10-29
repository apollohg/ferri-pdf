//! Ruby box generation and annotation pairing (CSS Ruby 1 §2) from HTML markup and the
//! ruby display values.
use crate::document::{BlockContent, Paragraph, StyledDocument};
use crate::resources::{FileResolver, ResourceBase};
use crate::style::{Display, UnicodeBidi};
use std::fmt::Write;

fn document(css: &str, body: &str) -> Result<StyledDocument, crate::document::DocumentError> {
    StyledDocument::parse(
        &format!("<style>{css}</style>{body}"),
        &ResourceBase::default(),
        &[],
        &FileResolver::default(),
    )
}

fn paragraph(document: &StyledDocument) -> &Paragraph {
    let mut pending: Vec<_> = document.root().children().iter().rev().collect();
    while let Some(child) = pending.pop() {
        match child {
            BlockContent::Inline(paragraph) => return paragraph,
            BlockContent::Block(block) | BlockContent::Positioned(block) => {
                pending.extend(block.children().iter().rev());
            }
            BlockContent::Events(_) => {}
        }
    }
    panic!("the fixture holds a paragraph")
}

fn annotation_text(paragraph: &Paragraph, block: usize) -> String {
    match paragraph.annotations()[block].block.children() {
        [BlockContent::Inline(contents)] => contents.text().to_owned(),
        children => panic!(
            "an annotation box holds one paragraph, not {}",
            children.len()
        ),
    }
}

/// The paragraph's rubies, one string each: segments `|`-separated as their bases' text, then
/// each level `over` or `under` with its annotations `text@bases`, white space as `␣@gap`,
/// hidden ones bracketed.
fn structure(css: &str, body: &str) -> (String, Vec<String>) {
    let document = document(css, body)
        .unwrap_or_else(|error| panic!("ruby fixture must format: {error}\n{css}\n{body}"));
    let paragraph = paragraph(&document);
    let text = paragraph.text();
    let rubies = paragraph
        .rubies()
        .iter()
        .map(|ruby| {
            ruby.segments
                .iter()
                .map(|segment| {
                    let bases: Vec<&str> = segment
                        .bases
                        .iter()
                        .map(|&base| &text[paragraph.boxes()[base].range.clone()])
                        .collect();
                    let mut described = format!("[{}]", bases.join(","));
                    for level in &segment.levels {
                        let annotations: Vec<String> = level
                            .annotations
                            .iter()
                            .map(|annotation| {
                                let content = if annotation.whitespace {
                                    "␣".to_owned()
                                } else {
                                    annotation
                                        .block
                                        .map(|block| annotation_text(paragraph, block))
                                        .unwrap_or_default()
                                };
                                let placed = format!(
                                    "{content}@{}-{}",
                                    annotation.bases.start, annotation.bases.end
                                );
                                if annotation.hidden {
                                    format!("({placed})")
                                } else {
                                    placed
                                }
                            })
                            .collect();
                        write!(
                            described,
                            " {} {}",
                            if level.under { "under" } else { "over" },
                            annotations.join(" ")
                        )
                        .unwrap();
                    }
                    described
                })
                .collect::<Vec<_>>()
                .join(" | ")
        })
        .collect();
    (text.to_owned(), rubies)
}

#[test]
fn html_text_runs_become_anonymous_bases_paired_in_segments() {
    assert_eq!(
        structure("", "<p><ruby>漢<rt>kan</rt>字<rt>ji</rt></ruby></p>"),
        (
            "漢字".into(),
            vec!["[漢] over kan@0-1 | [字] over ji@0-1".into()]
        ),
        "each run of base text before its annotations is a segment of its own (§2.2)"
    );
    assert_eq!(
        structure(
            "",
            "<p><ruby><rb>a</rb><rb>b</rb><rb>c</rb><rt>1</rt><rt>2</rt></ruby></p>"
        ),
        ("abc".into(), vec!["[a,b,c] over 1@0-1 2@1-2".into()]),
        "annotations pair with bases in order; the third base has an empty annotation (§2.3.2)"
    );
    let (text, rubies) = structure("", "<p><ruby><rb>a</rb><rt>1</rt><rt>2</rt></ruby></p>");
    assert_eq!(text, "a");
    assert_eq!(
        rubies,
        ["[a,] over 1@0-1 2@1-2"],
        "an annotation beyond the bases pairs with an empty anonymous base"
    );
}

#[test]
fn annotation_containers_are_levels_that_alternate_and_may_span() {
    assert_eq!(
        structure(
            "",
            "<p><ruby><rb>旧</rb><rb>金</rb><rb>山</rb><rt>jiù</rt><rt>jīn</rt><rt>shān</rt>\
             <rtc>San Francisco</rtc></ruby></p>"
        )
        .1,
        ["[旧,金,山] over jiù@0-1 jīn@1-2 shān@2-3 under San Francisco@0-3"],
        "a lone anonymous annotation spans its segment, and the second level alternates under"
    );
    assert_eq!(
        structure(
            "rtc{ruby-position:under}",
            "<p><ruby>a<rtc>x</rtc><rtc>y</rtc><rtc>z</rtc></ruby></p>"
        )
        .1,
        ["[a] under x@0-1 under y@0-1 under z@0-1"],
        "levels without alternate stack on their own side (§4.1)"
    );
    assert_eq!(
        structure("", "<p><ruby><rtc>x</rtc><rb>a</rb><rb>b</rb></ruby></p>").1,
        ["[] over x@0-1 | [a,b]"],
        "an annotation container first in its ruby pairs with an empty base container"
    );
}

#[test]
fn intra_ruby_white_space_is_discarded_or_kept_by_its_neighbours() {
    let (text, rubies) = structure(
        "",
        "<p><ruby> <rb>W</rb> <rb>W</rb> <rb>W</rb>\n <rt>World</rt> <rt>Wide</rt> <rt>Web</rt> </ruby></p>",
    );
    assert_eq!(
        text, "W W W",
        "inter-base white space stays in the base level; leading, trailing and inter-level \
         white space goes (§2.5)"
    );
    assert_eq!(
        rubies,
        ["[W,W,W] over World@0-1 ␣@1-1 Wide@1-2 ␣@2-2 Web@2-3"],
        "inter-annotation white space pairs with the gap between the bases"
    );
    let (text, rubies) = structure(
        "",
        "<p><ruby><rb>a</rb><rt>x</rt> <rb>b</rb><rt>y</rt></ruby></p>",
    );
    assert_eq!(
        text, "a b",
        "inter-segment white space joins the base level"
    );
    assert_eq!(rubies, ["[a] over x@0-1 | [b] over y@0-1"]);
}

#[test]
fn annotations_hide_by_visibility_or_matching_their_base_unless_merged() {
    let markup = "<p><ruby><rb>振</rb><rb>り</rb><rb>仮</rb><rb>名</rb><rp>(</rp>\
                  <rt>ふ</rt><rt>り</rt><rt>が</rt><rt>な</rt><rp>)</rp></ruby></p>";
    assert_eq!(
        structure("", markup),
        (
            "振り仮名".into(),
            vec!["[振,り,仮,名] over ふ@0-1 (り@1-2) が@2-3 な@3-4".into()]
        ),
        "rp generates no box, and an annotation reading as its base is auto-hidden (§2.4)"
    );
    assert_eq!(
        structure("ruby{ruby-merge:merge}", markup).1,
        ["[振,り,仮,名] over ふ@0-1 り@1-2 が@2-3 な@3-4"],
        "merging disables hiding"
    );
    assert_eq!(
        structure(
            ".easy{visibility:collapse}",
            "<p><ruby><rb>昆</rb><rb>虫</rb><rt>こん</rt><rt class=easy>ちゅう</rt></ruby></p>"
        )
        .1,
        ["[昆,虫] over こん@0-1 (ちゅう@1-2)"]
    );
}

#[test]
fn misparented_internal_boxes_get_anonymous_ruby_containers() {
    let (text, rubies) = structure("", "<p>漢<rt>あ</rt>字</p>");
    assert_eq!(text, "漢字");
    assert_eq!(
        rubies,
        ["[] over あ@0-1"],
        "an annotation outside a ruby is wrapped with an empty base (§2.2)"
    );
    let (text, rubies) = structure(
        "span{display:ruby-base}i{display:ruby-text}",
        "<p><span>漢</span><i>かん</i> <span>字</span><i>じ</i></p>",
    );
    assert_eq!(text, "漢 字");
    assert_eq!(
        rubies,
        ["[漢] over かん@0-1 | [字] over じ@0-1"],
        "consecutive misparented boxes and the white space between share one container"
    );
}

#[test]
fn ruby_display_values_build_containers_from_any_element() {
    let (text, rubies) = structure(
        "div{display:block ruby}b{display:ruby-text}",
        "<div>漢<b>かん</b></div>",
    );
    assert_eq!(text, "漢");
    assert_eq!(
        rubies,
        ["[漢] over かん@0-1"],
        "a block-level ruby holds an inline-level ruby container (§2.1.2)"
    );
    let (_, rubies) = structure(
        "b{display:ruby}i{display:ruby-text}",
        "<p><b>漢<span>字</span><i>a</i></b></p>",
    );
    assert_eq!(
        rubies,
        ["[漢字] over a@0-1"],
        "inline boxes join the anonymous base around them"
    );
}

#[test]
fn ruby_boxes_isolate_inlinify_and_stop_decorations() {
    let document = document(
        "u{text-decoration:underline}rtc{text-decoration:overline}b{display:block}",
        "<p><u><ruby dir=ltr>漢<b>x</b><rt>a</rt><rtc>b</rtc></ruby></u></p>",
    )
    .unwrap();
    let paragraph = paragraph(&document);
    let ruby = &paragraph.rubies()[0];
    let container = &paragraph.boxes()[ruby.container].style;
    assert_eq!(container.unicode_bidi, UnicodeBidi::Isolate);
    let base = &paragraph.boxes()[ruby.segments[0].bases[0]].style;
    assert_eq!(
        base.unicode_bidi,
        UnicodeBidi::Isolate,
        "anonymous bases are isolated as the internal boxes are (§3.5)"
    );
    assert_eq!(
        paragraph.atomics()[0].block.style().display,
        Display::InlineBlock,
        "a block inside a ruby base is inlinified (§2.2)"
    );
    let levels = &ruby.segments[0].levels;
    let rt = &levels[0].annotations[0];
    assert!(
        rt.style.text_decorations.propagated.is_empty(),
        "the underline does not propagate to annotations (§4.4)"
    );
    let wrapper = |annotation: &crate::document::ruby::Annotation| {
        paragraph.annotations()[annotation.block.unwrap()]
            .block
            .style()
            .text_decorations
            .propagated
            .len()
    };
    assert_eq!(
        wrapper(rt),
        0,
        "an anonymous container's annotations draw none"
    );
    assert_eq!(
        wrapper(&levels[1].annotations[0]),
        1,
        "an annotation container's own overline decorates its annotations"
    );
}

#[test]
fn annotations_take_the_ua_size_feature_and_line_height() {
    let document = document(
        "p{font-size:20pt}",
        "<p><ruby>漢<rt>a</rt><rtc><rt>b</rt></rtc></ruby></p>",
    )
    .unwrap();
    let paragraph = paragraph(&document);
    let levels = &paragraph.rubies()[0].segments[0].levels;
    let rt = &levels[0].annotations[0].style;
    assert_eq!(
        rt.size, 10.0,
        "an rt outside rtc is half size (CSS Ruby 1 §A.1)"
    );
    assert!(
        rt.font_features
            .east_asian
            .contains(&crate::style::RUBY_FEATURE),
        "annotations take the OpenType ruby feature"
    );
    assert_eq!(rt.line_breaking.justify, crate::style::TextJustify::Ruby);
    assert!(!rt.white_space.wraps());
    assert_eq!(
        levels[1].annotations[0].style.size, 10.0,
        "an rt inside rtc keeps the container's half size"
    );
    assert_eq!(levels[1].style.size, 10.0);
}

#[test]
fn inter_character_annotations_report_their_vertical_layout() {
    let error = document(
        "rtc{ruby-position:inter-character}",
        "<p><ruby>漢<rtc>ㄏㄢˋ</rtc></ruby></p>",
    )
    .err()
    .expect("inter-character ruby is not laid out");
    assert!(
        error.to_string().contains("inter-character"),
        "the error names the value: {error}"
    );
}

#[test]
fn forced_breaks_inside_annotations_are_collapsible_segment_breaks() {
    let (_, rubies) = structure(
        "rt{white-space:pre}",
        "<p><ruby>漢<rt>a<br>b\nc</rt></ruby></p>",
    );
    assert_eq!(
        rubies,
        ["[漢] over a b c@0-1"],
        "a forced break and a preserved newline both collapse to a space (§2.2)"
    );
}
