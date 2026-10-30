//! CSS Overflow 5 §3.1: `::scroll-marker-group` boxes and the `::scroll-marker` boxes in them.
use crate::document::{BlockBox, BlockContent, BoxNavigation, LinkTarget, StyledDocument};
use crate::resources::{FileResolver, ResourceBase};

fn parse(html: &str) -> StyledDocument {
    StyledDocument::parse(
        html,
        &ResourceBase::default(),
        &[],
        &FileResolver::default(),
    )
    .unwrap_or_else(|error| panic!("scroll marker fixture must format: {error}\n{html}"))
}

/// The box tree below `block`: `[]` for each block, nesting its children, and the text of
/// each paragraph.
fn outline(block: &BlockBox, depth: usize, output: &mut Vec<String>) {
    for child in block.children() {
        match child {
            BlockContent::Events(_) => {}
            BlockContent::Block(child) | BlockContent::Positioned(child) => {
                output.push(format!("{}[]", "  ".repeat(depth)));
                outline(child, depth + 1, output);
            }
            BlockContent::Inline(paragraph) => {
                output.push(format!("{}{:?}", "  ".repeat(depth), paragraph.text()));
            }
        }
    }
}

fn own_text(block: &BlockBox) -> String {
    block
        .children()
        .iter()
        .filter_map(|child| match child {
            BlockContent::Inline(paragraph) => Some(paragraph.text()),
            _ => None,
        })
        .collect()
}

fn root_outline(document: &StyledDocument) -> Vec<String> {
    let mut output = Vec::new();
    outline(document.root(), 0, &mut output);
    output
}

#[test]
fn groups_are_siblings_of_their_scroll_containers_holding_the_markers_of_their_contents() {
    let document = parse(
        "<style>body{margin:0;counter-reset:n}.s{overflow:auto;scroll-marker-group:after}\
         .t{overflow:hidden;scroll-marker-group:before}\
         .s::scroll-marker-group,.t::scroll-marker-group{display:block}\
         .m{counter-increment:n}.m::scroll-marker{content:'M' counter(n);display:block}\
         .plain{overflow:auto}</style>\
         <p>Before</p><div class=s><p class=m>One</p><div class=plain><p class=m>Hidden</p></div>\
         <p class=m>Two</p></div><div class=t><p class=m>Three</p><p>No marker</p></div><p>After</p>",
    );
    assert_eq!(
        root_outline(&document),
        [
            "[]",
            "  []",
            "    \"Before\"",
            "  []",
            "    []",
            "      \"One\"",
            "    []",
            "      []",
            "        \"Hidden\"",
            "    []",
            "      \"Two\"",
            "  []",
            "    []",
            "      \"M1\"",
            "    []",
            "      \"M3\"",
            "  []",
            "    []",
            "      \"M4\"",
            "  []",
            "    []",
            "      \"Three\"",
            "    []",
            "      \"No marker\"",
            "  []",
            "    \"After\"",
        ],
        "an `after` group follows its scroll container and a `before` group precedes it; each \
         holds the markers whose nearest ancestor scroll container it belongs to, in tree \
         order, with counters as they stand at each originating element, so the nested \
         container's own markerless group drops M2"
    );
}

#[test]
fn the_root_element_places_its_group_first_or_last_and_inline_groups_join_the_inline_content() {
    for (side, expected) in [
        (
            "before",
            vec![
                "\"#a#b\"",
                "[]",
                "  []",
                "    \"Section a\"",
                "  []",
                "    \"Section b\"",
            ],
        ),
        (
            "after",
            vec![
                "[]",
                "  []",
                "    \"Section a\"",
                "  []",
                "    \"Section b\"",
                "\"#a#b\"",
            ],
        ),
    ] {
        let document = parse(&format!(
            "<style>html{{scroll-marker-group:{side}}}body{{margin:0}}\
             section::scroll-marker{{content:'#' attr(id)}}</style>\
             <section id=a>Section a</section><section id=b>Section b</section>"
        ));
        assert_eq!(
            root_outline(&document),
            expected,
            "`{side}`: the root element's inline group is its {} child, an anonymous block of \
             inline markers",
            if side == "before" { "first" } else { "last" }
        );
    }
}

#[test]
fn a_marker_takes_its_originating_elements_counters_and_keeps_its_own_changes() {
    let document = parse(
        "<style>ol{overflow:auto;scroll-marker-group:after;counter-reset:c;list-style:none}\
         li{counter-increment:c}li::after{content:' after ' counter(c)}\
         li::scroll-marker{content:'(' counter(c) ')';counter-increment:c 10}</style>\
         <ol><li>One</li><li>Two</li></ol>",
    );
    let outline = root_outline(&document);
    assert!(
        outline.iter().any(|line| line.contains("One after 1"))
            && outline.iter().any(|line| line.contains("Two after 2")),
        "a marker's counter-increment stays within the marker: {outline:#?}"
    );
    assert!(
        outline.iter().any(|line| line.contains("(11)(12)")),
        "each marker sees its originating element's counter, then its own increment: {outline:#?}"
    );
}

