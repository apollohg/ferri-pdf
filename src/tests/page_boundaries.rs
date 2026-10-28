use super::common;

use common::{PROFILE, local_options};
use ferri_pdf_document::{BlockBox, BlockContent, StyledDocument};
use ferri_pdf_resources::FileResolver;
use ferri_pdf_style::{Display, Page};

fn document(css: &str, body: &str) -> StyledDocument {
    StyledDocument::parse(
        &format!("<!doctype html><style>{PROFILE}{css}</style>{body}"),
        &local_options().base,
        &[],
        &FileResolver::default(),
    )
    .unwrap()
}

fn containing<'a>(block: &'a BlockBox, text: &str) -> &'a BlockBox {
    let mut pending = vec![block];
    while let Some(block) = pending.pop() {
        for child in block.children() {
            match child {
                BlockContent::Events(_) => {}
                BlockContent::Inline(paragraph) => {
                    if paragraph.text() == text {
                        return block;
                    }
                    pending.extend(paragraph.atomics().iter().map(|anchor| &anchor.block));
                    pending.extend(paragraph.anchors().iter().map(|anchor| &anchor.block));
                }
                BlockContent::Block(child) | BlockContent::Positioned(child) => pending.push(child),
            }
        }
    }
    panic!("no generated block directly contains {text:?}");
}

fn assert_boundary(block: &BlockBox, used: Option<&str>, start: Option<&str>, end: Option<&str>) {
    let actual = block.page_boundary();
    assert_eq!(
        (
            actual.used.as_deref(),
            actual.start.as_deref(),
            actual.end.as_deref()
        ),
        (used, start, end),
        "generated {:?} box with computed page {:?}",
        block.style().display,
        block.style().page,
    );
}

#[test]
fn used_page_scopes_follow_real_ancestors_and_restore_after_boxless_elements() {
    let document = document(
        "body{page:A}.contents{display:contents;page:B}.hidden{display:none;page:C}",
        "<div class=hidden>Hidden</div><p>Before</p><div class=contents><p>Inside</p><p style='page:inherit'>Inherited</p></div><p>After</p>",
    );
    for (text, scope) in [
        ("Before", "A"),
        ("Inside", "B"),
        ("Inherited", "B"),
        ("After", "A"),
    ] {
        let block = containing(document.root(), text);
        assert_boundary(block, Some(scope), Some(scope), Some(scope));
        if text == "Inherited" {
            assert_eq!(block.style().page, Page::Name("B".into()));
        } else {
            assert_eq!(
                block.style().page,
                Page::Auto,
                "used names do not become inherited computed values"
            );
        }
    }
    assert_boundary(document.root(), None, Some("A"), Some("A"));
}

#[test]
fn inline_and_positioned_subtrees_do_not_supply_outer_page_boundaries() {
    let document = document(
        "body{page:A}span,.atomic,.positioned{page:B}.atomic{display:inline-block}.positioned{position:absolute}",
        "<div class=positioned>Positioned</div><p><span>Inline</span> <span class=atomic><span style='display:block;page:C'>Atomic</span></span></p><p>After</p>",
    );
    assert_boundary(document.root(), None, Some("A"), Some("A"));
    assert_boundary(
        containing(document.root(), "Positioned"),
        Some("B"),
        Some("B"),
        Some("B"),
    );
    assert_boundary(
        containing(document.root(), "Atomic"),
        Some("C"),
        Some("C"),
        Some("C"),
    );
    assert_boundary(
        containing(document.root(), "After"),
        Some("A"),
        Some("A"),
        Some("A"),
    );
}

