use super::common;

use crate::document::{BlockContent, Paragraph, StyledDocument};
use crate::style::TextAlign;
use crate::text::{AtomicMetrics, Line, TextContext, TextError};
use common::{assert_pt, local_options};
use ferri_pdf_resources::FileResolver;

const PROFILE: &str = "@font-face{font-family:Fixed;src:url('../assets/DejaVuSans.ttf')}html,body,p{margin:0;padding:0;font:10pt/10pt Fixed}";

fn document(css: &str, body: &str) -> StyledDocument {
    StyledDocument::parse(
        &format!(
            "<!doctype html><style>{}{PROFILE}{css}</style>{body}",
            common::PROFILE
        ),
        &local_options().base,
        &[],
        &FileResolver::default(),
    )
    .unwrap()
}

fn paragraphs(document: &StyledDocument) -> Vec<&Paragraph> {
    let mut pending = document.root().children().iter().rev().collect::<Vec<_>>();
    let mut paragraphs = Vec::new();
    while let Some(child) = pending.pop() {
        match child {
            BlockContent::Events(_) => {}
            BlockContent::Inline(paragraph) => paragraphs.push(paragraph),
            BlockContent::Block(block) | BlockContent::Positioned(block) => {
                pending.extend(block.children().iter().rev());
            }
        }
    }
    paragraphs
}

fn glyphs_from(lines: &[Line], start: usize) -> Vec<(usize, u32)> {
    let mut glyphs = lines
        .iter()
        .flat_map(|line| &line.runs)
        .filter(|run| run.source_range.start >= start)
        .flat_map(|run| {
            run.glyphs()
                .iter()
                .map(|glyph| (run.source_range.start + glyph.text_range.start, glyph.id))
        })
        .collect::<Vec<_>>();
    glyphs.sort_unstable();
    glyphs
}

#[test]
fn resumed_lines_use_new_widths_and_absolute_source_checkpoints() {
    let words = (0..12)
        .map(|index| format!("A{index:03}"))
        .collect::<Vec<_>>();
    let document = document("", &format!("<p>{}</p>", words.join(" ")));
    let paragraphs = paragraphs(&document);
    let source = paragraphs[0];
    let resolver = FileResolver::default();
    let context = TextContext::new(&document.styles().fonts, &resolver);
    let prepared = context.prepare_paragraph(source);
    let mut cursor = prepared.start();
    for (width, range, expected) in [
        (100.0, 0..15, words[..3].join(" ")),
        (200.0, 15..45, words[3..9].join(" ")),
        (100.0, 45..59, words[9..].join(" ")),
    ] {
        let mut lines = prepared.lines(cursor, width, &[]).unwrap();
        let line = lines.next().unwrap().unwrap();
        assert_eq!(
            line.range, range,
            "width {width} consumes original source bytes including trimmed spaces"
        );
        assert_eq!(line.text, expected);
        assert_pt(line.height, 10.0);
        cursor = lines.checkpoint();
    }
    assert!(prepared.lines(cursor, 300.0, &[]).unwrap().next().is_none());
}

