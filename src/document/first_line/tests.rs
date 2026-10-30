use super::*;
use crate::document::{BlockBox, BlockContent, StyledDocument};
use crate::resources::{FileResolver, ResourceBase};
use crate::style::Display;

const CANONICAL_SIZE: f32 = 20.0;
const FIRST_LINE_SIZE: f32 = 40.0;
const FIRST_LETTER_SIZE: f32 = 30.0;
const FIRST_LETTER_COLOR: [u8; 3] = [0, 128, 0];
const CANONICAL_COLOR: [u8; 3] = [0, 0, 255];

fn parse(html: &str) -> StyledDocument {
    StyledDocument::parse(
        html,
        &ResourceBase::default(),
        &[],
        &FileResolver::default(),
    )
    .expect("Math typographic boundary fixture must format")
}

fn paragraphs(block: &BlockBox) -> Vec<&Paragraph> {
    let mut result = Vec::new();
    let mut blocks = vec![block];
    while let Some(block) = blocks.pop() {
        for child in block.children() {
            match child {
                BlockContent::Block(child) | BlockContent::Positioned(child) => blocks.push(child),
                BlockContent::Inline(paragraph) => result.push(paragraph),
                BlockContent::Events(_) => {}
            }
        }
    }
    result
}

#[test]
fn host_first_line_keeps_math_atomic_styles_canonical() {
    let document = parse(
        "<style>body{font-size:20pt;color:blue}p::first-line{font-size:40pt;color:red}</style><p>Lead <math><mfrac linethickness='1ex'><mn>1</mn><mn>2</mn></mfrac></math></p>",
    );
    let paragraphs = paragraphs(document.root());
    let paragraph = paragraphs
        .iter()
        .find(|paragraph| !paragraph.atomics().is_empty())
        .expect("host paragraph must contain the real Math atomic");
    let canonical = paragraph.atomics()[0].block.style();
    assert_eq!(canonical.display, Display::InlineMath);
    assert_eq!(canonical.size, CANONICAL_SIZE);
    assert_eq!(canonical.color.rgb, CANONICAL_COLOR);
    let line = paragraph
        .first_line()
        .expect("host first-line variant must remain");
    assert_eq!(
        line.style().size,
        FIRST_LINE_SIZE,
        "host line strut still inherits first-line size"
    );
    let selected = line.atomics()[0].block.style();
    assert_eq!(
        selected.size, canonical.size,
        "Math content and font-relative native attributes must not be remeasured from a host line fragment's font"
    );
    assert_eq!(
        selected.color, canonical.color,
        "host first-line color stops at the Math formatting context"
    );
}

#[test]
fn math_boxes_do_not_generate_their_own_first_line_styles() {
    for display in ["inline", "block"] {
        let document = parse(&format!(
            "<style>math{{font-size:20pt}}math::first-line{{font-size:40pt;color:red}}</style><p><math display='{display}'><mtext>Math</mtext></math></p>"
        ));
        let mut blocks = vec![document.root()];
        let mut math = None;
        while let Some(block) = blocks.pop() {
            if matches!(
                block.style().display,
                Display::InlineMath | Display::BlockMath
            ) {
                math = Some(block.style());
                break;
            }
            for child in block.children() {
                match child {
                    BlockContent::Block(child) | BlockContent::Positioned(child) => {
                        blocks.push(child);
                    }
                    BlockContent::Inline(paragraph) => {
                        blocks.extend(paragraph.atomics().iter().map(|atomic| &atomic.block));
                    }
                    BlockContent::Events(_) => {}
                }
            }
        }
        let style = math.expect("actual computed Math box");
        let origin = style.origin.node().expect("Math source origin").element;
        let resolver = FileResolver::default();
        let base = ResourceBase::default();
        let mut cache = crate::images::ImageCache::default();
        let mut resources = ImageResources {
            base: &base,
            resolver: &resolver,
            cache: &mut cache,
            elements: document.element_images(),
        };
        assert!(
            FirstLineStyles::of(
                style,
                &[origin],
                document.styles(),
                &document.source.html,
                &mut resources
            )
            .unwrap()
            .is_none(),
            "computed {display} math must not create a first-line pseudo style"
        );
    }
}

#[test]
fn non_math_display_math_keeps_ordinary_typographic_pseudos() {
    let document = parse(
        "<style>span{display:block math;font-size:20pt;color:blue}span::first-line{font-size:40pt;color:red}span::first-letter{font-size:30pt;color:green}</style><span>Flow</span>",
    );
    let paragraphs = paragraphs(document.root());
    let paragraph = paragraphs
        .iter()
        .find(|paragraph| paragraph.text() == "Flow")
        .expect("ordinary CSS flow content must remain real text");
    let line = paragraph
        .first_line()
        .expect("CSS flow override keeps first-line");
    assert_eq!(line.style().size, FIRST_LINE_SIZE);
    let first = paragraph
        .spans()
        .iter()
        .find(|span| span.range.start == 0 && !span.range.is_empty())
        .expect("first-letter source span");
    assert_eq!(
        first.style.size, FIRST_LETTER_SIZE,
        "CSS flow override keeps first-letter sizing"
    );
    assert_eq!(
        first.style.color.rgb, FIRST_LETTER_COLOR,
        "CSS flow override keeps first-letter color"
    );
}