/// The navigation of every block and inline box of `block` with any, labelled by its text.
fn navigation(block: &BlockBox, output: &mut Vec<(String, BoxNavigation)>) {
    if !block.navigation().is_empty() {
        output.push((own_text(block), block.navigation()));
    }
    for child in block.children() {
        match child {
            BlockContent::Events(_) => {}
            BlockContent::Block(child) | BlockContent::Positioned(child) => {
                navigation(child, output);
            }
            BlockContent::Inline(paragraph) => {
                for inline_box in paragraph.boxes() {
                    if !inline_box.navigation.is_empty() {
                        output.push((
                            paragraph.text()[inline_box.range.clone()].to_owned(),
                            inline_box.navigation,
                        ));
                    }
                }
                for atomic in paragraph.atomics() {
                    navigation(atomic.block(), output);
                }
            }
        }
    }
}

#[test]
fn markers_link_to_their_originating_elements() {
    let document = parse(
        "<style>.s{overflow:auto;scroll-marker-group:before}\
         .s p::scroll-marker{content:'go ' attr(class)}</style>\
         <a href='#x'>Link first</a><div class=s><p class=one>One</p><p class=two>Two</p></div>",
    );
    let mut found = Vec::new();
    navigation(document.root(), &mut found);
    let links = &document.navigation().links;
    let destination_of = |text: &str| {
        found
            .iter()
            .find(|(label, navigation)| label == text && navigation.destination.is_some())
            .and_then(|(_, navigation)| navigation.destination)
            .unwrap_or_else(|| panic!("{text:?} has no destination: {found:#?}"))
    };
    for (marker, target) in [("go one", "One"), ("go two", "Two")] {
        let link = found
            .iter()
            .find(|(label, _)| label == marker)
            .and_then(|(_, navigation)| navigation.link)
            .unwrap_or_else(|| panic!("{marker:?} carries no hyperlink: {found:#?}"));
        assert_eq!(
            links[link],
            LinkTarget::Destination(destination_of(target)),
            "{marker:?} links to {target:?}, the element it scrolls into view; links: {links:?}"
        );
    }
}

#[test]
fn formatting_a_first_line_copy_numbers_marker_links_and_destinations_as_the_document_does() {
    let document = parse(
        "<style>html{scroll-marker-group:after}p::first-line{color:red}\
         .ib{display:inline-block}.s{overflow:auto;scroll-marker-group:after}\
         .s span::scroll-marker{content:'*'}i::scroll-marker{content:'root'}</style>\
         <p><span class=ib><i>outer group</i>\
         <span class=s style=display:block><span>a</span><span>b</span></span>\
         <h2 id=later>Later</h2><a href='#later'>back</a></span></p>",
    );
    let mut paragraphs = Vec::new();
    super::paragraphs(document.root(), &mut paragraphs);
    let atomic = paragraphs
        .iter()
        .flat_map(|paragraph| paragraph.atomics())
        .next()
        .expect("the inline-block is an atomic inline");
    let copy = atomic
        .block()
        .first_line_box()
        .expect("the first line restyles the inline-block");
    let (mut source, mut copied) = (Vec::new(), Vec::new());
    navigation(atomic.block(), &mut source);
    navigation(copy, &mut copied);
    assert!(
        source.iter().any(|(label, _)| label == "*"),
        "the markers carry links: {source:#?}"
    );
    assert_eq!(
        copied, source,
        "the first-line copy allocates the marker links, also those of the root's group outside \
         it, and the heading destination after them as the document's formatting does"
    );
}

#[test]
fn nested_groups_placed_before_their_containers_number_and_link_their_markers_in_place() {
    let document = parse(
        "<style>body{margin:0;counter-reset:n}.s{overflow:auto;scroll-marker-group:before}\
         .s::scroll-marker-group{display:block}.m{counter-increment:n}\
         .m::scroll-marker{content:'M' counter(n);display:block}</style>\
         <a href='#a'>Link</a><div class=s><p class=m id=a>A</p><div class=s>\
         <p class=m>B</p><div class=s><p class=m>C</p></div><p class=m>D</p></div>\
         <p class=m>E</p></div>",
    );
    assert_eq!(
        root_outline(&document),
        [
            "[]",
            "  \"Link\"",
            "  []",
            "    []",
            "      \"M1\"",
            "    []",
            "      \"M5\"",
            "  []",
            "    []",
            "      \"A\"",
            "    []",
            "      []",
            "        \"M2\"",
            "      []",
            "        \"M4\"",
            "    []",
            "      []",
            "        \"B\"",
            "      []",
            "        []",
            "          \"M3\"",
            "      []",
            "        []",
            "          \"C\"",
            "      []",
            "        \"D\"",
            "    []",
            "      \"E\"",
        ],
        "each group placed before its container holds the markers of the elements whose \
         nearest scroll container it is, numbered by the counters at each originating element"
    );
    let mut found = Vec::new();
    navigation(document.root(), &mut found);
    let links = &document.navigation().links;
    for (marker, target) in [
        ("M1", "A"),
        ("M2", "B"),
        ("M3", "C"),
        ("M4", "D"),
        ("M5", "E"),
    ] {
        let destination = found
            .iter()
            .find(|(label, navigation)| label == target && navigation.destination.is_some())
            .and_then(|(_, navigation)| navigation.destination)
            .unwrap_or_else(|| panic!("{target:?} has no destination: {found:#?}"));
        let link = found
            .iter()
            .find(|(label, _)| label == marker)
            .and_then(|(_, navigation)| navigation.link)
            .unwrap_or_else(|| panic!("{marker:?} carries no hyperlink: {found:#?}"));
        assert_eq!(
            links[link],
            LinkTarget::Destination(destination),
            "{marker:?} links to {target:?}; links: {links:?}"
        );
    }
}