#[test]
fn resumed_arabic_keeps_full_word_shaping_absolute_spans_and_unboxed_link_identity() {
    let document = document(
        "p{overflow-wrap:anywhere}a{display:contents}",
        "<p dir=rtl>A000 A001 A002 <a href='https://example.com/'><span>سسسس</span></a></p>",
    );
    let paragraphs = paragraphs(&document);
    let source = paragraphs[0];
    assert_eq!(
        source.links().len(),
        1,
        "display:contents links use source text ranges"
    );
    let resolver = FileResolver::default();
    let context = TextContext::new(&document.styles().fonts, &resolver);
    let prepared = context.prepare_paragraph(source);
    let arabic_start = source.text().find('س').unwrap();
    let reference = context.layout_paragraph(source, 300.0, &[]).unwrap();
    let mut prefix = prepared.lines(prepared.start(), 100.0, &[]).unwrap();
    let line = prefix.next().unwrap().unwrap();
    assert_eq!(line.range.end, arabic_start);
    let mut cursor = prefix.checkpoint();
    let mut resumed = Vec::new();
    for width in [6.0, 12.0, 6.0, 30.0] {
        let mut lines = prepared.lines(cursor, width, &[]).unwrap();
        if let Some(line) = lines.next() {
            let line = line.unwrap();
            assert!(
                !line.links.is_empty(),
                "each resumed Arabic fragment retains its link: text {:?}, range {:?}, runs {:?}, source links {:?}",
                line.text,
                line.range,
                line.runs
                    .iter()
                    .map(|run| (&run.text, run.source_range.clone(), run.x, run.width))
                    .collect::<Vec<_>>(),
                source
                    .links()
                    .iter()
                    .map(|link| (link.link, link.range.clone()))
                    .collect::<Vec<_>>()
            );
            assert!(
                line.links
                    .iter()
                    .all(|link| link.link == source.links()[0].link)
            );
            assert!(
                line.runs
                    .iter()
                    .all(|run| run.source_range.start >= arabic_start
                        && run.source_span < source.spans().len())
            );
            resumed.push(line);
            cursor = lines.checkpoint();
        }
    }
    assert!(prepared.lines(cursor, 30.0, &[]).unwrap().next().is_none());
    assert_eq!(
        resumed
            .iter()
            .map(|line| line.text.as_str())
            .collect::<String>(),
        "سسسس"
    );
    assert_eq!(
        glyphs_from(&resumed, arabic_start),
        glyphs_from(&reference, arabic_start),
        "resuming inside a word must preserve original joining forms, not reshape a sliced paragraph"
    );
}

#[test]
fn a_resumed_numeric_line_keeps_bidi_embedding_from_preceding_source() {
    let document = document("", "<p>A000 A001 A002 \u{202b}אבג 12 34 56 78\u{202c}</p>");
    let paragraphs = paragraphs(&document);
    let source = paragraphs[0];
    let resolver = FileResolver::default();
    let context = TextContext::new(&document.styles().fonts, &resolver);
    let prepared = context.prepare_paragraph(source);
    let numbers = source.text().find("12").unwrap();
    let mut prefix = prepared.lines(prepared.start(), 20.0, &[]).unwrap();
    let mut consumed = 0;
    while consumed < numbers {
        let line = prefix.next().unwrap().unwrap();
        consumed = line.range.end;
    }
    assert_eq!(consumed, numbers);
    let mut next = prepared.lines(prefix.checkpoint(), 60.0, &[]).unwrap();
    let line = next.next().unwrap().unwrap();
    assert_eq!(line.range, numbers..source.text().find("78").unwrap());
    assert_eq!(line.text, "12 34 56");
    let visual = line
        .runs
        .iter()
        .flat_map(|run| run.text.split_ascii_whitespace())
        .filter_map(|text| text.parse::<u32>().ok())
        .collect::<Vec<_>>();
    assert_eq!(
        visual,
        [56, 34, 12],
        "the earlier RLE is still active; a sliced LTR paragraph would reorder these as 12,34,56"
    );
}

#[test]
fn soft_hyphens_and_emergency_graphemes_resume_at_consumed_source_boundaries() {
    let document = document(
        "p{hyphens:manual}#comb{overflow-wrap:anywhere}",
        "<p>A000 A001 A002 ab\u{ad}cd A003</p><p id=comb>e\u{301}e\u{301}e\u{301}e\u{301}</p>",
    );
    let paragraphs = paragraphs(&document);
    let resolver = FileResolver::default();
    let context = TextContext::new(&document.styles().fonts, &resolver);
    let prepared = context.prepare_paragraph(paragraphs[0]);
    let mut first = prepared.lines(prepared.start(), 100.0, &[]).unwrap();
    assert_eq!(first.next().unwrap().unwrap().text, "A000 A001 A002");
    let mut hyphen = prepared.lines(first.checkpoint(), 18.0, &[]).unwrap();
    let line = hyphen.next().unwrap().unwrap();
    assert_eq!(line.text, "ab\u{2010}");
    assert_eq!(line.range, 15..19);
    let mut end = prepared.lines(hyphen.checkpoint(), 100.0, &[]).unwrap();
    assert_eq!(end.next().unwrap().unwrap().text, "cd A003");
    assert!(end.next().is_none());
    let prepared = context.prepare_paragraph(paragraphs[1]);
    let mut first = prepared.lines(prepared.start(), 1.0, &[]).unwrap();
    let line = first.next().unwrap().unwrap();
    assert_eq!((line.text.as_str(), line.range), ("e\u{301}", 0..3));
    let mut rest = prepared.lines(first.checkpoint(), 30.0, &[]).unwrap();
    let line = rest.next().unwrap().unwrap();
    assert_eq!(
        (line.text.as_str(), line.range),
        ("e\u{301}e\u{301}e\u{301}", 3..12)
    );
    assert!(rest.next().is_none());
}

