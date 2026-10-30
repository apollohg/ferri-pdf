use crate::document::{
    BlockBox, BlockContent, Bookmark, BoxNavigation, LinkTarget, StyledDocument,
};
use crate::resources::{FileResolver, ResourceBase, Url};

fn parse(html: &str, base: &ResourceBase) -> StyledDocument {
    StyledDocument::parse(html, base, &[], &FileResolver::default())
        .expect("navigation fixture must parse")
}

fn url_base() -> ResourceBase {
    Url::parse("https://example.org/docs/index.html")
        .expect("static URL")
        .into()
}

/// Navigation of every block and inline box with any, labelled by its first text.
fn boxes(block: &BlockBox, output: &mut Vec<(String, BoxNavigation)>) {
    if !block.navigation().is_empty() {
        output.push((format!("block {:?}", first_text(block)), block.navigation()));
    }
    for child in block.children() {
        match child {
            BlockContent::Events(_) => {}
            BlockContent::Block(child) | BlockContent::Positioned(child) => boxes(child, output),
            BlockContent::Inline(paragraph) => {
                for inline_box in paragraph.boxes() {
                    if !inline_box.navigation.is_empty() {
                        output.push((
                            format!("inline {:?}", &paragraph.text()[inline_box.range.clone()]),
                            inline_box.navigation,
                        ));
                    }
                }
                for atomic in paragraph.atomics() {
                    boxes(atomic.block(), output);
                }
            }
        }
    }
}

fn first_text(block: &BlockBox) -> String {
    block
        .children()
        .iter()
        .find_map(|child| match child {
            BlockContent::Events(_) => None,
            BlockContent::Inline(paragraph) => Some(paragraph.text().to_owned()),
            BlockContent::Block(child) | BlockContent::Positioned(child) => {
                Some(first_text(child)).filter(|text| !text.is_empty())
            }
        })
        .unwrap_or_default()
}

fn link(link: usize) -> BoxNavigation {
    BoxNavigation {
        link: Some(link),
        destination: None,
    }
}

#[test]
fn hyperlinks_resolve_against_the_base_and_same_document_references_find_their_targets() {
    let document = parse(
        "<a href='https://other.example/x'>abs</a> <a href='rel/page.html'>rel</a> \
         <a href='  mailto:a@b.c '>mail</a> <a href='#t%32'>pct</a> <a href='index.html#named'>same</a> \
         <a href='#'>empty</a> <a href=''>self</a> <a href='#TOP'>top</a> <a href='#missing'>missing</a> \
         <a href='http://[bad'>bad</a> <a href='other.html#t2'>other</a>\
         <p id=t2>by id</p><a name=named>by name</a><a name=t2>shadowed name</a>",
        &url_base(),
    );
    let links = &document.navigation().links;
    assert_eq!(
        links,
        &vec![
            LinkTarget::Uri("https://other.example/x".into()),
            LinkTarget::Uri("https://example.org/docs/rel/page.html".into()),
            LinkTarget::Uri("mailto:a@b.c".into()),
            LinkTarget::Destination(0),
            LinkTarget::Destination(1),
            LinkTarget::Destination(2),
            LinkTarget::Destination(2),
            LinkTarget::Destination(2),
            LinkTarget::Uri("https://example.org/docs/other.html#t2".into()),
        ],
        "missing targets and unparsable references make inert links; ids win over names"
    );
    assert_eq!(document.navigation().destinations, 3);
    let mut found = Vec::new();
    boxes(document.root(), &mut found);
    assert_eq!(
        document.root().navigation(),
        BoxNavigation {
            link: None,
            destination: Some(2)
        },
        "the root element is the top of the document"
    );
    assert!(
        found.contains(&(
            "block \"by id\"".into(),
            BoxNavigation {
                link: None,
                destination: Some(0)
            }
        )),
        "{found:#?}"
    );
    assert!(
        found.contains(&(
            "inline \"by name\"".into(),
            BoxNavigation {
                link: None,
                destination: Some(1)
            }
        )),
        "{found:#?}"
    );
    assert!(
        !found
            .iter()
            .any(|(label, _)| label == "inline \"bad\"" || label == "inline \"missing\""),
        "inert links give their boxes no navigation: {found:#?}"
    );
}

#[test]
fn every_box_inside_a_hyperlink_carries_it_and_the_innermost_link_wins() {
    let document = parse(
        "<style>.ib{display:inline-block}a::after{content:'!'}</style>\
         <div><a href='https://outer.example/'>out <span>inner</span> <span class=ib>atomic</span> \
         <img src='data:image/svg+xml,%3Csvg xmlns=%22http://www.w3.org/2000/svg%22 width=%221%22 height=%221%22/%3E'>\
         <div>block inside</div> after</a></div>\
         <a href='https://block.example/' style='display:block'><p>para</p></a>",
        &url_base(),
    );
    let mut found = Vec::new();
    boxes(document.root(), &mut found);
    let outer = link(0);
    let labels: Vec<_> = found
        .iter()
        .map(|(label, navigation)| (label.as_str(), *navigation))
        .collect();
    assert_eq!(
        labels,
        vec![
            ("inline \"out inner \u{fffc} \u{fffc}\"", outer),
            ("inline \"inner\"", outer),
            ("block \"atomic\"", outer),
            ("block \"\"", outer),
            ("block \"block inside\"", outer),
            ("inline \"after!\"", outer),
            ("inline \"!\"", outer),
            ("block \"para\"", link(1)),
            ("block \"para\"", link(1)),
            ("inline \"!\"", link(1)),
        ],
        "the split inline box continues after the block; generated content and descendants share the link"
    );
}

