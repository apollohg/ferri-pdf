use super::common;

use crate::document::encoding::{decode_css, decode_html};
use crate::{
    RenderDiagnostic, ResolvedResource, Resource, ResourceBase, ResourceError, ResourceMetadata,
    ResourceResolver, render_bytes, render_bytes_with_resolver, render_html,
};
use common::{assert_pt, local_options, render_trace};
use std::cell::Cell;

#[test]
fn html_encoding_precedence_and_unicode_entry_are_distinct() {
    for (bytes, transport, encoding, expected) in [
        (b"\xef\xbb\xbf<meta charset=windows-1252><p>caf\xc3\xa9".as_slice(), Some("windows-1251"), "UTF-8", "<meta charset=windows-1252><p>café"),
        (b"<meta charset=windows-1252><p>\xe9".as_slice(), Some("windows-1251"), "windows-1251", "<meta charset=windows-1252><p>й"),
        (b"<meta charset=latin1><p>caf\xe9".as_slice(), Some("utf-7"), "windows-1252", "<meta charset=latin1><p>café"),
        (b"<meta charset=x-user-defined><p>\x80".as_slice(), None, "windows-1252", "<meta charset=x-user-defined><p>€"),
        (b"<meta charset=utf-16le><p>caf\xc3\xa9".as_slice(), None, "UTF-8", "<meta charset=utf-16le><p>café"),
        (b"<!--<meta charset=latin1>--><p>caf\xc3\xa9".as_slice(), None, "UTF-8", "<!--<meta charset=latin1>--><p>café"),
        (b"<script>'<meta charset=latin1>'</script><p>caf\xc3\xa9".as_slice(), None, "UTF-8", "<script>'<meta charset=latin1>'</script><p>café"),
        (b"<link charset=latin1><p>caf\xc3\xa9".as_slice(), None, "UTF-8", "<link charset=latin1><p>café"),
        (b"<meta charset=bad http-equiv=content-type content='text/html;charset=latin1'><p>caf\xe9".as_slice(), None, "windows-1252", "<meta charset=bad http-equiv=content-type content='text/html;charset=latin1'><p>café"),
        (b"<?xml encoding='windows-1252'?><p>caf\xe9".as_slice(), None, "windows-1252", "<?xml encoding='windows-1252'?><p>café"),
    ] {
        let decoded = decode_html(bytes, transport).expect("decode test HTML");
        assert_eq!(decoded.text, expected, "HTML decoding with {transport:?}: {bytes:?}");
        assert_eq!(decoded.encoding, encoding);
        assert!(!decoded.had_errors, "valid bytes for {encoding}");
    }
    let unicode = format!(
        "<style>{}</style><meta charset=windows-1252><p>café",
        common::PROFILE
    );
    assert_eq!(
        render_trace(&unicode).pages[0][0].text,
        "café",
        "Unicode entry must ignore encoding declarations"
    );
    let bytes = b"<meta charset=windows-1252><p>caf\xe9";
    assert_eq!(
        render_bytes(bytes, &local_options())
            .unwrap()
            .layout
            .unwrap()
            .pages[0][0]
            .text,
        "café"
    );
}

#[test]
fn late_meta_and_utf16_restart_from_original_buffered_bytes() {
    const BEYOND_PRESCAN: usize = 1100;
    let mut bytes = format!(
        "<!--{}--><meta charset=windows-1252><p>caf",
        "x".repeat(BEYOND_PRESCAN)
    )
    .into_bytes();
    bytes.push(0xe9);
    let decoded = decode_html(&bytes, None).expect("decode test HTML");
    assert_eq!(decoded.encoding, "windows-1252");
    assert!(decoded.text.ends_with("<p>café"));
    assert!(
        !decoded.had_errors,
        "discarded tentative-decoder errors must not survive a successful restart"
    );
    for big_endian in [false, true] {
        let source = "<meta charset=windows-1252><p>café";
        let mut bytes = if big_endian {
            vec![0xfe, 0xff]
        } else {
            vec![0xff, 0xfe]
        };
        for unit in source.encode_utf16() {
            bytes.extend(if big_endian {
                unit.to_be_bytes()
            } else {
                unit.to_le_bytes()
            });
        }
        let decoded = decode_html(&bytes, Some("windows-1251")).expect("decode test HTML");
        assert_eq!(decoded.text, source);
        assert_eq!(
            decoded.encoding,
            if big_endian { "UTF-16BE" } else { "UTF-16LE" }
        );
    }
    let source = "<?xml version='1.0'?><p>café";
    let bytes: Vec<_> = source.encode_utf16().flat_map(u16::to_le_bytes).collect();
    assert_eq!(
        decode_html(&bytes, None).expect("decode test HTML").text,
        source,
        "UTF-16 XML prefix is recognized without a BOM"
    );
}