#[test]
fn page_local_iteration_does_not_turn_a_wrapped_line_into_a_paragraph_last_line() {
    let document = document(
        "p{text-align:justify;text-align-last:right}",
        "<p>A000 A001 A002 A003<br>A004 A005</p>",
    );
    let paragraphs = paragraphs(&document);
    let resolver = FileResolver::default();
    let context = TextContext::new(&document.styles().fonts, &resolver);
    let prepared = context.prepare_paragraph(paragraphs[0]);
    let mut first = prepared.lines(prepared.start(), 100.0, &[]).unwrap();
    assert_eq!(first.next().unwrap().unwrap().align, TextAlign::Justify);
    let mut second = prepared.lines(first.checkpoint(), 200.0, &[]).unwrap();
    let line = second.next().unwrap().unwrap();
    assert_eq!((line.text.as_str(), line.align), ("A003", TextAlign::Right));
    let mut last = prepared.lines(second.checkpoint(), 100.0, &[]).unwrap();
    let line = last.next().unwrap().unwrap();
    assert_eq!(
        (line.text.as_str(), line.align),
        ("A004 A005", TextAlign::Right)
    );
    assert!(last.next().is_none());
}

#[test]
fn empty_inline_edges_are_consumed_once_even_when_the_next_page_changes_width() {
    let document = document("span{padding:2pt;border:1pt solid}", "<p><span></span></p>");
    let paragraphs = paragraphs(&document);
    assert_eq!(paragraphs.len(), 1);
    assert_eq!(paragraphs[0].text(), "");
    assert!(!paragraphs[0].boxes().is_empty());
    let resolver = FileResolver::default();
    let context = TextContext::new(&document.styles().fonts, &resolver);
    let prepared = context.prepare_paragraph(paragraphs[0]);
    let mut first = prepared.lines(prepared.start(), 20.0, &[]).unwrap();
    let line = first.next().unwrap().unwrap();
    assert_eq!(line.range, 0..0);
    assert!(line.height > 0.0);
    assert!(
        prepared
            .lines(first.checkpoint(), 200.0, &[])
            .unwrap()
            .next()
            .is_none()
    );
}

#[test]
fn resumed_atomic_metrics_keep_original_indices_and_ignore_consumed_objects() {
    let document = document(
        "span{display:inline-block}",
        "<p><span>First</span> <span>Second</span></p>",
    );
    let paragraphs = paragraphs(&document);
    assert_eq!(paragraphs[0].atomics().len(), 2);
    let resolver = FileResolver::default();
    let context = TextContext::new(&document.styles().fonts, &resolver);
    let prepared = context.prepare_paragraph(paragraphs[0]);
    let initial = [
        AtomicMetrics {
            width: 30.0,
            height: 10.0,
            baseline: 8.0,
        },
        AtomicMetrics {
            width: 40.0,
            height: 10.0,
            baseline: 8.0,
        },
    ];
    let changed = [
        AtomicMetrics {
            width: 99.0,
            ..initial[0]
        },
        AtomicMetrics {
            width: 20.0,
            ..initial[1]
        },
    ];
    let mut first = prepared.lines(prepared.start(), 35.0, &initial).unwrap();
    let line = first.next().unwrap().unwrap();
    assert_eq!(line.atomics.len(), 1);
    assert_eq!(line.atomics[0].index, 0);
    let mut rest = prepared.lines(first.checkpoint(), 25.0, &changed).unwrap();
    let line = rest.next().unwrap().unwrap();
    assert_eq!(line.atomics.len(), 1);
    assert_eq!((line.atomics[0].index, line.atomics[0].width), (1, 20.0));
    assert!(rest.next().is_none());
}

