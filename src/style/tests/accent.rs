//! CSS UI 4 §7.1 and CSS Color 4 §6.2: `AccentColor` takes the element's `accent-color`.
use crate::dom::{DocumentTree, Selector};
use crate::resources::FileResolver;
use crate::style::{Style, Stylesheet, StylesheetSource};

/// Used `color` and `background-color` of `#parent` and of its child `#child`.
fn colors(css: &str) -> [([u8; 3], [u8; 3]); 2] {
    let sheet = Stylesheet::parse(&[StylesheetSource {
        text: css.into(),
        base: Default::default(),
    }])
    .unwrap_or_else(|error| panic!("`{css}` must parse: {error}"));
    let html = DocumentTree::parse("<div id=parent><span id=child>X</span></div>").unwrap();
    let mut parent = Style::default();
    ["#parent", "#child"].map(|id| {
        let element = html.select(&Selector::parse(id).unwrap()).next().unwrap();
        parent = sheet
            .computed_with_resolver(element, &parent, &FileResolver::default())
            .unwrap_or_else(|error| panic!("`{css}`: {id} must compute: {error}"));
        (parent.color.rgb, parent.decoration.background.rgb)
    })
}

#[test]
fn accent_color_system_colours_follow_the_accent_color_property() {
    const UA_ACCENT: [u8; 3] = [0x00, 0x60, 0xdf];
    const PURPLE: [u8; 3] = [0x66, 0x33, 0x99];
    assert_eq!(
        colors("#parent{color:AccentColor;background:AccentColorText}")[0],
        (UA_ACCENT, [0xff; 3]),
        "`auto` keeps the user agent's accent pair"
    );
    let [parent, child] = colors(
        "#parent{accent-color:rebeccapurple;color:AccentColor;background:AccentColorText}\
         #child{color:AccentColor}",
    );
    assert_eq!(
        parent,
        (PURPLE, [0xff; 3]),
        "white contrasts more on purple"
    );
    assert_eq!(
        child.0, PURPLE,
        "accent-color inherits into the child's AccentColor"
    );
    assert_eq!(
        colors("#parent{accent-color:yellow;background:AccentColorText}")[0].1,
        [0; 3],
        "black contrasts more on yellow"
    );
    assert_eq!(
        colors("#parent{accent-color:rgb(0 0 255 / 50%);color:AccentColor}")[0].0,
        [128, 128, 255],
        "a translucent accent is pre-composed over Canvas"
    );
    assert_eq!(
        colors("#parent{color:green}#child{color:lime;accent-color:currentcolor;background:AccentColor}")[1]
            .1,
        [0, 0xff, 0],
        "currentcolor in accent-color is the element's own colour"
    );
}