#[test]
fn css_encoding_transport_bom_charset_and_environment() {
    for (bytes, transport, environment, encoding, expected, errors) in [
        (
            b"@charset \"windows-1252\";p{content:'\xe9'}".as_slice(),
            None,
            Some("windows-1251"),
            "windows-1252",
            "@charset \"windows-1252\";p{content:'é'}",
            false,
        ),
        (
            b"p{content:'\xe9'}".as_slice(),
            None,
            Some("windows-1251"),
            "windows-1251",
            "p{content:'й'}",
            false,
        ),
        (
            b"p{content:'\xe9'}".as_slice(),
            None,
            Some("latin1"),
            "windows-1252",
            "p{content:'é'}",
            false,
        ),
        (
            b"\xef\xbb\xbfp{content:'caf\xc3\xa9'}".as_slice(),
            Some("windows-1252"),
            None,
            "UTF-8",
            "p{content:'café'}",
            false,
        ),
        (
            b"@charset \"windows-1252\";p{content:'\xe9'}".as_slice(),
            Some("UTF-8"),
            None,
            "UTF-8",
            "@charset \"windows-1252\";p{content:'�'}",
            true,
        ),
        (
            b"@charset \"utf-16le\";p{content:'caf\xc3\xa9'}".as_slice(),
            None,
            None,
            "UTF-8",
            "@charset \"utf-16le\";p{content:'café'}",
            false,
        ),
        (
            b"@Charset 'windows-1252';p{content:'\xe9'}".as_slice(),
            None,
            Some("windows-1251"),
            "windows-1251",
            "@Charset 'windows-1252';p{content:'й'}",
            false,
        ),
    ] {
        let decoded = decode_css(bytes, transport, environment);
        assert_eq!(
            decoded.text, expected,
            "CSS transport={transport:?}, environment={environment:?}"
        );
        assert_eq!(decoded.encoding, encoding);
        assert_eq!(decoded.had_errors, errors);
    }
}

struct SheetResolver {
    charset: Option<&'static str>,
    bytes: &'static [u8],
    calls: Cell<usize>,
}
impl ResourceResolver for SheetResolver {
    fn resolve(&self, reference: &str, base: &ResourceBase) -> Result<Resource, ResourceError> {
        assert_eq!(reference, "sheet.css");
        self.calls.set(self.calls.get() + 1);
        Ok(Resource {
            bytes: self.bytes.to_vec(),
            base: base.clone(),
        })
    }
    fn resolve_with_metadata(
        &self,
        reference: &str,
        base: &ResourceBase,
    ) -> Result<ResolvedResource, ResourceError> {
        Ok(ResolvedResource {
            resource: self.resolve(reference, base)?,
            metadata: ResourceMetadata {
                media_type: Some("text/css".into()),
                charset: self.charset.map(str::to_owned),
            },
        })
    }
}

#[test]
fn external_css_uses_transport_then_document_encoding_without_refetching() {
    for (document_encoding, transport, expected) in [
        ("windows-1251", None, "й"),
        ("windows-1252", None, "é"),
        ("windows-1251", Some("windows-1252"), "é"),
    ] {
        let resolver = SheetResolver {
            charset: transport,
            bytes: b"p{font-size:20pt}p::before{content:'\xe9'}",
            calls: Cell::new(0),
        };
        let html = format!(
            "<meta charset={document_encoding}><link rel=stylesheet href=sheet.css><p>body</p>"
        );
        let output =
            render_bytes_with_resolver(html.as_bytes(), &local_options(), &resolver).unwrap();
        let trace = output.layout.unwrap();
        assert_eq!(trace.pages[0][0].text, format!("{expected}body"));
        assert_pt(trace.pages[0][0].font_size, 20.0);
        assert!(output.diagnostics.is_empty());
        assert_eq!(resolver.calls.get(), 1);
    }
}