#[test]
fn a_checkpoint_from_another_paragraph_is_rejected_before_layout() {
    let document = document("", "<p>Same text</p><p>Same text</p>");
    let paragraphs = paragraphs(&document);
    let resolver = FileResolver::default();
    let context = TextContext::new(&document.styles().fonts, &resolver);
    let first = context.prepare_paragraph(paragraphs[0]);
    let second = context.prepare_paragraph(paragraphs[1]);
    assert!(matches!(
        second.lines(first.start(), 100.0, &[]),
        Err(TextError::ParagraphMismatch)
    ));
}

#[test]
fn producing_one_line_does_not_shape_an_unrequested_suffix_or_repeat_a_terminal_error() {
    let words = (0..30)
        .map(|index| format!("A{index:03}"))
        .collect::<Vec<_>>()
        .join(" ");
    let document = document(
        "@font-face{font-family:Blocked;src:url(blocked.ttf)}p{font-family:Fixed,Blocked}",
        &format!("<p>{words} 漢</p>"),
    );
    let paragraphs = paragraphs(&document);
    struct DeniedFallback(std::cell::Cell<usize>);
    impl ferri_pdf_resources::ResourceResolver for DeniedFallback {
        fn resolve(
            &self,
            reference: &str,
            base: &ferri_pdf_resources::ResourceBase,
        ) -> Result<ferri_pdf_resources::Resource, ferri_pdf_resources::ResourceError> {
            if reference.ends_with("blocked.ttf") {
                self.0.set(self.0.get() + 1);
                return Err(ferri_pdf_resources::ResourceError::Denied(
                    base.resolve(reference)?,
                ));
            }
            ferri_pdf_resources::ResourceResolver::resolve(
                &FileResolver::default(),
                reference,
                base,
            )
        }
    }
    let resolver = DeniedFallback(std::cell::Cell::new(0));
    let context = TextContext::new(&document.styles().fonts, &resolver);
    let prepared = context.prepare_paragraph(paragraphs[0]);
    let mut lines = prepared.lines(prepared.start(), 100.0, &[]).unwrap();
    assert_eq!(lines.next().unwrap().unwrap().text, "A000 A001 A002");
    assert_eq!(
        resolver.0.get(),
        0,
        "the unrequested suffix must not load its fallback"
    );
    let mut denied = false;
    for line in lines.by_ref() {
        match line {
            Ok(_) => {}
            Err(TextError::Font(crate::fonts::FontError::Resource(
                ferri_pdf_resources::ResourceError::Denied(_),
            ))) => {
                denied = true;
                break;
            }
            Err(error) => panic!("unexpected continuation failure: {error}"),
        }
    }
    assert!(
        denied,
        "the final character must surface the real resource-policy failure"
    );
    assert!(
        lines.next().is_none(),
        "a failed line stream must terminate rather than yield the error forever"
    );
    assert_eq!(
        resolver.0.get(),
        1,
        "a policy failure must not be retried after iterator termination"
    );
}

#[test]
fn pathological_missing_combining_sequences_cannot_trigger_unbounded_matching() {
    const COMBINING_MARKS: usize = 65_536;
    let text = format!("\u{ef00}{}", "\u{1ab0}".repeat(COMBINING_MARKS));
    let document = document("", &format!("<p>{text}</p>"));
    let paragraphs = paragraphs(&document);
    let resolver = FileResolver::default();
    let context = TextContext::new(&document.styles().fonts, &resolver);
    let result = context.layout_paragraph(paragraphs[0], 100.0, &[]);
    assert!(
        matches!(
            result,
            Err(TextError::GlyphMatchingLimit {
                limit: crate::text::MAX_GLYPH_MATCH_WORK
            })
        ),
        "a pathological unsupported combining tail must hit the typed matching work bound"
    );
    assert!(
        matches!(
            context.layout_paragraph(paragraphs[0], 100.0, &[]),
            Err(TextError::GlyphMatchingLimit { .. })
        ),
        "the work failure is sticky across the same context"
    );
}

