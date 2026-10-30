use crate::dom::{DocumentTree, Selector};
use crate::resources::FileResolver;
use crate::style::{LengthPercentage, Size, Style, Stylesheet, StylesheetSource};

fn styles(child: &str) -> (Style, Style) {
    let sheet = Stylesheet::parse(&[StylesheetSource {
        text: format!("#parent{{zoom:2;width:40pt;font-size:10pt;line-height:12pt}}#child{{width:inherit;font-size:inherit;line-height:inherit;{child}}}"),
        base: Default::default(),
    }]).unwrap();
    let html =
        DocumentTree::parse("<section id=parent><div id=child>Text</div></section>").unwrap();
    let select = |id| html.select(&Selector::parse(id).unwrap()).next().unwrap();
    let resolver = FileResolver::default();
    let parent = sheet
        .computed_with_resolver(select("#parent"), &Style::default(), &resolver)
        .unwrap();
    let child = sheet
        .computed_with_resolver(select("#child"), &parent.used(), &resolver)
        .unwrap();
    (parent, child)
}

#[test]
fn zoom_keeps_computed_lengths_and_root_bases_unscaled() {
    let (parent, child) = styles("zoom:.5");
    for style in [&parent, &child] {
        assert_eq!(style.size, 10.0, "computed inherited font size");
        assert_eq!(
            style.width,
            Size::Value(LengthPercentage::Length(40.0)),
            "computed explicit width inheritance"
        );
    }
    assert_eq!((parent.zoom, parent.effective_zoom), (2.0, 2.0));
    assert_eq!((child.zoom, child.effective_zoom), (0.5, 1.0));
    let used = parent.used();
    assert_eq!(used.size, 20.0, "used font size");
    assert_eq!(used.width.resolve(200.0), Some(80.0), "used width");
    assert_eq!(used.used(), used, "projection is applied once");
    assert_eq!(
        used.canonical(),
        &parent,
        "used styles retain canonical inheritance input"
    );
}

#[test]
fn anonymous_boxes_keep_used_adjustments_and_unscaled_inheritance_separate() {
    use crate::style::{Display, logical::WritingMode};
    let (parent, _) = styles("");
    let mut adjusted = parent.used();
    adjusted.writing_mode = WritingMode::VerticalRl;
    adjusted.size *= 3.0;
    let anonymous = adjusted.anonymous(Display::InlineBlock);
    for style in [&anonymous, &anonymous.anonymous(Display::Inline)] {
        assert_eq!(
            style.writing_mode,
            WritingMode::VerticalRl,
            "layout writing-mode adjustment must survive anonymous descendants"
        );
        assert_eq!(
            style.size, 60.0,
            "used initial-letter size must survive without another zoom projection"
        );
        assert_eq!(
            style.canonical().size,
            10.0,
            "CSS inheritance still uses the computed font size"
        );
        assert_eq!(
            style.canonical().writing_mode,
            parent.writing_mode,
            "a layout-only vertical annotation cannot change computed inheritance"
        );
        assert_eq!(
            style.used(),
            *style,
            "anonymous styles are already projected"
        );
    }
}

#[test]
fn zoom_defaults_and_explicit_inheritance_have_distinct_accumulation() {
    for (declaration, expected_own, expected_effective) in [
        ("", 1.0, 2.0),
        ("zoom:normal", 1.0, 2.0),
        ("zoom:0", 1.0, 2.0),
        ("zoom:0%", 1.0, 2.0),
        ("zoom:calc(-1)", 1.0, 2.0),
        ("zoom:inherit", 2.0, 4.0),
        ("zoom:50%", 0.5, 1.0),
        ("zoom:3;zoom:-1", 3.0, 6.0),
        ("zoom:3;zoom:reset", 3.0, 6.0),
        ("zoom:3;zoom:2 50%", 3.0, 6.0),
    ] {
        let (_, child) = styles(declaration);
        assert_eq!(
            (child.zoom, child.effective_zoom),
            (expected_own, expected_effective),
            "{declaration}"
        );
    }
}