#[test]
fn replacement_and_unavailable_source_diagnostics_reach_render_output() {
    let output = render_bytes(b"<p>caf\xff", &local_options()).unwrap();
    assert_eq!(output.layout.unwrap().pages[0][0].text, "caf�");
    assert!(
        matches!(&output.diagnostics[..], [RenderDiagnostic::Decoding { encoding, .. }] if encoding == "UTF-8")
    );
    let output = render_html(
        "<link rel=stylesheet href=missing-foundations.css><p>survives</p>",
        &local_options(),
    )
    .unwrap();
    assert_eq!(output.layout.unwrap().pages[0][0].text, "survives");
    assert!(
        matches!(&output.diagnostics[..], [RenderDiagnostic::UnavailableSource { reference, .. }] if reference == "missing-foundations.css")
    );
}

#[test]
fn data_stylesheet_charset_controls_byte_decoding() {
    let html = "<link rel=stylesheet href=\"data:text/css;charset=windows-1252,p::before%7Bcontent:%27%E9%27%7D\"><p>body</p>";
    let output = render_html(html, &local_options()).unwrap();
    assert_eq!(output.layout.unwrap().pages[0][0].text, "ébody");
    assert!(output.diagnostics.is_empty());
}

const GRAPH_BASE: &str = "https://document.test/report/index.html";
struct GraphSheet {
    bytes: &'static [u8],
    final_url: &'static str,
    charset: Option<&'static str>,
    media_type: Option<&'static str>,
}
impl GraphSheet {
    fn css(final_url: &'static str, bytes: &'static [u8]) -> Self {
        Self {
            bytes,
            final_url,
            charset: None,
            media_type: Some("text/css"),
        }
    }
}
#[derive(Default)]
struct GraphResolver {
    sheets: std::collections::BTreeMap<&'static str, GraphSheet>,
    unavailable: std::collections::BTreeSet<&'static str>,
    calls: std::cell::RefCell<Vec<String>>,
}
fn graph_options() -> crate::RenderOptions {
    crate::RenderOptions {
        base: crate::Url::parse(GRAPH_BASE).unwrap().into(),
        ..local_options()
    }
}
impl ResourceResolver for GraphResolver {
    fn resolve(&self, reference: &str, base: &ResourceBase) -> Result<Resource, ResourceError> {
        self.resolve_with_metadata(reference, base)
            .map(|resolved| resolved.resource)
    }
    fn resolve_with_metadata(
        &self,
        reference: &str,
        base: &ResourceBase,
    ) -> Result<ResolvedResource, ResourceError> {
        let url = base.resolve(reference)?;
        self.calls.borrow_mut().push(url.to_string());
        if self.unavailable.contains(url.as_str()) {
            return Err(ResourceError::Read {
                path: url.as_str().into(),
                source: std::io::Error::from(std::io::ErrorKind::NotFound),
            });
        }
        let Some(sheet) = self.sheets.get(url.as_str()) else {
            return Err(ResourceError::Denied(url));
        };
        Ok(ResolvedResource {
            resource: Resource {
                bytes: sheet.bytes.to_vec(),
                base: crate::Url::parse(sheet.final_url).unwrap().into(),
            },
            metadata: ResourceMetadata {
                media_type: sheet.media_type.map(str::to_owned),
                charset: sheet.charset.map(str::to_owned),
            },
        })
    }
}