#[test]
fn resumed_css_bidi_overrides_preserve_visual_order_and_original_source_positions() {
    let words = (0..30)
        .map(|index| format!("A{index:02}"))
        .collect::<Vec<_>>()
        .join(" ");
    let document = document(
        "",
        &format!("<p><span style='direction:rtl;unicode-bidi:isolate-override'>{words}</span></p>"),
    );
    let paragraphs = paragraphs(&document);
    let resolver = FileResolver::default();
    let context = TextContext::new(&document.styles().fonts, &resolver);
    let prepared = context.prepare_paragraph(paragraphs[0]);
    let mut first = prepared.lines(prepared.start(), 100.0, &[]).unwrap();
    let head = first.next().unwrap().unwrap();
    let checkpoint = first.checkpoint();
    let mut lines = vec![head];
    lines.extend(
        prepared
            .lines(checkpoint, 60.0, &[])
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap(),
    );
    assert!(
        lines.len() > 3,
        "the override must continue across several differently sized fragmentainers"
    );
    for line in &lines {
        let visual: String = line
            .runs
            .iter()
            .flat_map(|run| {
                run.glyphs()
                    .iter()
                    .map(|glyph| &run.text[glyph.text_range.clone()])
            })
            .collect();
        assert_eq!(
            visual,
            line.text.chars().rev().collect::<String>(),
            "each continued line must retain its RTL override: {:?}",
            line.range
        );
    }
    assert_eq!(
        lines
            .iter()
            .flat_map(|line| line.text.split_whitespace())
            .collect::<Vec<_>>(),
        words.split_whitespace().collect::<Vec<_>>()
    );
    let source_glyphs = glyphs_from(&lines, 0);
    let offsets = source_glyphs
        .iter()
        .filter(|(offset, _)| paragraphs[0].text().as_bytes()[*offset] != b' ')
        .map(|(offset, _)| *offset)
        .collect::<Vec<_>>();
    let expected = paragraphs[0]
        .text()
        .char_indices()
        .filter(|(_, ch)| *ch != ' ')
        .map(|(offset, _)| offset)
        .collect::<Vec<_>>();
    assert_eq!(
        offsets, expected,
        "source glyph positions must have neither injected controls nor omissions across the checkpoint"
    );
}

#[test]
fn pathological_css_bidi_expansion_hits_a_sticky_typed_limit() {
    const SCOPES: usize = 128;
    const BREAKS: usize = 90_000;
    let body = format!(
        "<p style='white-space:pre'>{}{}{}</p><p>Ordinary</p>",
        "<span style='unicode-bidi:embed'>".repeat(SCOPES),
        "a\n".repeat(BREAKS),
        "</span>".repeat(SCOPES)
    );
    let document = document("", &body);
    let paragraphs = paragraphs(&document);
    let resolver = FileResolver::default();
    let context = TextContext::new(&document.styles().fonts, &resolver);
    let ready = context.prepare_paragraph(paragraphs[1]);
    let mut live = ready.lines(ready.start(), 100.0, &[]).unwrap();
    let prepared = context.prepare_paragraph(paragraphs[0]);
    assert!(
        matches!(
            prepared.lines(prepared.start(), 100.0, &[]),
            Err(TextError::BidiExpansionLimit {
                limit: crate::document::MAX_INLINE_TEXT_BYTES
            })
        ),
        "reopening deep CSS scopes must hit the typed expansion bound before running UBA on an unbounded stream"
    );
    let next = context.prepare_paragraph(paragraphs[1]);
    assert!(
        matches!(
            next.lines(next.start(), 100.0, &[]),
            Err(TextError::BidiExpansionLimit { .. })
        ),
        "exhausted bidi work must remain terminal for the shared text context"
    );
    assert!(
        matches!(
            ready.lines(ready.start(), 100.0, &[]),
            Err(TextError::BidiExpansionLimit { .. })
        ),
        "a previously prepared paragraph must not bypass the context's terminal work bound"
    );
    assert!(
        matches!(live.next(), Some(Err(TextError::BidiExpansionLimit { .. }))),
        "an already-created iterator must observe shared exhaustion before emitting another line"
    );
    assert!(
        live.next().is_none(),
        "the failed iterator must remain terminated"
    );
}

