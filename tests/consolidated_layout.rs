mod common;

use common::{PROFILE, assert_pt, local_options};
use ferri_pdf::{LinkShape, render_html};

const PAGE_MARGIN_PT: f32 = 20.0;
const COLUMN_PITCH_PT: f32 = 140.0;
const LINE_HEIGHT_PT: f32 = 18.0;

#[test]
fn column_fragmentation_preserves_links_and_destination_coordinates() {
    let html = format!(
        "<!doctype html><style>{PROFILE}@page{{size:300pt 200pt;margin:20pt}}\
         p{{margin:0}}.columns{{column-count:2;column-gap:20pt;column-fill:auto;height:36pt}}</style>\
         <div class=columns><p><a href='#target'>A</a><br>B<br><span id=target>C</span><br>\
         <a href='https://example.org/'>D</a></p></div>"
    );
    let trace = render_html(&html, &local_options())
        .unwrap()
        .layout
        .unwrap();
    assert_eq!(
        trace.pages.len(),
        1,
        "four lines fit two columns on one page"
    );
    let lines = &trace.pages[0];
    assert_eq!(
        lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>(),
        ["A", "B", "C", "D"]
    );
    for (line, x) in lines.iter().zip([
        PAGE_MARGIN_PT,
        PAGE_MARGIN_PT,
        PAGE_MARGIN_PT + COLUMN_PITCH_PT,
        PAGE_MARGIN_PT + COLUMN_PITCH_PT,
    ]) {
        assert_pt(line.x, x);
    }
    assert_pt(lines[0].baseline, lines[2].baseline);
    assert_pt(lines[1].baseline - lines[0].baseline, LINE_HEIGHT_PT);
    let destination = trace
        .destinations
        .iter()
        .flatten()
        .next()
        .expect("target in the second column must be located");
    assert_eq!(destination.page, 0);
    assert!(
        destination.top >= PAGE_MARGIN_PT && destination.top < PAGE_MARGIN_PT + LINE_HEIGHT_PT,
        "second-column target belongs to its first line: {destination:?}"
    );
    let second_column_link = trace.links[0]
        .iter()
        .find(|link| link.link == 1)
        .expect("external link survives adopted column paint");
    let LinkShape::Rect(rect) = second_column_link.shape else {
        panic!("untransformed link should be rectangular");
    };
    assert_pt(rect.x, PAGE_MARGIN_PT + COLUMN_PITCH_PT);
}

#[test]
fn forced_page_break_keeps_only_following_margin() {
    let html = format!(
        "<!doctype html><style>{PROFILE}@page{{size:300pt 200pt;margin:20pt}}\
         p{{margin:0}}.before{{margin-bottom:40pt}}.after{{break-before:page;margin-top:20pt}}</style>\
         <p class=before>Before</p><p class=after>After</p>"
    );
    let trace = render_html(&html, &local_options())
        .unwrap()
        .layout
        .unwrap();
    assert_eq!(trace.pages.len(), 2);
    assert_eq!(trace.pages[0][0].text, "Before");
    assert_eq!(trace.pages[1][0].text, "After");
    assert_pt(
        trace.pages[1][0].baseline - trace.pages[0][0].baseline,
        PAGE_MARGIN_PT,
    );
}

#[test]
fn bookmark_pseudo_metrics_use_the_render_jobs_resolver() {
    use ferri_pdf::{
        Resource, ResourceBase, ResourceError, ResourceResolver, render_with_resolver,
    };
    use std::cell::Cell;
    const MEMORY_FONT: &str = "memory:face";
    struct Fonts(Cell<usize>);
    impl ResourceResolver for Fonts {
        fn resolve(&self, reference: &str, base: &ResourceBase) -> Result<Resource, ResourceError> {
            assert_eq!(
                reference, MEMORY_FONT,
                "every font lookup must retain the job's resolver"
            );
            self.0.set(self.0.get() + 1);
            Ok(Resource {
                bytes: include_bytes!("oracle/assets/DejaVuSans.ttf").to_vec(),
                base: base.clone(),
            })
        }
    }
    let resolver = Fonts(Cell::new(0));
    let html = "<!doctype html><style>@font-face{font-family:Memory;src:url('memory:face')}\
                h1{bookmark-label:content(before)}h1::before{font-family:Memory;content:'Prefix';margin-left:1ex}</style><h1>Heading</h1>";
    let output = render_with_resolver(html, &local_options(), &resolver)
        .expect("bookmark pseudo metrics must use the supplied resolver");
    assert!(
        resolver.0.get() > 0,
        "the custom face must actually be measured"
    );
    assert!(
        output.layout.unwrap().pages[0]
            .iter()
            .any(|line| line.text.contains("Prefix"))
    );
}
