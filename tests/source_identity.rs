mod common;

use std::collections::BTreeMap;

#[test]
fn paragraph_identity_follows_source_order_across_flex_order_and_sizing_probes() {
    for css in [
        "section{display:flex;width:250pt}",
        "section{display:flex;width:150pt}#second{order:-1}",
        "section{display:flex;width:100pt;flex-wrap:wrap}#second{order:-1}",
    ] {
        let trace = common::render_trace(&format!(
            "<!doctype html><style>{}p{{margin:0}}{css}</style><section><p>First</p><p id=second>Second</p></section><p>After</p>",
            common::PROFILE
        ));
        let identities = trace
            .pages
            .iter()
            .flatten()
            .map(|line| (line.text.as_str(), line.paragraph))
            .collect::<BTreeMap<_, _>>();
        assert_eq!(
            identities,
            BTreeMap::from([("First", 0), ("Second", 1), ("After", 2)]),
            "source IDs must not depend on flex candidate measurement or paint order: {css}"
        );
    }
}