#[test]
fn leader_intrinsic_widths_keep_one_pattern_and_do_not_mutate_after_expansion() {
    let document = document("p::after{content:leader('.') 'Z'}", "<p>A</p>");
    let source = paragraphs(&document)[0];
    let resolver = FileResolver::default();
    let context = TextContext::new(&document.styles().fonts, &resolver);
    let minimum = Paragraph::generated(".Z", source.style()).unwrap().unwrap();
    let maximum = Paragraph::generated("A", source.style()).unwrap().unwrap();
    let expected_min = context.intrinsic_widths(&minimum, &[]).unwrap().max;
    let expected_max = expected_min + context.intrinsic_widths(&maximum, &[]).unwrap().max;
    let before = context.intrinsic_widths(source, &[]).unwrap();
    assert_pt(before.min, expected_min);
    assert_pt(before.max, expected_max);
    let prepared = context.prepare_paragraph(source);
    let wide = prepared
        .lines(prepared.start(), 200.0, &[])
        .unwrap()
        .next()
        .unwrap()
        .unwrap();
    assert!(wide.text.matches('.').count() > 20);
    let narrow: Vec<_> = prepared
        .lines(prepared.start(), before.min, &[])
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(
        narrow
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>(),
        ["A", ".Z"]
    );
    let after = context.intrinsic_widths(source, &[]).unwrap();
    assert_pt(after.min, before.min);
    assert_pt(after.max, before.max);
    assert_eq!(source.text(), "A.Z");
}

#[test]
fn indentation_keeps_first_and_forced_line_state_across_resumed_widths() {
    for (keywords, indents) in [
        ("", [20.0, 0.0, 0.0, 0.0]),
        ("each-line", [20.0, 0.0, 40.0, 0.0]),
        ("each-line hanging", [0.0, 30.0, 0.0, 10.0]),
    ] {
        let document = document(
            &format!("p{{text-indent:20% {keywords}}}"),
            "<p>MMMM MMMM<br>MMMM MMMM</p>",
        );
        let resolver = FileResolver::default();
        let context = TextContext::new(&document.styles().fonts, &resolver);
        let prepared = context.prepare_paragraph(paragraphs(&document)[0]);
        let mut cursor = prepared.start();
        for (basis, expected) in [100.0, 150.0, 200.0, 50.0].into_iter().zip(indents) {
            let mut stream = prepared.lines_in_block(cursor, 60.0, basis, &[]).unwrap();
            let line = stream.next().unwrap().unwrap();
            assert_eq!(line.text, "MMMM", "each checkpoint must consume one word");
            assert_pt(line.indent, expected);
            cursor = stream.checkpoint();
        }
        assert!(prepared.lines(cursor, 100.0, &[]).unwrap().next().is_none());
    }
}

#[test]
fn intrinsic_indentation_uses_zero_percentage_and_the_selected_line() {
    let document = document("", "<p>MMMM MMMM</p>");
    let resolver = FileResolver::default();
    let context = TextContext::new(&document.styles().fonts, &resolver);
    let source = paragraphs(&document)[0];
    let baseline = context.intrinsic_widths(source, &[]).unwrap();
    for (amount, hanging, each_line, min_extra, max_extra) in [
        (
            crate::style::LengthPercentage::Length(20.0),
            false,
            false,
            20.0,
            20.0,
        ),
        (
            crate::style::LengthPercentage::Percentage(0.2),
            false,
            false,
            0.0,
            0.0,
        ),
        (
            crate::style::LengthPercentage::Length(20.0),
            true,
            false,
            20.0,
            0.0,
        ),
        (
            crate::style::LengthPercentage::Length(-10.0),
            false,
            false,
            0.0,
            -10.0,
        ),
    ] {
        let mut style = source.style().clone();
        style.text_indent = crate::style::TextIndent {
            amount,
            hanging,
            each_line,
        };
        let paragraph = Paragraph::generated(source.text(), &style)
            .unwrap()
            .unwrap();
        let widths = context.intrinsic_widths(&paragraph, &[]).unwrap();
        assert_pt(widths.min, baseline.min + min_extra);
        assert_pt(widths.max, baseline.max + max_extra);
    }
}
