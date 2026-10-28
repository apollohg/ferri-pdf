mod common;

use common::{PROFILE, assert_pt, render_trace};
use ferri_pdf::{DisplayItem, LayoutTrace};

const MARKED: [u8; 3] = [0x12, 0x34, 0x56];

fn marked_boxes(trace: &LayoutTrace) -> usize {
    trace.display_lists.iter().flatten().filter(|item| matches!(item, DisplayItem::Box(paint) if paint.decoration.background.rgb == MARKED && paint.decoration.background.alpha > 0.0)).count()
}

#[test]
fn slots_remove_their_principal_box_and_allow_explicit_box_overrides() {
    for (display, expected) in [
        ("", 0),
        ("display:contents;", 0),
        ("display:inline;", 1),
        ("display:block;", 1),
    ] {
        let html = format!(
            "<!doctype html><style>{PROFILE}</style><div><template shadowrootmode=open><style>slot{{{display}background:#123456;padding:7pt;border:3pt solid red}}</style><slot></slot></template>Slotted</div>"
        );
        let trace = render_trace(&html);
        assert_eq!(
            marked_boxes(&trace),
            expected,
            "principal slot paint for {display:?}"
        );
        assert_eq!(
            trace
                .pages
                .iter()
                .flatten()
                .map(|line| line.text.as_str())
                .collect::<Vec<_>>(),
            ["Slotted"]
        );
    }
}

#[test]
fn contents_keep_inheritance_pseudos_and_inline_continuity() {
    let html = format!(
        "<!doctype html><style>{PROFILE}#unboxed{{display:contents;color:#123456;font-size:19pt;background:#123456;border:5pt solid red}}#unboxed::before{{content:'['}}#unboxed::after{{content:']';display:contents}}#unboxed>span{{font-size:inherit}}</style><div>Before<div id=unboxed><span>Inside</span></div>After</div>"
    );
    let trace = render_trace(&html);
    let lines: Vec<_> = trace.pages.iter().flatten().collect();
    assert_eq!(lines.len(), 1, "unboxing must not add paragraph boundaries");
    assert_eq!(lines[0].text, "Before[Inside]After");
    assert_eq!(
        marked_boxes(&trace),
        0,
        "contents elements and pseudos generate no principal decoration"
    );
    let inside = lines[0]
        .runs
        .iter()
        .find(|run| run.text.contains("Inside"))
        .unwrap();
    assert_eq!(inside.color.rgb, MARKED);
    assert_pt(inside.font_size, 19.0);
}

#[test]
fn contents_on_replaced_elements_computes_to_none_before_resource_acquisition() {
    let html = format!(
        "<!doctype html><style>{PROFILE}.hidden{{display:contents}}</style><div>A<img class=hidden src='missing-image.png'><iframe class=hidden src='missing-page.html'>No frame</iframe><textarea class=hidden>No textarea</textarea><svg class=hidden><text>No SVG</text></svg><math class=hidden><mi>No math</mi></math>B</div>"
    );
    let trace = render_trace(&html);
    assert_eq!(
        trace
            .pages
            .iter()
            .flatten()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>(),
        ["AB"]
    );
}

#[test]
fn contents_links_keep_text_and_block_descendants_without_adding_a_font_strut() {
    let html = format!(
        "<!doctype html><style>{PROFILE}a{{display:contents;background:#123456;font-size:80pt}}a span{{font-size:10pt}}</style><p>A<a href='https://inline.example/'><span>linked</span></a>Z</p><p>Next</p><a href='https://block.example/'><div>Block link</div><span>After block</span></a><p><a href='https://direct.example/' style='font-size:12pt'>Direct-only</a></p>"
    );
    let trace = render_trace(&html);
    assert_eq!(marked_boxes(&trace), 0);
    let lines: Vec<_> = trace.pages.iter().flatten().collect();
    let reference = render_trace(&html.replace("font-size:80pt", "font-size:10pt"));
    let reference_lines: Vec<_> = reference.pages.iter().flatten().collect();
    assert_pt(
        lines[1].baseline - lines[0].baseline,
        reference_lines[1].baseline - reference_lines[0].baseline,
    );
    for link in [0, 1, 2] {
        assert!(
            trace.links.iter().flatten().any(|area| area.link == link),
            "contents hyperlink {link} keeps visible descendant hit areas"
        );
    }
    for area in trace.links.iter().flatten().filter(|area| area.link == 0) {
        if let ferri_pdf::LinkShape::Rect(rect) = area.shape {
            assert!(
                rect.height <= lines[0].height,
                "a boxless ancestor's 80pt font must not enlarge the hit area: {rect:?}"
            );
        }
    }
}