#[test]
fn anonymous_table_and_flex_boxes_keep_scope_without_fictional_named_ancestors() {
    let document = document(
        ".host{page:A}.cell{display:table-cell;page:B}.flex{display:flex;page:C}",
        "<div class=host><div class=cell><p>Cell</p></div></div><div class=flex>Anonymous item</div>",
    );
    let cell = containing(document.root(), "Cell");
    assert_boundary(cell, Some("B"), Some("B"), Some("B"));
    let mut pending = vec![document.root()];
    let mut wrappers = Vec::new();
    while let Some(block) = pending.pop() {
        if matches!(block.style().display, Display::Table | Display::TableRow) {
            wrappers.push(block);
        }
        pending.extend(block.children().iter().filter_map(|child| match child {
            BlockContent::Block(block) => Some(block),
            _ => None,
        }));
    }
    assert_eq!(
        wrappers.len(),
        2,
        "the actual table normalization inserts a table and row"
    );
    for wrapper in wrappers {
        assert_eq!(wrapper.style().page, Page::Auto);
        assert_boundary(wrapper, Some("A"), Some("A"), Some("A"));
    }
    let item = containing(document.root(), "Anonymous item");
    assert_eq!(item.style().page, Page::Auto);
    assert_boundary(item, Some("C"), Some("C"), Some("C"));
}

#[test]
fn anonymous_inline_content_and_empty_names_are_real_start_and_end_values() {
    let document = document(
        ".named{page:B}.inline-first{page:A}",
        "<div class=inline-first>Leading<span> inline</span><p class=named>Middle</p>Trailing</div><div><p>Default</p><p class=named>Last</p></div>",
    );
    let inline_parent = containing(document.root(), "Leading inline");
    assert_boundary(inline_parent, Some("A"), Some("A"), Some("A"));
    let body = document
        .root()
        .children()
        .iter()
        .find_map(|child| match child {
            BlockContent::Block(block) => Some(block),
            _ => None,
        })
        .unwrap();
    assert_boundary(body, None, Some("A"), Some("B"));
    let default_parent = body
        .children()
        .iter()
        .filter_map(|child| match child {
            BlockContent::Block(block) => Some(block),
            _ => None,
        })
        .nth(1)
        .unwrap();
    assert_boundary(default_parent, None, None, Some("B"));
}

#[test]
fn generated_blocks_and_flex_order_determine_first_and_last_page_values() {
    let document = document(
        "body{page:A}.generated::before{display:block;content:'Before';page:B}.generated::after{display:block;content:'After';page:C}.flex{display:flex}.first{order:-1;page:D}.last{order:1;page:E}",
        "<div class=generated><p>Middle</p></div><div class=flex><p class=last>Last</p><p class=first>First</p></div>",
    );
    assert_boundary(
        containing(document.root(), "Before"),
        Some("B"),
        Some("B"),
        Some("B"),
    );
    assert_boundary(
        containing(document.root(), "After"),
        Some("C"),
        Some("C"),
        Some("C"),
    );
    assert_boundary(document.root(), None, Some("B"), Some("E"));
}

#[test]
fn table_page_boundaries_follow_caption_header_and_footer_layout_order() {
    let captions = document(
        "body{page:A}thead{page:B}tfoot{page:C}caption{page:D;caption-side:bottom}",
        "<table><caption>Bottom</caption><tfoot><tr><td>Footer</td></tr></tfoot><tbody><tr><td>Body</td></tr></tbody><thead><tr><td>Header</td></tr></thead></table>",
    );
    assert_boundary(captions.root(), None, Some("B"), Some("D"));
    let repeated = document(
        "body{page:A}thead{page:B}tfoot{page:C}.ordinary-header{page:D}.ordinary-footer{page:E}",
        "<table><tfoot><tr><td>First footer</td></tr></tfoot><thead><tr><td>First header</td></tr></thead><thead class=ordinary-header><tr><td>Ordinary header</td></tr></thead><tfoot class=ordinary-footer><tr><td>Ordinary footer</td></tr></tfoot></table>",
    );
    assert_boundary(repeated.root(), None, Some("B"), Some("C"));
}

#[test]
fn generated_line_break_pseudos_use_the_line_break_elements_own_page_scope() {
    let document = document(
        "body{page:A}br{page:B}br::before{display:block;content:'Before break'}",
        "<p><br></p>",
    );
    assert_boundary(
        containing(document.root(), "Before break"),
        Some("B"),
        Some("B"),
        Some("B"),
    );
}
