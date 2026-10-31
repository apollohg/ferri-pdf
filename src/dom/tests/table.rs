use crate::dom::{DocumentTree, NamespaceMap, Selector};

/// Row-spanning, downward-growing and column-spanning cells across two row groups, with a
/// span-only column group and one holding `col` elements (HTML §4.9.12.1). Columns, counted
/// from one: `a` 1, `b` 2–3, `c` 4, `d` 2, `e` 3, `f` 4, `g` 1–3, `h` 4; five in all.
const TABLE: &str = "<!doctype html><table class=outer>\
    <colgroup span=2 id=first></colgroup>\
    <colgroup id=second><col id=third><col span=2 id=wide></colgroup>\
    <tr><td id=a rowspan=2>a<td id=b colspan=2>b<th id=c>c\
    <tr><td id=d>d<td id=e rowspan=0>e<td id=f>f\
    <tbody><tr><td id=g colspan=3>g<td id=h>h\
    <tr><td id=i><table><col id=inner><tr><td id=nested>n</table>\
    </table>";

fn matched(tree: &DocumentTree, selector: &str) -> Vec<String> {
    let parsed = Selector::parse(selector)
        .unwrap_or_else(|error| panic!("{selector} should parse: {error:?}"));
    tree.select(&parsed)
        .map(|element| element.attr("id").unwrap_or(element.name()).to_owned())
        .collect()
}

#[test]
fn column_combinator_matches_cells_in_any_column_of_the_column_element() {
    let example = DocumentTree::parse(
        "<!doctype html><table><col span=2><col class=selected>\
         <tr><td>A<td>B<td id=C>C<tr><td colspan=2>D<td id=E>E\
         <tr><td>F<td id=G colspan=2>G</table>",
    );
    assert_eq!(
        matched(&example, "col.selected || td"),
        ["C", "E", "G"],
        "Selectors 5 §8.1 example"
    );
    let tree = DocumentTree::parse(TABLE);
    for (selector, expected) in [
        ("#wide || td", &["f", "h"][..]),
        ("#wide || :is(td, th)", &["c", "f", "h"]),
        ("#wide||td", &["f", "h"]),
        ("#third || *", &["b", "e", "g"]),
        ("#first || td", &["a", "b", "d", "g", "i"]),
        ("colgroup || #e", &["e"]),
        ("col || td", &["b", "e", "f", "g", "h", "nested"]),
        (
            "table.outer > colgroup > col || td",
            &["b", "e", "f", "g", "h"],
        ),
        (
            "table.outer col || td",
            &["b", "e", "f", "g", "h", "nested"],
        ),
        ("#inner || td", &["nested"]),
        ("#first || td ~ #f", &["f"]),
        ("#wide || td + td", &[]),
        ("#second || td > *", &[]),
        ("tr td || td", &[]),
    ] {
        assert_eq!(matched(&tree, selector), expected, "{selector}");
    }
}

#[test]
fn column_combinator_inside_has_searches_the_whole_table() {
    let tree = DocumentTree::parse(TABLE);
    for (selector, expected) in [
        ("col:has(|| #e)", &["third"][..]),
        ("colgroup:has(> col || #c)", &["second"]),
        ("colgroup:has(|| td[rowspan])", &["first", "second"]),
        ("table:has(#inner || td)", &["table", "table"]),
        ("col:has(|| #nested)", &["inner"]),
    ] {
        assert_eq!(matched(&tree, selector), expected, "{selector}");
    }
}

#[test]
fn nth_col_counts_every_column_a_cell_spans_from_either_end() {
    let tree = DocumentTree::parse(TABLE);
    for (selector, expected) in [
        (":nth-col(1)", &["a", "g", "i", "nested"][..]),
        ("td:nth-col(2)", &["b", "d", "g"]),
        (":nth-col(3)", &["b", "e", "g"]),
        (":nth-col(4)", &["c", "f", "h"]),
        (":nth-col(5)", &[]),
        (":nth-col(2n+3)", &["b", "e", "g"]),
        (":nth-col(-n+1)", &["a", "g", "i", "nested"]),
        (":nth-last-col(1)", &["nested"]),
        (":nth-last-col(2)", &["c", "f", "h"]),
        (":nth-last-col(odd)", &["a", "b", "e", "g", "i", "nested"]),
        ("col:nth-col(1), tr:nth-col(1)", &[]),
    ] {
        assert_eq!(matched(&tree, selector), expected, "{selector}");
    }
    let math = DocumentTree::parse(
        "<!doctype html><math><mtable><mtr><mtd id=x columnspan=2>x</mtd><mtd id=y>y</mtd>\
         </mtr><mtr><mtd id=z>z</mtd></mtr></mtable></math>",
    );
    assert_eq!(matched(&math, ":nth-col(2)"), ["x"], "MathML Core §3.5.3");
    assert_eq!(matched(&math, ":nth-last-col(1)"), ["y"]);
}

#[test]
fn column_combinator_does_not_take_a_namespace_separator() {
    let mut namespaces = NamespaceMap::new();
    namespaces.insert("h".into(), crate::dom::HTML_NAMESPACE.into());
    let tree = DocumentTree::parse(TABLE);
    for (selector, expected) in [
        ("h|col || h|td", &["b", "e", "f", "g", "h", "nested"][..]),
        ("*|col||*|td#a", &[]),
        ("*||#a", &["a"]),
        ("h|*||h|*#nested", &["nested"]),
        ("col |||td", &[]),
    ] {
        let parsed = Selector::parse_with_namespaces(selector, &namespaces, false)
            .unwrap_or_else(|error| panic!("{selector} should parse: {error:?}"));
        let found: Vec<_> = tree
            .select(&parsed)
            .map(|element| element.attr("id").unwrap_or_default().to_owned())
            .collect();
        assert_eq!(found, expected, "{selector}");
    }
    for invalid in ["col | td", "|| td", "col ||", "[a||b]"] {
        assert!(
            Selector::parse_with_namespaces(invalid, &namespaces, false).is_err(),
            "{invalid} should be invalid"
        );
    }
}