#[test]
fn imports_preserve_order_layers_and_redirect_bases() {
    const START: &str = "https://document.test/report/start.css";
    const NESTED: &str = "https://cdn.test/files/deep/inner.css";
    const FONT: &str = "https://cdn.test/v2/fonts/test.ttf";
    let resolver=GraphResolver {
        sheets:std::collections::BTreeMap::from([
            (START,GraphSheet::css("https://cdn.test/files/sheet.css",b"@import 'deep/inner.css';p{font-size:19pt}")),
            (NESTED,GraphSheet::css("https://cdn.test/v2/nested/sheet.css",b"@font-face{font-family:Imported;src:url('../fonts/test.ttf')}p{font-family:Imported;color:red}")),
            (FONT,GraphSheet::css(FONT,include_bytes!("../../assets/fonts/DejaVuSans.ttf"))),
        ]),..Default::default()
    };
    let html = "<!doctype html><style>@import 'start.css' layer(outer) supports(width:12pt) print;@layer final,outer;@layer final{p{color:#123456}}</style><p>Import</p>";
    let output = crate::render_with_resolver(html, &graph_options(), &resolver).unwrap();
    let trace = output.layout.unwrap();
    assert_pt(trace.pages[0][0].font_size, 19.0);
    assert_eq!(
        trace.pages[0][0].runs[0].color.rgb,
        [0x12, 0x34, 0x56],
        "the import layer registers at the import's source position"
    );
    assert_eq!(
        &*resolver.calls.borrow(),
        &[START, NESTED, FONT],
        "nested CSS and font references use each final response base"
    );
}

#[test]
fn repeated_imports_keep_distinct_encoding_and_layer_context() {
    const FIRST: &str = "https://document.test/report/first.css";
    const SECOND: &str = "https://document.test/report/second.css";
    const SHARED: &str = "https://document.test/report/shared.css";
    const PARENT: &str = "https://document.test/report/parent.css";
    let resolver = GraphResolver {
        sheets: std::collections::BTreeMap::from([
            (
                FIRST,
                GraphSheet::css(
                    PARENT,
                    b"@charset \"windows-1252\";@import 'shared.css' layer(nested);",
                ),
            ),
            (
                SECOND,
                GraphSheet::css(
                    PARENT,
                    b"@charset \"windows-1251\";@import 'shared.css' layer(nested);",
                ),
            ),
            (
                SHARED,
                GraphSheet::css(SHARED, b".\xe9::before{content:'\xe9'}p{font-size:19pt}"),
            ),
        ]),
        ..Default::default()
    };
    let html = "<!doctype html><style>@import 'first.css' layer(first);@import 'second.css' layer(second);</style><p class=é>One</p><p class=й>Two</p>";
    let trace = crate::render_with_resolver(html, &graph_options(), &resolver)
        .unwrap()
        .layout
        .unwrap();
    let text: Vec<_> = trace
        .pages
        .iter()
        .flatten()
        .map(|paragraph| paragraph.text.as_str())
        .collect();
    assert_eq!(
        text,
        ["éOne", "йTwo"],
        "a repeated URL must be decoded and applied in each referring encoding/context"
    );
    assert_eq!(
        resolver
            .calls
            .borrow()
            .iter()
            .filter(|url| url.as_str() == SHARED)
            .count(),
        1,
        "both referring sheets share a final base, so this must exercise a raw-response cache hit"
    );
    for paragraph in trace.pages.iter().flatten() {
        assert_pt(paragraph.font_size, 19.0);
    }
}

#[test]
fn false_import_conditions_do_not_fetch() {
    let resolver = GraphResolver::default();
    let html = "<!doctype html><style>@import 'grid.css' layer(first) supports(scroll-marker-group:before);@import 'screen.css' screen;@import 'unknown.css' supports(future-function());@layer second,first;@layer second{p{font-size:23pt}}@layer first{p{font-size:19pt}}</style><p>Inactive</p>";
    let trace = crate::render_with_resolver(html, &graph_options(), &resolver)
        .unwrap()
        .layout
        .unwrap();
    assert!(
        resolver.calls.borrow().is_empty(),
        "false import conditions must be evaluated before calling the resolver"
    );
    assert_pt(trace.pages[0][0].font_size, 19.0);
    let resolver = GraphResolver {
        unavailable: std::collections::BTreeSet::from(["https://document.test/report/missing.css"]),
        ..Default::default()
    };
    let html = "<!doctype html><style>@import 'missing.css' layer(first) supports(width:12pt);@layer second,first;@layer second{p{font-size:23pt}}@layer first{p{font-size:19pt}}</style><p>Unavailable</p>";
    let output = crate::render_with_resolver(html, &graph_options(), &resolver).unwrap();
    assert_pt(output.layout.unwrap().pages[0][0].font_size, 23.0);
    assert_eq!(
        output.diagnostics.len(),
        1,
        "an unavailable active import warns while retaining its layer registration"
    );
}

