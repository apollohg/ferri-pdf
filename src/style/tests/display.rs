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
    let html = DocumentTree::parse("<div id=run><div id=child><div id=leaf>X</div></div></div>")
        .expect("parse test HTML");
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
