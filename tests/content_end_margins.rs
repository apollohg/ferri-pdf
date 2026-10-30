//! CSS 2.1 §10.6.7 and CSS Break 3 §5.2: a formatting context root's automatic block size
//! reaches the block-end margin edge of its last in-flow child, which adjoins no break.
mod common;

use common::{assert_labelled_pt, painted_box, render_trace};

const CONTAINER: [u8; 3] = [1, 2, 3];
const MARGIN: f32 = 10.0;
const CHILD: f32 = 30.0;
/// Two children of block size `CHILD` with `MARGIN` around each, collapsing between them.
const CONTENT: f32 = 3.0 * MARGIN + 2.0 * CHILD;

fn container(css: &str) -> ferri_pdf::Rect {
    let trace = render_trace(&format!(
        "<!doctype html><style>@page{{size:400pt 400pt;margin:0}}body{{margin:0}}\
         .c{{background:rgb(1,2,3);margin-trim:none;{css}}}\
         .c>div{{margin:{MARGIN}pt;inline-size:50pt;block-size:{CHILD}pt}}</style>\
         <div class=c><div></div><div></div></div><p>after</p>"
    ));
    painted_box(&trace, CONTAINER)
}

#[test]
fn multicol_containers_keep_their_last_childs_block_end_margin() {
    let multicol = container("columns:1");
    assert_labelled_pt("multi-column container height", multicol.height, CONTENT);
}

#[test]
fn orthogonal_flows_keep_their_last_childs_block_end_margin() {
    // CSS Writing Modes 4 §7.3: the orthogonal flow's automatic block size, its width here,
    // fits its content, whose block axis runs across the page.
    let vertical = container("writing-mode:vertical-rl");
    assert_labelled_pt("vertical flow width", vertical.width, CONTENT);
    let lr = container("writing-mode:vertical-lr");
    assert_labelled_pt("vertical-lr flow width", lr.width, CONTENT);
}