#[test]
fn unboxed_ordered_lists_do_not_apply_their_boxless_counter_reset() {
    let html = format!(
        "<!doctype html><style>{PROFILE}ol{{display:contents}}</style><ol start=4><li>First</li><li>Second</li></ol>"
    );
    let trace = render_trace(&html);
    let markers: Vec<_> = trace
        .pages
        .iter()
        .flatten()
        .filter(|line| line.text.ends_with(". "))
        .map(|line| line.text.as_str())
        .collect();
    assert_eq!(
        markers,
        ["1. ", "2. "],
        "counter-reset hints have no effect on an element that generates no box"
    );
}

#[test]
fn contents_hoist_flex_items_and_table_cells_into_the_real_container() {
    let flex = render_trace(&format!(
        "<!doctype html><style>{PROFILE}#flex{{display:flex;width:200pt}}#strip{{display:contents;width:999pt;position:absolute;background:#123456}}.item{{width:100pt;flex:none}}</style><div id=flex><section id=strip><div class=item>Left</div><div class=item>Right</div></section></div>"
    ));
    let lines: Vec<_> = flex.pages.iter().flatten().collect();
    assert_eq!(
        lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>(),
        ["Left", "Right"]
    );
    assert_pt(lines[1].x - lines[0].x, 100.0);
    assert_pt(lines[1].baseline, lines[0].baseline);
    assert_eq!(marked_boxes(&flex), 0);
    let table = |display: &str| {
        render_trace(&format!(
            "<!doctype html><style>{PROFILE}table{{border-spacing:0;width:160pt}}td{{width:80pt;padding:0}}tbody,tr{{{display}}}</style><table><tbody><tr><td>Left</td><td>Right</td></tr></tbody></table>"
        ))
    };
    let reference = table("");
    let unboxed = table("display:contents");
    for (actual, expected) in unboxed
        .pages
        .iter()
        .flatten()
        .zip(reference.pages.iter().flatten())
    {
        assert_eq!(actual.text, expected.text);
        assert_pt(actual.x, expected.x);
        assert_pt(actual.baseline, expected.baseline);
    }
    assert_eq!(unboxed.pages.iter().flatten().count(), 2);
}

#[test]
fn contents_drop_their_own_text_decoration_but_preserve_real_ancestor_decoration() {
    let plain = render_trace(&format!(
        "<!doctype html><style>{PROFILE}#strip{{display:contents;text-decoration:underline}}</style><div id=strip><span>Inline</span><p>Block</p></div>"
    ));
    assert!(
        !plain
            .display_lists
            .iter()
            .flatten()
            .any(|item| matches!(item, DisplayItem::Decoration(_))),
        "boxless declarations do not propagate decoration"
    );
    let inherited = render_trace(&format!(
        "<!doctype html><style>{PROFILE}p{{text-decoration:underline}}#strip{{display:contents}}</style><p><span id=strip><em>Inherited</em></span></p>"
    ));
    assert!(
        inherited
            .display_lists
            .iter()
            .flatten()
            .any(|item| matches!(item, DisplayItem::Decoration(_))),
        "real ancestor decoration crosses boxless elements"
    );
}

#[test]
fn contents_unbox_html_elements_whose_normal_principal_box_is_special() {
    let html = format!(
        "<!doctype html><style>{PROFILE}button,fieldset,legend,details{{display:contents}}</style><div><button>Button</button><fieldset><legend>Legend</legend><span>Field</span></fieldset><details open><span>Details</span></details></div>"
    );
    let trace = render_trace(&html);
    assert_eq!(
        trace
            .pages
            .iter()
            .flatten()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>(),
        ["ButtonLegendFieldDetails"]
    );
}

#[test]
fn boxless_link_geometry_follows_transformed_text_and_bidi_runs() {
    for (prefix, label, expected) in [("ẞ", "LINK", "link"), ("A", "אב", "אב")] {
        let trace = render_trace(&format!(
            "<!doctype html><style>{PROFILE}p{{text-transform:lowercase}}a{{display:contents}}</style><p>{prefix}<a href='https://text.example/'>{label}</a>Z</p>"
        ));
        let line = &trace.pages[0][0];
        let run = line
            .runs
            .iter()
            .find(|run| run.text == expected)
            .expect("the linked source has a distinct glyph run");
        let areas: Vec<_> = trace
            .links
            .iter()
            .flatten()
            .filter(|area| area.link == 0)
            .collect();
        assert_eq!(areas.len(), 1, "one visible linked run in {label}");
        let ferri_pdf::LinkShape::Rect(rect) = areas[0].shape else {
            panic!("untransformed link geometry must be rectangular");
        };
        assert_pt(rect.x, line.x + run.x);
        assert_pt(rect.width, run.width);
    }
}

