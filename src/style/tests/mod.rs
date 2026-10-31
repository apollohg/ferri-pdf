mod accent;
mod calc;
mod colors;
mod counter_styles;
mod display;
mod font_family;
mod inert;
mod line_breaking;
mod logical;
mod named_page_rules;
mod page_selection;
mod page_size;
mod registered_properties;
mod relative_units;
mod source_set;
mod unsupported_messages;
mod variables;
mod visual_effects;
mod wide_keywords;
mod zoom;

use crate::dom::{DocumentTree, Selector};
use crate::resources::FileResolver;
use crate::style::{Degradation, Style, Stylesheet, StylesheetSource};

/// The selector of the element `discarded` styles.
const TARGET: &str = "#target";

/// The degradations computing `<p id=target>` with `css` reports; computing must succeed.
fn discarded(css: &str) -> Vec<Degradation> {
    let sheet = Stylesheet::parse(&[StylesheetSource {
        text: css.into(),
        base: Default::default(),
    }])
    .unwrap_or_else(|error| panic!("`{css}` must parse: {error}"));
    let html = DocumentTree::parse("<p id=target>Text</p>");
    let element = html
        .select(&Selector::parse(TARGET).expect("static selector"))
        .next()
        .expect("the target element");
    if let Err(error) =
        sheet.computed_with_resolver(element, &Style::default(), &FileResolver::default())
    {
        panic!("`{css}` must compute, discarding what the engine cannot: {error}");
    }
    sheet.take_degradations()
}