#[test]
fn import_cycle_and_limit_errors_preserve_policy() {
    const CYCLE: &str = "https://document.test/report/cycle.css";
    let resolver = GraphResolver {
        sheets: std::collections::BTreeMap::from([(
            CYCLE,
            GraphSheet::css(CYCLE, b"@import './cycle.css#again';p{font-size:19pt}"),
        )]),
        ..Default::default()
    };
    let output = crate::render_with_resolver(
        "<!doctype html><style>@import 'cycle.css';</style><p>Cycle</p>",
        &graph_options(),
        &resolver,
    )
    .expect("a cyclic edge is cut while the imported sheet's remaining rules still apply");
    assert_pt(output.layout.unwrap().pages[0][0].font_size, 19.0);
    assert_eq!(
        &*resolver.calls.borrow(),
        &[CYCLE],
        "canonical fragments cannot evade active-path cycle detection"
    );
    assert_eq!(
        output.diagnostics.len(),
        1,
        "a cut cycle retains a source diagnostic"
    );
    let resolver = GraphResolver::default();
    assert!(
        matches!(
            crate::render_with_resolver(
                "<!doctype html><style>@import 'denied.css';</style><p>Denied</p>",
                &graph_options(),
                &resolver
            ),
            Err(crate::Error::Css(crate::StyleError::Resource(
                ResourceError::Denied(_)
            )))
        ),
        "active import denial must never become an unavailable-source warning"
    );
}

#[test]
fn html_style_link_applicability_matches_initial_document() {
    const ACTIVE: &str = "https://document.test/report/active.css";
    let resolver = GraphResolver {
        sheets: std::collections::BTreeMap::from([(
            ACTIVE,
            GraphSheet::css(ACTIVE, b"p{font-size:19pt;color:#123456}"),
        )]),
        ..Default::default()
    };
    let html = "<!doctype html><style type='text/css; charset=utf-8'>p{scroll-marker-group:before}</style><style type=text/plain>p{scroll-marker-group:before}</style><style media=screen>p{scroll-marker-group:before}</style><link rel='preload StyleSheet' href=active.css media=print type='Text/CSS; charset=utf-8'><link rel=stylesheet disabled href=disabled.css><link rel='alternate stylesheet' title=Unselected href=alternate.css><link rel=stylesheet type=text/plain href=wrong-type.css><link rel=stylesheet media=screen href=screen.css><link rel=stylesheet><p>Applicable</p>";
    let mut options = graph_options();
    options.stylesheets.push("p{font-size:23pt}".into());
    let trace = crate::render_with_resolver(html, &options, &resolver)
        .unwrap()
        .layout
        .unwrap();
    assert_pt(trace.pages[0][0].font_size, 23.0);
    assert_eq!(trace.pages[0][0].runs[0].color.rgb, [0x12, 0x34, 0x56]);
    assert_eq!(&*resolver.calls.borrow(), &[ACTIVE]);
}

