use ferri_pdf_dom::{DocumentTree, Selector, non_negative_integer_digits};

#[test]
fn integer_digits_follow_html_sign_prefix_and_negative_zero_rules() {
    for (source, expected) in [
        (" \t+0002tail", Some("0002")),
        ("-0000tail", Some("0000")),
        ("-0.5", Some("0")),
        ("-01", None),
        ("+-1", None),
        ("１２", None),
        ("", None),
        (" \n", None),
    ] {
        assert_eq!(non_negative_integer_digits(source), expected, "{source:?}");
    }
}

#[test]
fn heading_levels_include_offsets_resets_and_namespace_boundaries() {
    let markup = format!(
        "<!doctype html><body headingoffset=2>\
         <h1 id=base>A</h1><section headingoffset=1><h2 id=nested>B</h2>\
         <div headingreset=false headingoffset=2><h2 id=reset>C</h2></div></section>\
         <h1 id=own headingoffset='+2tail'>D</h1>\
         <h1 id=negative headingoffset=-1>E</h1>\
         <h1 id=aria aria-level=7>F</h1><div id=role role=heading>G</div>\
         <h6 id=huge headingoffset='{}'>H</h6>\
         <svg headingoffset=8 headingreset><foreignObject><h1 id=foreign>I</h1></foreignObject></svg>",
        "9".repeat(1024),
    );
    let tree = DocumentTree::parse(&markup).expect("parse test HTML");
    for (id, expected) in [
        ("base", Some(3)),
        ("nested", Some(5)),
        ("reset", Some(4)),
        ("own", Some(5)),
        ("negative", Some(3)),
        ("aria", Some(3)),
        ("role", None),
        ("huge", Some(9)),
        ("foreign", Some(3)),
    ] {
        let selector = Selector::parse(&format!("#{id}")).unwrap();
        let element = tree.select(&selector).next().unwrap();
        assert_eq!(element.heading_level(), expected, "#{id}: {element:?}");
    }
}
