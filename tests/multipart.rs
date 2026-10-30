mod common;

use ferri_pdf::{HtmlPart, render_parts};

#[test]
fn parts_keep_css_page_geometry_and_local_traces_but_report_global_headings() {
    let index = "<style>@page{size:240pt 180pt;margin:12pt}p,h1{margin:0;font:10pt/12pt serif}</style><h1 id=index>Index</h1><p><a href='#body'>Body</a></p>";
    let body = "<style>@page{size:360pt 280pt;margin:24pt}p,h1{margin:0;font:20pt/24pt serif}</style><h1 id=body>Body</h1><p><a href='#index'>Index</a></p>";
    let output = render_parts(
        &[HtmlPart::from_html(index), HtmlPart::from_html(body)],
        &common::local_options(),
    )
    .expect("independent HTML parts must render as one navigable PDF");
    assert_eq!(output.page_count, 2);
    assert_eq!(output.parts.len(), 2);
    for (index, (part, paper)) in output
        .parts
        .iter()
        .zip([[240.0, 180.0], [360.0, 280.0]])
        .enumerate()
    {
        assert_eq!(part.first_page, index + 1);
        assert_eq!(part.page_count, 1);
        assert_eq!(part.headings[0].page, index + 1);
        let trace = part.layout.as_ref().expect("per-part trace requested");
        assert_eq!(trace.pages.len(), 1);
        assert_eq!(trace.page_records[0].geometry.paper, paper);
        assert_eq!(trace.links.len(), 1);
        assert!(
            !trace.links[0].is_empty(),
            "cross-part links need actual painted hit areas"
        );
    }
}

#[test]
fn byte_parts_use_standard_encoding_detection_and_empty_jobs_are_rejected() {
    let html = "<h1>Encoded café</h1>";
    let bytes: Vec<u8> = [0xff, 0xfe]
        .into_iter()
        .chain(html.encode_utf16().flat_map(u16::to_le_bytes))
        .collect();
    let output = render_parts(&[HtmlPart::from_bytes(&bytes)], &common::local_options())
        .expect("BOM-based UTF-16 detection applies to each part");
    assert_eq!(output.parts[0].headings[0].text, "Encoded café");
    assert!(render_parts(&[], &common::local_options()).is_err());
}

#[test]
fn custom_resolver_receives_each_parts_base_without_fetching_link_targets() {
    use ferri_pdf::{
        Resource, ResourceBase, ResourceError, ResourceResolver, Url, render_parts_with_resolver,
    };
    use std::cell::RefCell;

    struct Resolver(RefCell<Vec<String>>);
    impl ResourceResolver for Resolver {
        fn resolve(&self, reference: &str, base: &ResourceBase) -> Result<Resource, ResourceError> {
            let url = base.resolve(reference)?;
            self.0.borrow_mut().push(url.to_string());
            let css = match url.path() {
                "/first/style.css" => "p{font-size:14pt;color:red}",
                "/second/style.css" => "p{font-size:22pt;color:blue}",
                _ => return Err(ResourceError::Denied(url)),
            };
            Ok(Resource {
                bytes: css.as_bytes().to_vec(),
                base: url.into(),
            })
        }
    }
    let resolver = Resolver(RefCell::new(Vec::new()));
    let first = "<link rel=stylesheet href=style.css><p><a href='../second/input.html#second'>First</a></p>";
    let second = "<link rel=stylesheet href=style.css><p id=second>Second</p>";
    let parts = [
        HtmlPart::from_html(first)
            .with_base(Url::parse("https://example.org/first/input.html").unwrap()),
        HtmlPart::from_html(second)
            .with_base(Url::parse("https://example.org/second/input.html").unwrap()),
    ];
    let output = render_parts_with_resolver(&parts, &common::local_options(), &resolver).unwrap();
    for (part, (size, color)) in output
        .parts
        .iter()
        .zip([(14.0, [255, 0, 0]), (22.0, [0, 0, 255])])
    {
        let line = &part.layout.as_ref().unwrap().pages[0][0];
        assert_eq!(line.font_size, size);
        assert_eq!(line.color.rgb, color);
    }
    assert_eq!(
        *resolver.0.borrow(),
        [
            "https://example.org/first/style.css",
            "https://example.org/second/style.css"
        ]
    );
}

/// Boxes that overflow their part's page, as content-sized indexes with `height: 100%` do.
#[test]
fn parts_whose_boxes_overflow_the_page_render_like_single_documents() {
    const INDEX: &str = "<style>@page{size:A4;margin:1.5cm}html,body{font-size:12pt;margin:0;height:100%}\
        .row{display:flex;font-weight:bold}a{display:flex;flex:1 1 auto}\
        a>span:last-child{display:block;flex:1 1 auto;border-bottom:1pt dotted black}</style>\
        <h2>Index</h2><div><div class=row><a href='#alpha'><span>Alpha</span><span></span></a><span>1</span></div>\
        <div class=row><a href='#beta'><span>Beta</span><span></span></a><span>2</span></div></div>";
    let cases: [&[&str]; 9] = [
        &["<style>body { height: 100vh }</style>"],
        &["<style>body { height: calc(100vh + 1px); margin: 0 }</style>"],
        &["<div style='height: 200vh'></div>"],
        &["<style>html, body { height: 100% }</style>"],
        &["<style>html, body { height: 100% }</style><p>a</p>"],
        &[
            "<style>html, body { height: 100% }</style><p>a</p>",
            "<p>b</p>",
        ],
        &["<style>body { height: 100vh }</style><p>a</p>", "<p>b</p>"],
        &[INDEX],
        &[
            "<p>cover</p>",
            INDEX,
            "<h1 id=alpha>Alpha</h1><h2 id=beta>Beta</h2>",
        ],
    ];
    let options = ferri_pdf::RenderOptions::default();
    for parts in cases {
        let rendered = render_parts(
            &parts
                .iter()
                .copied()
                .map(HtmlPart::from_html)
                .collect::<Vec<_>>(),
            &options,
        )
        .unwrap_or_else(|error| panic!("{parts:?} must render: {error}"));
        let alone: usize = parts
            .iter()
            .map(|html| {
                ferri_pdf::render_html(html, &options)
                    .unwrap_or_else(|error| panic!("{html:?} must render alone: {error}"))
                    .page_count
            })
            .sum();
        assert_eq!(
            rendered.page_count, alone,
            "{parts:?}: parts paginate as their documents do alone"
        );
    }
}