#[test]
fn shadow_imports_keep_their_tree_and_layer_context_after_redirects() {
    const COMMON: &str = "https://document.test/report/common.css";
    const ENTRY: &str = "https://document.test/report/entry.css";
    const FINAL_ENTRY: &str = "https://cdn.test/assets/entry.css";
    const NESTED: &str = "https://cdn.test/assets/common.css";
    const COMMON_CSS: &[u8] =
        b"@layer first,last;@layer first{p{font-size:17pt}}@layer last{p{font-size:19pt}}";
    let resolver = GraphResolver {
        sheets: std::collections::BTreeMap::from([
            (COMMON, GraphSheet::css(COMMON, COMMON_CSS)),
            (
                ENTRY,
                GraphSheet::css(FINAL_ENTRY, b"@layer last,first;@import 'common.css';"),
            ),
            (NESTED, GraphSheet::css(NESTED, COMMON_CSS)),
        ]),
        ..Default::default()
    };
    let html = "<!doctype html><link rel=stylesheet href=common.css><p>Outside</p><div><template shadowrootmode=open><link rel=stylesheet title=Ignored href=entry.css><p>Imported</p></template></div><div><template shadowrootmode=closed><link rel=stylesheet href=common.css><p>Shared</p></template></div><p>After</p>";
    let trace = crate::render_with_resolver(html, &graph_options(), &resolver)
        .unwrap()
        .layout
        .unwrap();
    let lines: Vec<_> = trace.pages.iter().flatten().collect();
    assert_eq!(
        lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>(),
        ["Outside", "Imported", "Shared", "After"]
    );
    for (line, expected) in lines.into_iter().zip([19.0, 17.0, 19.0, 19.0]) {
        assert_pt(line.font_size, expected);
    }
    assert_eq!(
        &*resolver.calls.borrow(),
        &[COMMON, ENTRY, NESTED],
        "redirected imports retain their owner context; cached bytes may serve another tree without merging its layers"
    );
}

#[test]
fn alternate_sheets_with_empty_effective_titles_remain_enabled() {
    const SHEET: &str = "https://document.test/report/untitled.css";
    for shadow in [true, false] {
        let resolver = GraphResolver {
            sheets: std::collections::BTreeMap::from([(
                SHEET,
                GraphSheet::css(SHEET, b"p{font-size:23pt}"),
            )]),
            ..Default::default()
        };
        let content = if shadow {
            "<div><template shadowrootmode=open><link rel='alternate stylesheet' title=Component href=untitled.css><p>Target</p></template></div><p>Outside</p>"
        } else {
            "<link rel='alternate stylesheet' href=untitled.css><p>Target</p>"
        };
        let html = format!(
            "<!doctype html><meta http-equiv=default-style content=Print><style title=Print>p{{font-size:13pt}}</style>{content}"
        );
        let trace = crate::render_with_resolver(&html, &graph_options(), &resolver)
            .unwrap()
            .layout
            .unwrap();
        let lines: Vec<_> = trace.pages.iter().flatten().collect();
        assert_eq!(lines[0].text, "Target");
        assert_eq!(
            lines[0].font_size, 23.0,
            "CSSOM enables empty effective titles even with the alternate flag; shadow={shadow}"
        );
        if shadow {
            assert_eq!(lines[1].text, "Outside");
            assert_pt(lines[1].font_size, 13.0);
        }
        assert_eq!(
            &*resolver.calls.borrow(),
            &[SHEET],
            "the enabled sheet is actually acquired; shadow={shadow}"
        );
    }
}

#[test]
fn initial_style_sets_use_case_sensitive_titles_and_meta_preference() {
    const SELECTED: &str = "https://document.test/report/selected.css";
    const UNTITLED: &str = "https://document.test/report/untitled.css";
    for (meta, expected_size, fetched) in [
        ("", 19.0, true),
        (
            "<meta http-equiv=default-style content=Second>",
            23.0,
            false,
        ),
    ] {
        let resolver = GraphResolver {
            sheets: std::collections::BTreeMap::from([
                (SELECTED, GraphSheet::css(SELECTED, b"p{color:#123456}")),
                (UNTITLED, GraphSheet::css(UNTITLED, b"p{padding-left:3pt}")),
            ]),
            ..Default::default()
        };
        let html = format!(
            "<!doctype html><style title=First>p{{font-size:19pt}}</style><style title=Second>p{{font-size:23pt}}</style><style title=first>p{{font-size:31pt}}</style><link rel='alternate stylesheet' title=First href=selected.css><link rel='alternate stylesheet' href=untitled.css><style disabled>@page{{margin:0}}p{{margin-left:7pt}}</style>{meta}<p>Set</p>"
        );
        let trace = crate::render_with_resolver(&html, &graph_options(), &resolver)
            .unwrap()
            .layout
            .unwrap();
        assert_pt(trace.pages[0][0].font_size, expected_size);
        assert_pt(trace.pages[0][0].x, 16.0);
        assert_eq!(
            &*resolver.calls.borrow(),
            &if fetched {
                vec![SELECTED, UNTITLED]
            } else {
                vec![UNTITLED]
            },
            "the selected named alternate and the empty-title sheet have independent enablement"
        );
        assert_eq!(
            trace.pages[0][0].runs[0].color.rgb,
            if fetched {
                [0x12, 0x34, 0x56]
            } else {
                [0, 0, 0]
            }
        );
    }
}