#[test]
fn shadow_links_resolve_document_fragments_without_exposing_shadow_targets() {
    let document = parse(
        "<div><template shadowrootmode=closed>\
         <p id=target>Shadow duplicate</p><a name=public-name>Shadow name</a>\
         <a href='https://shadow.example/path'>Shadow</a>\
         <a href='#target'>Back</a><a href='#public-name'>Named</a>\
         <a href='#private'>Private</a><a href='#private-name'>Private name</a>\
         <p id=private>Private target</p><a name=private-name>Private name target</a>\
         <div><template shadowrootmode=open>\
         <a href='https://nested.example/'>Nested</a><slot></slot>\
         </template><a href='https://projected.example/'>Projected</a></div><slot></slot>\
         </template><a href='https://light.example/'>Light</a>\
         <template><a href='https://inert.example/'>Inert</a></template></div>\
         <p id=target>Document target</p><a name=public-name>Document name</a>\
         <a href='#target'>Document link</a><a href='#private'>Private from light</a>",
        &url_base(),
    );
    assert_eq!(
        document.navigation().links,
        [
            LinkTarget::Uri("https://shadow.example/path".into()),
            LinkTarget::Destination(0),
            LinkTarget::Destination(1),
            LinkTarget::Uri("https://nested.example/".into()),
            LinkTarget::Uri("https://projected.example/".into()),
            LinkTarget::Uri("https://light.example/".into()),
            LinkTarget::Destination(0),
        ],
        "connected shadow roots supply links once, in shadow-including order; ordinary templates and inaccessible fragments remain inert"
    );
    assert_eq!(document.navigation().destinations, 2);
    let mut found = Vec::new();
    boxes(document.root(), &mut found);
    for (label, destination) in [("Document target", 0), ("Document name", 1)] {
        assert!(
            found.iter().any(|(text, navigation)| {
                text.contains(label) && navigation.destination == Some(destination)
            }),
            "document target {label:?} must locate destination {destination}: {found:#?}"
        );
    }
    assert!(
        found.iter().all(|(text, navigation)| {
            !text.contains("Shadow duplicate") && !text.contains("Shadow name")
                || navigation.destination.is_none()
        }),
        "shadow IDs and legacy names cannot capture document fragments: {found:#?}"
    );
}

#[test]
fn bookmarks_follow_document_order_with_labels_from_their_elements() {
    let document = parse(
        "<style>h1::before,h4::before{content:'Ch. '}h1::after{content:' end'}\
         .before{bookmark-label:content(before)}.after{bookmark-label:content(after)}\
         .label{bookmark-level:2;bookmark-label:'Item ' attr(data-x) ': ' content(text)}\
         .closed{bookmark-state:closed}.none{bookmark-level:none}\
         li{bookmark-level:4;bookmark-label:content(marker) ' ' content(first-letter)}\
         .inline{bookmark-level:5}</style>\
         <h1 class=closed id=top>First   <em>heading</em>\n text</h1>\
         <h2 class=none>none</h2><h3 style=display:none>hidden</h3>\
         <div class=label data-x=7><h4 class=before>Inner</h4></div>\
         <ol start=3><li>«Quoted item</li></ol>\
         <p>text <span class=inline>inline bookmark</span></p><h1 class=after>After</h1>",
        &url_base(),
    );
    let bookmark = |level, label: &str, open, destination| Bookmark {
        level,
        label: label.into(),
        open,
        destination,
    };
    assert_eq!(
        document.navigation().bookmarks,
        vec![
            bookmark(1, "First heading text", false, 0),
            bookmark(2, "Item 7: Inner", true, 2),
            bookmark(4, "Ch.", true, 3),
            bookmark(4, "3. «Q", true, 4),
            bookmark(5, "inline bookmark", true, 5),
            bookmark(1, "end", true, 6),
        ]
    );
    assert_eq!(
        document
            .navigation()
            .headings
            .iter()
            .map(|heading| (heading.level, heading.text.as_str(), heading.destination))
            .collect::<Vec<_>>(),
        [
            (1, "First heading text", 0),
            (2, "none", 1),
            (4, "Inner", 3),
            (1, "After", 6)
        ],
        "semantic heading records include suppressed bookmarks and retain heading text independently of authored bookmark labels"
    );
}

