//! CSS Display 3 §2.7 and §5: run-in boxes inlinify their contents.
use crate::dom::{DocumentTree, Selector};
use crate::resources::FileResolver;
use crate::style::{Display, Style, Stylesheet, StylesheetSource};

/// Computed displays of `#run`, `#child` and `#leaf`, each the previous one's child.
fn displays(css: &str) -> [Display; 3] {
    let sheet = Stylesheet::parse(&[StylesheetSource {
        text: css.into(),
        base: Default::default(),
    }])
    .unwrap_or_else(|error| panic!("`{css}` must parse: {error}"));
    let html = DocumentTree::parse("<div id=run><div id=child><div id=leaf>X</div></div></div>");
    let mut parent = Style::default();
    ["#run", "#child", "#leaf"].map(|id| {
        let element = html.select(&Selector::parse(id).unwrap()).next().unwrap();
        parent = sheet
            .computed_with_resolver(element, &parent, &FileResolver::default())
            .unwrap_or_else(|error| panic!("`{css}`: {id} must compute: {error}"));
        parent.display.clone()
    })
}

#[test]
fn run_in_flow_boxes_inlinify_their_in_flow_contents() {
    assert_eq!(
        displays("#run{display:run-in}"),
        [
            Display::RunIn(Box::new(Display::Inline)),
            Display::InlineBlock,
            Display::Block
        ],
        "a block child becomes an inline block, which inlinifies nothing"
    );
    assert_eq!(
        displays("#run{display:run-in}#child{display:inline}#leaf{display:flex}"),
        [
            Display::RunIn(Box::new(Display::Inline)),
            Display::Inline,
            Display::InlineFlex
        ],
        "an inlinified inline box inlinifies its own children"
    );
    assert_eq!(
        displays("#run{display:run-in list-item}#child{display:list-item}#leaf{display:run-in}"),
        [
            Display::RunIn(Box::new(Display::InlineListItem)),
            Display::InlineFlowRootListItem,
            Display::RunIn(Box::new(Display::Inline))
        ],
        "a list item keeps its marker as an inline block"
    );
    assert_eq!(
        displays("#run{display:run-in}#child{display:inline}#leaf{display:run-in}"),
        [
            Display::RunIn(Box::new(Display::Inline)),
            Display::Inline,
            Display::Inline
        ],
        "an inlinified run-in becomes an inline box"
    );
    assert_eq!(
        displays("#run{display:run-in}#child{position:absolute}"),
        [
            Display::RunIn(Box::new(Display::Inline)),
            Display::Block,
            Display::Block
        ],
        "out-of-flow children are blockified, not inlinified"
    );
    assert_eq!(
        displays("#run{display:run-in flow-root}"),
        [
            Display::RunIn(Box::new(Display::InlineBlock)),
            Display::Block,
            Display::Block
        ],
        "a run-in flow-root box is a block container whose contents stay block-level"
    );
    assert_eq!(
        displays("#run{display:run-in;float:left}")[0],
        Display::Block,
        "a floated run-in blockifies to a block box"
    );
}

#[test]
fn math_inner_display_uses_namespace_and_table_tag_before_box_fixup() {
    for (specified, html_display, table_display) in [
        ("inline math", Display::Inline, Display::InlineTable),
        ("block math", Display::Block, Display::Table),
    ] {
        let sheet = Stylesheet::parse(&[StylesheetSource {
            text: format!("#target{{display:{specified}}}"),
            base: Default::default(),
        }])
        .unwrap();
        for (markup, expected) in [
            ("<div id=target>X</div>", html_display.clone()),
            ("<mtable id=target>X</mtable>", html_display.clone()),
            ("<svg><g id='target'/></svg>", html_display),
            ("<math><mtable id=target></mtable></math>", table_display),
            (
                "<math><mtable><mtr id=target></mtr></mtable></math>",
                Display::TableRow,
            ),
            (
                "<math><mtable><mtr><mtd id=target></mtd></mtr></mtable></math>",
                Display::TableCell,
            ),
            (
                "<math><mrow id=target></mrow></math>",
                if specified == "inline math" {
                    Display::InlineMath
                } else {
                    Display::BlockMath
                },
            ),
        ] {
            let document = DocumentTree::parse(markup);
            let target = document
                .select(&Selector::parse("#target").unwrap())
                .next()
                .unwrap_or_else(|| {
                    panic!("computed-display fixture must retain #target: {markup}")
                });
            let style = sheet
                .computed_with_resolver(target, &Style::default(), &FileResolver::default())
                .unwrap();
            assert_eq!(
                style.display, expected,
                "Core4.1 computed display for {specified} on {markup}"
            );
        }
    }
    let sheet = Stylesheet::parse(&[]).unwrap();
    let document = DocumentTree::parse("<math><mtable id=target></mtable></math>");
    let target = document
        .select(&Selector::parse("#target").unwrap())
        .next()
        .unwrap();
    assert_eq!(
        sheet
            .computed_with_resolver(target, &Style::default(), &FileResolver::default())
            .unwrap()
            .display,
        Display::InlineTable,
        "Core UA default keeps mtable inline unless the author changes it"
    );
}
