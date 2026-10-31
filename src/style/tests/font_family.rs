use crate::dom::{DocumentTree, Selector};
use crate::fonts::{FontFamily, GenericFamily};
use crate::resources::FileResolver;
use crate::style::{Style, Stylesheet, StylesheetSource};

fn families(declaration: &str) -> Vec<FontFamily> {
    let sheet = Stylesheet::parse(&[StylesheetSource {
        text: format!("p{{{declaration}}}"),
        base: Default::default(),
    }])
    .unwrap();
    let html = DocumentTree::parse("<p>Text</p>");
    let element = html.select(&Selector::parse("p").unwrap()).next().unwrap();
    sheet
        .computed_with_resolver(element, &Style::default(), &FileResolver::default())
        .unwrap()
        .families
}

#[test]
fn generic_keywords_are_generic_only_where_a_list_entry_starts() {
    let named = |name: &str| FontFamily::Named(name.into());
    let initial = vec![FontFamily::Generic(GenericFamily::Serif)];
    for (declaration, expected) in [
        (
            "font-family: DejaVu Serif, sans-serif",
            vec![
                named("DejaVu Serif"),
                FontFamily::Generic(GenericFamily::SansSerif),
            ],
        ),
        (
            "font-family: 'Missing font', Open Sans Light, monospace",
            vec![
                named("Missing font"),
                named("Open Sans Light"),
                FontFamily::Generic(GenericFamily::Monospace),
            ],
        ),
        (
            "font-family: 'sans-serif', Display Serif, serif",
            vec![
                named("sans-serif"),
                named("Display Serif"),
                FontFamily::Generic(GenericFamily::Serif),
            ],
        ),
        // `Serif` starts an entry, where the generic keyword claims it, so `Display` is
        // invalid and the declaration is dropped.
        (
            "font-family: monospace; font-family: Serif Display, serif",
            vec![FontFamily::Generic(GenericFamily::Monospace)],
        ),
        ("font-family: initial", initial),
    ] {
        assert_eq!(
            families(declaration),
            expected,
            "{declaration}: an unquoted generic keyword at the start of an entry is the generic family, later words of a multi-word name are family-name identifiers, and the initial family is serif"
        );
    }
}