#[test]
fn bookmark_text_uses_shadow_content_and_slot_order_instead_of_light_children() {
    let document = parse(
        "<style>.book{bookmark-level:2}</style>\
         <div class=book><template shadowrootmode=closed>Shadow <slot name=second>unused</slot> <slot name=first>unused</slot> <slot>Fallback</slot></template><span slot=first>First</span><span slot=second>Second</span><b slot=missing>Unassigned</b></div>\
         <div class=book><template shadowrootmode=open><section><template shadowrootmode=closed>Inner <slot name=forward></slot></template><slot slot=forward name=projected>Unused</slot></section></template><span slot=projected>Projected</span><span slot=missing>Excluded</span></div>",
        &url_base(),
    );
    let labels: Vec<_> = document
        .navigation()
        .bookmarks
        .iter()
        .map(|bookmark| bookmark.label.as_str())
        .collect();
    assert_eq!(
        labels,
        ["Shadow Second First Fallback", "Inner Projected"],
        "labels must use composed text once, in slot order, without unused fallback or unassigned children"
    );
}

#[test]
fn bookmark_first_letter_keeps_composed_graphemes_separate_from_after_content() {
    let document = parse(
        "<style>.book{bookmark-level:2;bookmark-label:content(first-letter) '|' content(text) '|' content(after)}.book::after{content:'After'}</style>\
         <div class=book><template shadowrootmode=closed>«e&#x301;lan <slot name=term>Fallback</slot></template><span slot=term>Assigned</span><b slot=missing>Wrong</b></div>",
        &url_base(),
    );
    assert_eq!(
        document.navigation().bookmarks[0].label,
        "«e\u{301}|«e\u{301}lan Assigned|After",
        "first-letter must use the composed grapheme while pseudo content stays separate"
    );
}

#[test]
fn multipart_capture_keeps_unreferenced_targets_and_html_fragment_precedence() {
    use crate::document::ParseOptions;
    use crate::style::PrintSettings;

    let html = "<a href='#other-part'>Across parts</a><a href='#duplicate'>Local</a>\
        <p id=duplicate>First duplicate</p><p id=duplicate>Second duplicate</p>\
        <p id='a%20b'>Literal</p><p id='a b'>Decoded</p><p id='café'>Unicode</p>\
        <a name=legacy>Legacy</a><a name=duplicate>Shadowed name</a>\
        <p id=hidden style='display:none'>Hidden</p>";
    let base = url_base();
    let settings = PrintSettings::default();
    let options = ParseOptions {
        base: &base,
        additional: &[],
        user: &[],
        settings: &settings,
        capture_fragment_targets: true,
        presentation: crate::style::Presentation::Paged,
        ancestors: &[],
    };
    let document = StyledDocument::parse_with_options(html, &options, &FileResolver::default())
        .expect("multipart capture must retain fragment intents");
    let navigation = document.navigation();
    assert_eq!(
        navigation.links[0],
        LinkTarget::Fragment("other-part".into())
    );
    let targets = navigation.fragment_targets.as_ref().unwrap();
    assert_eq!(
        targets.document_url.as_ref().unwrap().as_str(),
        "https://example.org/docs/index.html"
    );
    assert_eq!(
        targets.resolve(Some("a%20b")),
        targets.ids.get("a%20b").copied()
    );
    assert_eq!(
        targets.resolve(Some("caf%C3%A9")),
        targets.ids.get("café").copied()
    );
    assert_eq!(
        targets.resolve(Some("legacy")),
        targets.names.get("legacy").copied()
    );
    assert_eq!(
        targets.resolve(Some("duplicate")),
        targets.ids.get("duplicate").copied()
    );
    assert_ne!(targets.ids["duplicate"], targets.names["duplicate"]);
    assert_eq!(targets.resolve(None), Some(targets.root));
    assert_eq!(targets.resolve(Some("TOP")), Some(targets.root));
    assert_eq!(targets.resolve(Some("other-part")), None);
    let mut located = Vec::new();
    boxes(document.root(), &mut located);
    let duplicate_boxes: Vec<_> = located
        .iter()
        .filter(|(_, nav)| nav.destination == Some(targets.ids["duplicate"]))
        .collect();
    assert_eq!(
        duplicate_boxes.len(),
        1,
        "only the first duplicate owns the destination: {located:?}"
    );
    assert!(duplicate_boxes[0].0.contains("First duplicate"));
    assert!(
        !located
            .iter()
            .any(|(_, nav)| nav.destination == Some(targets.ids["hidden"])),
        "hidden target identity must not manufacture a rendered destination"
    );

    let ordinary = parse(html, &base);
    assert!(ordinary.navigation().fragment_targets.is_none());
    assert_eq!(
        ordinary.navigation().links.len(),
        1,
        "ordinary rendering keeps unresolved fragments inert"
    );
}

#[test]
fn declared_base_changes_reference_resolution_but_not_document_identity() {
    let document = parse(
        "<base href='body.html'><p id=duplicate>Local</p>\
         <a href='#duplicate'>Other document</a><a href='index.html#duplicate'>Own document</a>",
        &url_base(),
    );
    assert_eq!(
        document.navigation().links,
        [
            LinkTarget::Uri("https://example.org/docs/body.html#duplicate".into()),
            LinkTarget::Destination(0),
        ]
    );
}