#[test]
fn link_charset_uses_valid_labels_before_the_document_encoding() {
    const SHEET: &str = "https://document.test/report/encoding.css";
    for (charset, expected) in [("windows-1252", "éBody"), ("invalid-encoding", "йBody")] {
        let resolver = GraphResolver {
            sheets: std::collections::BTreeMap::from([(
                SHEET,
                GraphSheet::css(SHEET, b"p::before{content:'\xe9'}"),
            )]),
            ..Default::default()
        };
        let html = format!(
            "<!doctype html><meta charset=windows-1251><link rel=stylesheet href=encoding.css charset={charset}><p>Body</p>"
        );
        let output =
            render_bytes_with_resolver(html.as_bytes(), &graph_options(), &resolver).unwrap();
        assert_eq!(output.layout.unwrap().pages[0][0].text, expected);
        assert!(output.diagnostics.is_empty());
        assert_eq!(resolver.calls.borrow().len(), 1);
    }
}

#[test]
fn stylesheet_mime_quirks_use_document_origin_and_final_response_url() {
    const SAME: &str = "https://document.test/report/plain.css";
    const OTHER: &str = "https://cdn.test/plain.css";
    for (doctype, base, request, response, expected) in [
        ("<!doctype html>", "", SAME, SAME, 12.0),
        ("", "", SAME, SAME, 19.0),
        ("", "", SAME, OTHER, 12.0),
        ("", "<base href='https://cdn.test/'>", OTHER, OTHER, 12.0),
    ] {
        let mut sheet = GraphSheet::css(response, b"p{font-size:19pt}");
        sheet.media_type = Some("text/plain");
        let resolver = GraphResolver {
            sheets: std::collections::BTreeMap::from([(request, sheet)]),
            ..Default::default()
        };
        let html = format!("{doctype}{base}<link rel=stylesheet href=plain.css><p>MIME</p>");
        let output = crate::render_with_resolver(&html, &graph_options(), &resolver).unwrap();
        assert_pt(output.layout.unwrap().pages[0][0].font_size, expected);
        assert_eq!(
            output.diagnostics.len(),
            usize::from(expected == 12.0),
            "unsupported resource types are optional source failures"
        );
    }
}

#[test]
fn import_placement_and_clauses_follow_their_stylesheet_boundary() {
    const VALID: &str = "https://document.test/report/valid.css";
    for prefix in ["p{}", "@layer block{}", "@media screen{}"] {
        let resolver = GraphResolver::default();
        let html =
            format!("<!doctype html><style>{prefix}@import 'denied.css';</style><p>Late</p>");
        let output = crate::render_with_resolver(&html, &graph_options(), &resolver).unwrap();
        assert!(
            resolver.calls.borrow().is_empty(),
            "a late import must not fetch after {prefix}"
        );
        assert!(
            !output.diagnostics.is_empty(),
            "a late import is discarded with a diagnostic"
        );
    }
    for import in [
        "@media print{@import 'denied.css';}",
        "@import 'denied.css' layer(a,b);",
        "@import 'denied.css' supports((width:12pt) and (color:red) or (display:flex));",
    ] {
        let resolver = GraphResolver::default();
        let html = format!("<!doctype html><style>{import}</style><p>Invalid</p>");
        let output = crate::render_with_resolver(&html, &graph_options(), &resolver).unwrap();
        assert!(
            resolver.calls.borrow().is_empty(),
            "invalid import: {import}"
        );
        assert!(!output.diagnostics.is_empty());
    }
    let resolver = GraphResolver {
        sheets: std::collections::BTreeMap::from([(
            VALID,
            GraphSheet::css(VALID, b"p{font-size:19pt}"),
        )]),
        ..Default::default()
    };
    let html = "<!doctype html><style>@unknown{}@layer before;@supports-condition --usable{width:12pt;}@import url('valid.css') layer(a.b) supports((--usable)) print;@layer a.b{p{color:#123456}}</style><p>Valid</p>";
    let trace = crate::render_with_resolver(html, &graph_options(), &resolver)
        .unwrap()
        .layout
        .unwrap();
    assert_pt(trace.pages[0][0].font_size, 19.0);
    assert_eq!(trace.pages[0][0].runs[0].color.rgb, [0x12, 0x34, 0x56]);
}