#[test]
fn boxless_backgrounds_and_border_images_are_not_acquired() {
    use ferri_pdf::{FileResolver, Resource, ResourceBase, ResourceError, ResourceResolver};
    use std::cell::RefCell;
    const UNUSED_IMAGE: &str = "unboxed-never-load.png";
    #[derive(Default)]
    struct Observed {
        calls: RefCell<Vec<String>>,
        files: FileResolver,
    }
    impl ResourceResolver for Observed {
        fn resolve(&self, reference: &str, base: &ResourceBase) -> Result<Resource, ResourceError> {
            self.calls.borrow_mut().push(reference.to_owned());
            self.files.resolve(reference, base)
        }
    }
    let resolver = Observed::default();
    let html = format!(
        "<!doctype html><style>{PROFILE}#strip,#strip::before{{display:contents;background-image:url({UNUSED_IMAGE});border-image-source:url({UNUSED_IMAGE})}}#strip::before{{content:'Before'}}</style><div id=strip>Text</div>"
    );
    let trace = ferri_pdf::render_with_resolver(&html, &common::local_options(), &resolver)
        .unwrap()
        .layout
        .unwrap();
    assert_eq!(trace.pages[0][0].text, "BeforeText");
    assert!(
        !resolver
            .calls
            .borrow()
            .iter()
            .any(|reference| reference.contains(UNUSED_IMAGE)),
        "no principal-box resource requests: {:?}",
        resolver.calls.borrow()
    );
}

#[test]
fn the_canvas_uses_body_background_values_even_when_its_box_is_removed() {
    for (display, expected) in [("contents", 1), ("inline", 1), ("block", 1), ("none", 0)] {
        let trace = render_trace(&format!(
            "<!doctype html><style>{PROFILE}body{{display:{display};background:#123456}}</style><div>Body</div>"
        ));
        assert_eq!(
            marked_boxes(&trace),
            expected,
            "canvas background for body display:{display}"
        );
    }
    let hidden_root = render_trace(&format!(
        "<!doctype html><style>{PROFILE}html{{display:none;background:#123456;border:4pt solid red;height:2000pt}}</style><p>Hidden</p>"
    ));
    assert_eq!(
        marked_boxes(&hidden_root),
        0,
        "a hidden root has no canvas background"
    );
    assert_eq!(
        hidden_root.pages.len(),
        1,
        "hidden root dimensions create no extra pages"
    );
    assert!(hidden_root.pages[0].is_empty());
}

#[test]
fn canvas_propagation_preserves_computed_values_for_explicit_inheritance() {
    for display in ["contents", "inline", "block"] {
        let trace = render_trace(&format!(
            "<!doctype html><style>{PROFILE}body{{display:{display};background:#123456}}div{{background:inherit}}</style><div>Inherited background</div>"
        ));
        assert_eq!(
            marked_boxes(&trace),
            2,
            "canvas plus explicitly inherited child background for {display}"
        );
    }
}

#[test]
fn a_legend_in_an_unboxed_fieldset_uses_ordinary_box_layout() {
    let trace = render_trace(&format!(
        "<!doctype html><style>{PROFILE}fieldset{{display:contents}}legend{{display:block}}</style><fieldset><legend>Legend</legend><span>Field</span></fieldset>"
    ));
    assert_eq!(
        trace
            .pages
            .iter()
            .flatten()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>(),
        ["Legend", "Field"]
    );
}

#[test]
fn propagated_body_background_is_not_repainted_as_a_table_layer() {
    let trace = render_trace(&format!(
        "<!doctype html><style>{PROFILE}body{{display:table-row-group;background:#123456}}.row{{display:table-row}}.cell{{display:table-cell}}</style><div class=row><span class=cell>Cell</span></div>"
    ));
    assert_eq!(
        marked_boxes(&trace),
        1,
        "the canvas consumes the body's used background, including table layers"
    );
    assert!(trace.pages.iter().flatten().any(|line| line.text == "Cell"));
}