#[test]
fn import_discovery_resolves_names_without_changing_source_order() {
    const FIRST: &str = "https://document.test/report/first.css";
    const SEED: &str = "https://document.test/report/seed.css";
    let resolver = GraphResolver {
        sheets: std::collections::BTreeMap::from([
            (
                FIRST,
                GraphSheet::css(FIRST, b"p{font-size:19pt;color:#123456}"),
            ),
            (
                SEED,
                GraphSheet::css(SEED, b"@custom-media --later true;p{font-size:23pt}"),
            ),
        ]),
        ..Default::default()
    };
    let html = "<!doctype html><link rel=stylesheet href=first.css media='(--later)'><link rel=stylesheet href=seed.css><p>Forward</p>";
    let trace = crate::render_with_resolver(html, &graph_options(), &resolver)
        .unwrap()
        .layout
        .unwrap();
    assert_eq!(
        &*resolver.calls.borrow(),
        &[SEED, FIRST],
        "discovered definitions may enable an earlier pending source"
    );
    assert_pt(trace.pages[0][0].font_size, 23.0);
    assert_eq!(trace.pages[0][0].runs[0].color.rgb, [0x12, 0x34, 0x56]);
}

#[test]
fn conditional_stylesheet_sources_retain_paper_dependencies() {
    const SHEET: &str = "https://document.test/report/paper.css";
    let resolver = GraphResolver {
        sheets: std::collections::BTreeMap::from([(
            SHEET,
            GraphSheet::css(SHEET, b"@page{size:400pt 500pt!important}p{font-size:19pt}"),
        )]),
        ..Default::default()
    };
    let html = "<!doctype html><style>@import 'paper.css' (width > 1pt);@page{size:250pt 350pt}</style><p>Paper</p>";
    let trace = crate::render_with_resolver(html, &graph_options(), &resolver)
        .unwrap()
        .layout
        .unwrap();
    assert_eq!([trace.page_width, trace.page_height], [250.0, 350.0]);
    assert_pt(trace.pages[0][0].font_size, 19.0);
}

#[test]
fn nested_data_stylesheets_keep_identity_separate_from_their_reference_base() {
    let mut options = local_options();
    options.base = crate::Url::parse(GRAPH_BASE).unwrap().into();
    let html = "<!doctype html><style>@import \"data:text/css,@import%20%27data:text/css,p%257Bfont-size:19pt%257D%27;\";</style><p>Data</p>";
    let output =
        crate::render_with_resolver(html, &options, &crate::FileResolver::default()).unwrap();
    assert_pt(output.layout.unwrap().pages[0][0].font_size, 19.0);
    assert!(
        output.diagnostics.is_empty(),
        "inherited base URLs cannot make distinct embedded sheets look cyclic"
    );
}

#[test]
fn unavailable_preferred_stylesheets_do_not_disable_the_next_available_set() {
    const MISSING: &str = "https://document.test/report/missing-set.css";
    for (meta, expected) in [
        ("", 19.0),
        ("<meta http-equiv=default-style content=First>", 12.0),
    ] {
        let resolver = GraphResolver {
            unavailable: std::collections::BTreeSet::from([MISSING]),
            ..Default::default()
        };
        let html = format!(
            "<!doctype html><link rel=stylesheet href=missing-set.css title=First><style title=Second>p{{font-size:19pt}}</style>{meta}<p>Available set</p>"
        );
        let output = crate::render_with_resolver(&html, &graph_options(), &resolver).unwrap();
        assert_pt(output.layout.unwrap().pages[0][0].font_size, expected);
        assert_eq!(output.diagnostics.len(), 1);
    }
}
