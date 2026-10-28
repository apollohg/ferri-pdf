use crate::{
    RenderOptions, Resource, ResourceBase, ResourceError, ResourceResolver, render_html,
    render_with_resolver,
};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

const FONT: &[u8] = include_bytes!("../../tests/oracle/assets/DejaVuSans.ttf");

fn asset_path(base: &ResourceBase, reference: &str) -> PathBuf {
    match base {
        ResourceBase::Directory(path) => path.join(reference),
        ResourceBase::Url(url) => panic!("local test asset received remote base: {url}"),
    }
}

struct Assets(HashMap<PathBuf, Vec<u8>>);
impl ResourceResolver for Assets {
    fn resolve(&self, reference: &str, base: &ResourceBase) -> Result<Resource, ResourceError> {
        let path = asset_path(base, reference);
        let bytes = self
            .0
            .get(&path)
            .cloned()
            .ok_or_else(|| ResourceError::Read {
                path: path.clone(),
                source: std::io::Error::from(std::io::ErrorKind::NotFound),
            })?;
        Ok(Resource {
            bytes,
            base: path.parent().unwrap().into(),
        })
    }
}

// HTML rendering defaults: 40px list padding at 96 CSS px per inch.
const HTML_LIST_CONTENT_X_PT: f32 = 36.0 + 40.0 * ferri_pdf_style::POINTS_PER_PIXEL;
// DejaVu Sans hmtx: bullet 1208 + space 651 units, at 12pt with 2048 units/em.
const BULLET_AND_SPACE_ADVANCE_PT: f32 = (1208.0 + 651.0) / 2048.0 * 12.0;

#[test]
fn file_resource_limit_and_root_apply_before_font_parsing() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let resolver = crate::FileResolver {
        max_resource_bytes: 4,
        ..Default::default()
    };
    assert!(
        matches!(
            resolver.resolve("crates/fonts/assets/fonts/DejaVuSans.ttf", &root.into()),
            Err(ResourceError::TooLarge { limit: 4, .. })
        ),
        "oversized resources must be rejected rather than read without a limit"
    );
    let resolver = crate::FileResolver {
        root: Some(root.join("tests/oracle/fixtures")),
        ..Default::default()
    };
    assert!(
        matches!(
            resolver.resolve("crates/fonts/assets/fonts/DejaVuSans.ttf", &root.into()),
            Err(ResourceError::OutsideRoot(_))
        ),
        "root restrictions must apply to canonical resources, not just URL strings"
    );
}

#[test]
fn nested_percentage_boxes_resolve_padding_and_auto_margins_against_containing_width() {
    let html = format!(
        "<style>{PROFILE}</style><section style='width:50%;margin:0 auto;padding:10%'><p style='width:50%;margin:0 0 0 10%'>A</p><div style='padding:6pt'><p style='margin:0'>B</p></div></section><p style='margin:0'>C</p>"
    );
    let trace = render_html(&html, &local_options())
        .expect("nested blocks must establish containing widths")
        .layout
        .unwrap();
    let lines = &trace.pages[0];
    assert_eq!(
        lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>(),
        ["A", "B", "C"]
    );
    const A_X_PT: f32 = 122.4;
    const B_X_PT: f32 = 114.0;
    const FIRST_BASELINE_PT: f32 = 77.9543;
    assert!(
        (lines[0].x - A_X_PT).abs() < 0.001,
        "percentage padding and margins use their respective containing widths: {}",
        lines[0].x
    );
    assert!(
        (lines[1].x - B_X_PT).abs() < 0.001,
        "nested padding must move the child content: {}",
        lines[1].x
    );
    assert!(
        (lines[0].baseline - FIRST_BASELINE_PT).abs() < 0.001,
        "top padding must remain at a page start: {}",
        lines[0].baseline
    );
}

#[test]
fn nested_parent_child_margins_collapse_as_one_strut() {
    let html = format!(
        "<style>{PROFILE}</style><div style='margin:24pt 0 30pt'><p style='margin:18pt 0 12pt'>A</p><div style='margin:-6pt 0 20pt'><p style='margin:0 0 14pt'>B</p></div></div><p style='margin:10pt 0 0'>C</p>"
    );
    let trace = render_html(&html, &local_options())
        .expect("nested margin collapse must be represented by the block tree")
        .layout
        .unwrap();
    let lines = &trace.pages[0];
    assert_eq!(lines.len(), 3);
    assert!(
        (lines[1].baseline - lines[0].baseline - 24.0).abs() < 0.001,
        "sibling margins must combine across parent/child edges"
    );
    assert!(
        (lines[2].baseline - lines[1].baseline - 48.0).abs() < 0.001,
        "trailing child margins must collapse with ancestor and next-sibling margins"
    );
}

#[test]
fn margins_inside_formatting_context_roots_survive_page_breaks() {
    let trace = render_html(
        include_str!("../../tests/oracle/fixtures/margin-formatting-context.html"),
        &local_options(),
    )
    .expect("margin fixture must render")
    .layout
    .unwrap();
    const PAGE_TOP_BASELINE_PT: f32 = 49.1543;
    const MARGIN_PT: f32 = 20.0;
    // Margins that start a cell, scroll container or flex item are kept after forced
    // and unforced breaks alike, and truncates those that adjoin the break.
    let expected = [
        (
            2,
            "Cell after an unforced break",
            PAGE_TOP_BASELINE_PT + MARGIN_PT,
        ),
        (
            3,
            "Cell after a forced break",
            PAGE_TOP_BASELINE_PT + MARGIN_PT,
        ),
        (5, "Overflow hidden", PAGE_TOP_BASELINE_PT + MARGIN_PT),
        (7, "Flex item", PAGE_TOP_BASELINE_PT + MARGIN_PT),
        (9, "Plain block truncates", PAGE_TOP_BASELINE_PT),
        (11, "Table margin truncates", PAGE_TOP_BASELINE_PT),
    ];
    let actual: Vec<_> = trace
        .pages
        .iter()
        .enumerate()
        .flat_map(|(index, page)| page.iter().map(move |line| (index + 1, line)))
        .collect();
    assert_eq!(
        actual
            .iter()
            .map(|(page, line)| (*page, line.text.as_str()))
            .collect::<Vec<_>>(),
        expected
            .iter()
            .map(|&(page, text, _)| (page, text))
            .collect::<Vec<_>>(),
        "lines by page"
    );
    for ((page, text, baseline), (_, line)) in expected.into_iter().zip(actual) {
        assert!(
            (line.baseline - baseline).abs() < 0.001,
            "page {page} {text:?}: expected baseline {baseline}, actual {}",
            line.baseline
        );
    }
}

#[test]
fn box_sizing_and_containing_direction_control_auto_and_overconstrained_margins() {
    let html = format!(
        "<style>{PROFILE}</style><div style='width:50%;padding:0 12pt;box-sizing:border-box;margin:0 auto'>A</div><div style='width:50%;padding:0 12pt;margin:0 auto'>B</div><div dir='rtl'><div style='width:72pt;padding:0 6pt;text-align:left'>R</div><div style='width:72pt;padding:0 6pt;margin:0 auto;text-align:left'>S</div></div>"
    );
    let trace = render_html(&html, &local_options())
        .unwrap()
        .layout
        .unwrap();
    const EXPECTED_CONTENT_X_PT: [f32; 4] = [120.0, 108.0, 246.0, 144.0];
    assert_eq!(trace.pages[0].len(), EXPECTED_CONTENT_X_PT.len());
    for (line, expected) in trace.pages[0].iter().zip(EXPECTED_CONTENT_X_PT) {
        assert!(
            (line.x - expected).abs() < 0.001,
            "{} content box x {}, expected {expected}; auto margins and overconstraints depend on containing direction",
            line.text,
            line.x
        );
    }
}

#[test]
fn anonymous_blocks_and_explicit_line_breaks_preserve_flow_order() {
    let html = format!(
        "<style>{PROFILE}</style>Before <span style='color:red'>inline<div>Middle</div> after</span> end<p>Paragraph<br>line<br><br>tail</p>After"
    );
    let trace = render_html(&html, &local_options())
        .expect("inline content must split into anonymous blocks around block children")
        .layout
        .unwrap();
    let lines = &trace.pages[0];
    assert_eq!(
        lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>(),
        [
            "Before inline",
            "Middle",
            "after end",
            "Paragraph",
            "line",
            "",
            "tail",
            "After"
        ]
    );
    assert!(
        lines[1].runs.iter().all(|r| r.color.rgb == [255, 0, 0]),
        "a block inside an inline element must inherit the inline element's text style"
    );
    assert!(
        (lines[5].baseline - lines[4].baseline - 18.0).abs() < 0.001,
        "an empty forced line must still occupy a line box"
    );
}

#[test]
fn display_none_prunes_subtrees_and_display_controls_box_generation() {
    let html = format!(
        "<style>{PROFILE}</style><div style='display:none'><p style='unsupported:invalid'>Hidden text</p></div><p style='display:inline'>Inline </p><span style='display:block;margin:6pt 0'>Block</span><p style='display:inline'>Tail</p>"
    );
    let trace = render_html(&html, &local_options())
        .expect("display must control formatting rather than an HTML tag allowlist")
        .layout
        .unwrap();
    assert_eq!(
        trace.pages[0]
            .iter()
            .map(|l| l.text.as_str())
            .collect::<Vec<_>>(),
        ["Inline", "Block", "Tail"]
    );
}

#[test]
fn empty_forced_lines_in_rtl_text_do_not_panic_or_lose_line_height() {
    let html = format!("<style>{PROFILE}</style><p dir='rtl'>مرحبا<br><br>שלום<br>العربية</p>");
    let trace = render_html(&html, &local_options())
        .expect("empty forced RTL lines must have an empty visual run list")
        .layout
        .unwrap();
    assert_eq!(
        trace.pages[0]
            .iter()
            .map(|l| l.text.as_str())
            .collect::<Vec<_>>(),
        ["مرحبا", "", "שלום", "العربية"]
    );
    assert!(
        (trace.pages[0][2].baseline - trace.pages[0][0].baseline - 36.0).abs() < 0.001,
        "the blank line must remain in the paragraph's vertical flow"
    );
}

#[test]
fn narrow_no_break_spaces_use_half_the_faces_normal_space_width() {
    for (family, expected) in [
        ("DejaVu Sans", 1.907_226_6),
        ("DejaVu Serif", 1.907_226_6),
        ("DejaVu Sans Mono", 3.612_304_7),
    ] {
        let html =
            format!("<style>{PROFILE}body{{font-family:'{family}'}}</style><p>i\u{202f}i</p>");
        let trace = render_html(&html, &local_options())
            .unwrap()
            .layout
            .unwrap();
        let run = &trace.pages[0][0].runs[0];
        let glyph = run
            .glyphs()
            .iter()
            .find(|g| run.text[g.text_range.clone()].contains('\u{202f}'))
            .unwrap();
        assert!(
            (glyph.x_advance * run.font_size - expected).abs() < 0.0001,
            "{family} narrow no-break space advance {}, expected {expected}",
            glyph.x_advance * run.font_size
        );
    }
}

#[test]
fn word_spaces_interrupt_kerning_pairs_while_words_keep_theirs() {
    // Liberation Serif units: A and V advance 1479, the space 512; its kern table holds
    // A–space and space–A at -113 and A–V and V–A at -264, matching Times New Roman.
    const UNITS_PER_EM: f32 = 2048.0;
    for (text, expected) in [
        ("A A", 1479.0 + 512.0 + 1479.0),
        ("AVA", 1215.0 + 1215.0 + 1479.0),
    ] {
        let html = format!("<style>{PROFILE}body{{font-family:serif}}</style><p>{text}</p>");
        let trace = render_html(&html, &local_options())
            .unwrap()
            .layout
            .unwrap();
        let run = &trace.pages[0][0].runs[0];
        let units = run.width / run.font_size * UNITS_PER_EM;
        assert!(
            (units - expected).abs() < 0.01,
            "{text:?} in {} measures {units} units, expected {expected}: words shape independently, so pairs across a space never kern",
            trace.fonts[run.font_id.index()].family
        );
    }
}

#[test]
fn invisible_format_controls_do_not_emit_phantom_space_glyphs() {
    let html = format!("<style>{PROFILE}</style><p>ii\u{200b}ii</p>");
    let trace = render_html(&html, &local_options())
        .unwrap()
        .layout
        .unwrap();
    let line = &trace.pages[0][0];
    assert_eq!(
        line.runs.iter().map(|r| r.glyphs().len()).sum::<usize>(),
        4,
        "a hidden space glyph gives the zero-width control a nonzero nominal PDF bounding box"
    );
}

#[test]
fn letter_spacing_disables_optional_ligatures_and_counts_the_final_cluster() {
    let html = format!(
        "<style>{PROFILE}p{{letter-spacing:2pt;text-align:center}}</style><p>ffi</p><p>i</p>"
    );
    let trace = render_html(&html, &local_options())
        .unwrap()
        .layout
        .unwrap();
    let lines = &trace.pages[0];
    assert_eq!(
        lines[0].runs[0].glyphs().len(),
        3,
        "tracking must disable the optional ffi ligature"
    );
    assert!(
        (lines[1].width - 5.333_984_4).abs() < 0.0001,
        "the final cluster retains its tracking advance"
    );
    assert!(
        (lines[1].x - 177.333).abs() < 0.0001,
        "the tracked width is centred, including its final advance"
    );
}

#[test]
fn word_spacing_inherits_lengths_and_percentages_with_distinct_font_size_rules() {
    for (spacing, expected) in [("0.5em", 6.0), ("50%", 12.0)] {
        let html = format!(
            "<style>{PROFILE}p{{word-spacing:{spacing}}}</style><p>i i <span style='font-size:24pt'>i i</span></p>"
        );
        let trace = render_html(&html, &local_options())
            .unwrap()
            .layout
            .unwrap();
        let run = trace.pages[0][0]
            .runs
            .iter()
            .find(|run| run.font_size == 24.0)
            .unwrap();
        let normal_width = 2.0 * 569.0 / 2048.0 * 24.0 + 651.0 / 2048.0 * 24.0;
        assert!(
            (run.width - normal_width - expected).abs() < 0.0001,
            "{spacing}: child width {}, normal {normal_width}, expected extra {expected}",
            run.width
        );
    }
}

#[test]
fn justification_stretches_soft_lines_and_preserves_forced_line_endings() {
    let html = format!(
        "<style>{PROFILE}p{{width:100pt;text-align:justify}}</style><p>One two three four five six seven eight nine ten</p><p>One two<br>three four five six seven eight nine ten</p>"
    );
    let trace = render_html(&html, &local_options())
        .unwrap()
        .layout
        .unwrap();
    let lines = &trace.pages[0];
    assert_eq!(lines[0].text, "One two three");
    assert!(
        (lines[0].width - 100.0).abs() < 0.0001,
        "soft lines consume the available inline width"
    );
    assert_eq!(lines[3].text, "ten");
    assert!(
        lines[3].width < 100.0,
        "the default final line stays at its natural width"
    );
    assert_eq!(lines[4].text, "One two");
    assert!(
        lines[4].width < 100.0,
        "an explicit br ends an unexpanded line"
    );
}

#[test]
fn justified_line_breaks_use_compressible_space_width_including_css_spacing() {
    for (word_spacing, tracking, below, above, expected_words) in [
        (0.0, 0.0, 22.86, 22.89, 4),
        (-0.5, 0.0, 21.61, 21.64, 4),
        (4.0, 0.0, 23.01, 23.04, 3),
        (0.0, 0.1, 23.51, 23.53, 4),
    ] {
        for (width, words) in [(below, expected_words - 1), (above, expected_words)] {
            let html = format!(
                "<style>{PROFILE}p{{width:{width}pt;text-align:justify;word-spacing:{word_spacing}pt;letter-spacing:{tracking}pt}}</style><p>i i i i i i i i i i</p>"
            );
            let trace = render_html(&html, &local_options())
                .unwrap()
                .layout
                .unwrap();
            let first = &trace.pages[0][0];
            assert_eq!(
                first.text.split(' ').count(),
                words,
                "width {width}, word spacing {word_spacing}, tracking {tracking}: first native line has {words} words; got {:?}",
                first.text
            );
            assert!(
                (first.width - width).abs() < 0.0001,
                "selected soft line must fill {width}pt, got {}",
                first.width
            );
        }
    }
}

#[test]
fn justified_overfull_final_and_forced_lines_compress_before_last_line_alignment() {
    for last in ["auto", "start", "end", "center", "justify"] {
        for ending in ["", "<br>i"] {
            let html = format!(
                "<style>{PROFILE}p{{width:23pt;text-align:justify;text-align-last:{last}}}</style><p>i i i i{ending}</p>"
            );
            let trace = render_html(&html, &local_options())
                .unwrap()
                .layout
                .unwrap();
            let first = &trace.pages[0][0];
            assert_eq!(first.text, "i i i i", "last:{last}, ending:{ending}");
            assert!(
                (first.width - 23.0).abs() < 0.0001,
                "last:{last}, ending:{ending}: native compressed width 23pt, got {}",
                first.width
            );
            assert!(
                (first.x - 36.0).abs() < 0.0001,
                "a compressed final line fills its box before alignment: {}",
                first.x
            );
        }
    }
}

#[test]
fn justification_compresses_mixed_spaces_proportionally_and_expands_them_equally() {
    for (width, expected_spaces) in [
        (27.0, [3.375_016_5, 6.913_998, 3.375_016_5]),
        (40.0, [7.554_687_5, 11.554_687_5, 7.554_687_5]),
    ] {
        let html = format!(
            "<style>{PROFILE}p{{width:{width}pt;text-align:justify;text-align-last:justify}}</style><p>i <span style='word-spacing:4pt'>i i</span> i</p>"
        );
        let trace = render_html(&html, &local_options())
            .unwrap()
            .layout
            .unwrap();
        let first = &trace.pages[0][0];
        assert_eq!(first.text, "i i i i", "width:{width}");
        let spaces: Vec<_> = first
            .runs
            .iter()
            .flat_map(|run| {
                run.glyphs()
                    .iter()
                    .filter(|g| &run.text[g.text_range.clone()] == " ")
                    .map(|g| g.x_advance * run.font_size + g.x_spacing)
            })
            .collect();
        assert_eq!(spaces.len(), expected_spaces.len());
        for (actual, expected) in spaces.iter().zip(expected_spaces) {
            assert!(
                (actual - expected).abs() < 0.001,
                "width:{width}, spaces:{spaces:?}, expected:{expected_spaces:?}"
            );
        }
    }
}

#[test]
fn unwrappable_justified_overflow_uses_the_full_space_compression_limit() {
    for width in [10.0, 20.0, 21.0, 22.0] {
        let html = format!(
            "<style>{PROFILE}p{{width:{width}pt;text-align:justify;white-space:nowrap}}</style><p>i i i i</p>"
        );
        let trace = render_html(&html, &local_options())
            .unwrap()
            .layout
            .unwrap();
        let line = &trace.pages[0][0];
        const EXPECTED_COMPRESSED_WIDTH_PT: f32 = 20.964_844;
        assert!(
            (line.width - EXPECTED_COMPRESSED_WIDTH_PT).abs() < 0.0001,
            "nowrap width:{width}: native compression limit {EXPECTED_COMPRESSED_WIDTH_PT}, got {}",
            line.width
        );
    }
}

#[test]
fn preserved_trailing_spaces_hang_only_with_content_from_the_same_text_node() {
    for align in ["start", "justify"] {
        for white_space in ["normal", "pre-wrap"] {
            for separate in [false, true] {
                let content = if separate {
                    "A B<span style='white-space:pre-wrap'>          </span>"
                } else {
                    "<span style='white-space:pre-wrap'>A B          </span>"
                };
                let html = format!(
                    "<style>{PROFILE}p{{width:30pt;text-align:{align};white-space:{white_space}}}</style><p>{content}</p>"
                );
                let trace = render_html(&html, &local_options())
                    .unwrap()
                    .layout
                    .unwrap();
                let lines = &trace.pages[0];
                let wraps_trailing_spaces = align == "justify" && white_space == "normal";
                assert_eq!(
                    lines.len(),
                    if separate || wraps_trailing_spaces {
                        2
                    } else {
                        1
                    },
                    "{align}, {white_space}, separate:{separate}: {:?}",
                    lines.iter().map(|line| &line.text).collect::<Vec<_>>()
                );
                assert_eq!(lines[0].text.trim_end(), if separate { "A" } else { "A B" });
            }
        }
    }
}

#[test]
fn optional_ligatures_follow_the_measured_tracking_threshold_at_each_font_size() {
    for (size, spacing, expected_glyphs) in [
        (10.0, 0.25, 1),
        (10.0, 0.251, 3),
        (12.0, -0.3, 1),
        (12.0, -0.301, 3),
        (24.0, 0.6, 1),
        (24.0, 0.601, 3),
    ] {
        let html = format!(
            "<style>{PROFILE}p{{font-size:{size}pt;letter-spacing:{spacing}pt}}</style><p>ffi</p>"
        );
        let trace = render_html(&html, &local_options())
            .unwrap()
            .layout
            .unwrap();
        assert_eq!(
            trace.pages[0][0].runs[0].glyphs().len(),
            expected_glyphs,
            "{size}pt font, {spacing}pt tracking: native ligature boundary is ±0.025em"
        );
    }
}

#[test]
fn unexpandable_soft_lines_use_the_last_line_alignment_fallback() {
    let html = format!(
        "<style>{PROFILE}p{{width:70pt;text-align:justify;text-align-last:end}}</style><p>One looooooong</p>"
    );
    let trace = render_html(&html, &local_options())
        .unwrap()
        .layout
        .unwrap();
    let first = &trace.pages[0][0];
    assert_eq!(first.text, "One");
    assert!(
        (first.x - 81.5664).abs() < 0.0001,
        "a soft line with no expansion opportunity uses the requested end alignment; x={}",
        first.x
    );
}

#[test]
fn overfull_lines_fall_back_to_directional_start_for_every_alignment() {
    for direction in ["ltr", "rtl"] {
        for align in ["start", "end", "left", "right", "center", "justify"] {
            for property in ["text-align", "text-align-last"] {
                let html = format!(
                    "<style>{PROFILE}p{{width:10pt;direction:{direction};{property}:{align}}}</style><p>Longword</p>"
                );
                let trace = render_html(&html, &local_options())
                    .unwrap()
                    .layout
                    .unwrap();
                let line = &trace.pages[0][0];
                let expected = if direction == "rtl" {
                    36.0 + 10.0 - line.width
                } else {
                    36.0
                };
                assert!(
                    (line.x - expected).abs() < 0.0001,
                    "{direction} {property}:{align}: overfull x {}, native directional start {expected}",
                    line.x
                );
            }
        }
    }
}

#[test]
fn overflowing_text_spacing_returns_a_layout_error_before_pdf_painting() {
    for (property, value) in [("letter-spacing", "3e38pt"), ("word-spacing", "-3e38pt")] {
        let html = format!(
            "<style>{PROFILE}p{{{property}:{value};text-align:center}}</style><p>i i i i</p>"
        );
        let Err(error) = render_html(&html, &local_options()) else {
            panic!("{property} accumulated beyond finite text geometry")
        };
        assert!(
            matches!(error, crate::Error::Layout(ref message) if message.contains("non-finite text geometry")),
            "{property} must report the text measurement failure: {error}"
        );
    }
    let html = format!(
        "<style>{PROFILE}p{{font-size:1e-38pt;text-align:justify;text-align-last:justify}}</style><p>i i i</p>"
    );
    let result = render_html(&html, &local_options());
    assert!(
        matches!(result, Err(crate::Error::Layout(ref message)) if message.contains("non-finite text geometry")),
        "justification must reject spacing that cannot be represented in glyph em units"
    );
}

#[test]
fn explicit_break_styles_contribute_metrics_without_painting_a_glyph() {
    let html =
        format!("<style>{PROFILE}</style><p>A<br style='font-size:60pt;line-height:1'>B</p>");
    let trace = render_html(&html, &local_options())
        .unwrap()
        .layout
        .unwrap();
    const EXPECTED_BASELINES_PT: [f32; 2] = [86.7715, 109.1543];
    for (line, expected) in trace.pages[0].iter().zip(EXPECTED_BASELINES_PT) {
        assert!(
            (line.baseline - expected).abs() < 0.001,
            "styled break must enlarge the preceding line box: {} baseline {}, expected {expected}",
            line.text,
            line.baseline
        );
        assert!(
            line.runs.iter().all(|run| !run.text.contains('\n')),
            "line break controls must not become painted glyphs"
        );
    }
}

#[test]
fn outside_list_markers_follow_descendant_baselines_and_stay_outside_padding() {
    let html = format!(
        "<style>{PROFILE}ul{{margin:0}}p{{margin:0}}</style><ul><li>A</li><li style='padding:6pt;border:2pt solid navy'>B</li><li><p style='padding-top:12pt'>C</p></li><li></li><li>D</li></ul>"
    );
    let trace = render_html(&html, &local_options())
        .expect("outside markers must participate in list formatting")
        .layout
        .unwrap();
    let lines = &trace.pages[0];
    assert_eq!(
        lines.len(),
        10,
        "five items each have one marker and one line, including the empty item"
    );
    const MARKER_AND_CONTENT: usize = 2;
    for pair in lines.as_chunks::<MARKER_AND_CONTENT>().0 {
        assert_eq!(pair[0].text, "• ");
        assert_eq!(
            pair[0].baseline, pair[1].baseline,
            "marker and first descendant share their baseline"
        );
        assert!(
            (pair[0].x - (HTML_LIST_CONTENT_X_PT - BULLET_AND_SPACE_ADVANCE_PT)).abs() < 0.001,
            "marker stays outside the border edge even when content is padded: {}",
            pair[0].x
        );
    }
    assert_eq!(
        lines[3].x,
        HTML_LIST_CONTENT_X_PT + 6.0 + 2.0,
        "padding and border affect content independently of marker placement"
    );
    assert_eq!(
        lines[7].text, "",
        "an empty list item still contributes its line box"
    );
}

#[test]
fn list_ordinals_skip_hidden_and_block_items_and_respect_value_and_nested_scopes() {
    let html = format!(
        "<style>{PROFILE}ol{{margin:0}}</style><ol start='9'><li>A</li><li style='display:none'>Hidden</li><li style='display:block'>Block</li><li value='26' style='list-style-type:lower-alpha'>Z<ol><li>Nested</li></ol></li><li style='list-style-type:lower-alpha'>AA</li></ol>"
    );
    let trace = render_html(&html, &local_options())
        .expect("HTML ordinal attributes must resolve before measurement")
        .layout
        .unwrap();
    let markers: Vec<_> = trace.pages[0]
        .iter()
        .filter(|line| line.text.ends_with(". "))
        .map(|line| line.text.as_str())
        .collect();
    assert_eq!(
        markers,
        ["9. ", "z. ", "1. ", "aa. "],
        "nested lists own counters; hidden and ordinary block boxes do not increment list ordinals"
    );
}

#[test]
fn inside_list_markers_share_the_first_line_and_wrapping_width() {
    let html = format!(
        "<style>{PROFILE}ul{{margin:0}}li{{list-style-position:inside;width:90pt}}</style><ul><li>One two three four five six seven eight</li><li><p style='margin:0'>Block</p></li></ul>"
    );
    let trace = render_html(&html, &local_options())
        .expect("inside markers must be real inline content")
        .layout
        .unwrap();
    let lines = &trace.pages[0];
    assert_eq!(
        lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>(),
        [
            "• One two",
            "three four five",
            "six seven",
            "eight",
            "•",
            "Block"
        ]
    );
    assert!(
        lines.iter().all(|line| line.x == HTML_LIST_CONTENT_X_PT),
        "continuation lines and a first block child start at the list content edge"
    );
}

#[test]
fn list_counter_scopes_follow_list_elements_across_formatting_boundaries() {
    let html = format!(
        "<style>{PROFILE}ol{{margin:0}}</style><ol start='5'><li>A</li><div><li>B</li></div><li>C</li></ol><ol start='5' style='display:inline;padding:0'><li>D</li><li>E</li></ol>"
    );
    let trace = render_html(&html, &local_options())
        .unwrap()
        .layout
        .unwrap();
    let markers: Vec<_> = trace
        .pages
        .iter()
        .flatten()
        .filter(|line| line.text.ends_with(". "))
        .map(|line| line.text.as_str())
        .collect();
    assert_eq!(
        markers,
        ["5. ", "6. ", "7. ", "5. ", "6. "],
        "a block wrapper cannot restart a list counter, and an inline list still establishes its own scope"
    );
}

#[test]
fn a_list_container_with_its_own_marker_uses_its_own_counter_scope() {
    let html = format!(
        "<style>{PROFILE}ol{{margin:0}}</style><ol style='display:list-item;list-style-type:decimal'><li>A</li><li>B</li></ol><div style='display:list-item;list-style-type:decimal'>C</div><ol start='5' style='display:list-item;list-style-type:decimal'><li>D</li></ol>"
    );
    let trace = render_html(&html, &local_options())
        .unwrap()
        .layout
        .unwrap();
    let markers: Vec<_> = trace
        .pages
        .iter()
        .flatten()
        .filter(|line| line.text.ends_with(". "))
        .map(|line| line.text.as_str())
        .collect();
    assert_eq!(
        markers,
        ["1. ", "2. ", "3. ", "4. ", "5. ", "6. "],
        "list-item containers increment after their reset; following siblings inherit their counter instance"
    );
}

#[test]
fn root_blockification_preserves_a_requested_list_item_marker() {
    let html = format!(
        "<style>{PROFILE}html{{display:list-item;list-style-type:decimal;font-family:Spike}}</style><p>Root</p>"
    );
    let trace = render_html(&html, &local_options())
        .unwrap()
        .layout
        .unwrap();
    assert_eq!(
        trace.pages[0][0].text, "1. ",
        "root blockification must retain list-item marker generation"
    );
    assert_eq!(trace.pages[0][0].baseline, trace.pages[0][1].baseline);
    assert!(trace.pages[0][0].x < 36.0);
}

#[test]
fn implicit_list_counters_cover_siblings_and_descendants_without_leaking_to_outer_siblings() {
    let html = format!(
        "<style>{PROFILE}.item{{display:list-item;list-style-type:decimal}}</style><section><div class='item'>A<div class='item'>B</div></div><div class='item'>C</div></section><div class='item'>D</div>"
    );
    let trace = render_html(&html, &local_options())
        .unwrap()
        .layout
        .unwrap();
    let markers: Vec<_> = trace
        .pages
        .iter()
        .flatten()
        .filter(|line| line.text.ends_with(". "))
        .map(|line| line.text.as_str())
        .collect();
    assert_eq!(
        markers,
        ["1. ", "2. ", "3. ", "1. "],
        "an implicit counter belongs to the parent scope, including following siblings and their descendants"
    );
}

#[test]
fn a_marker_only_item_preserves_a_following_forced_page_break() {
    let html = format!(
        "<style>{PROFILE}ul{{margin:0}}</style><ul><li><div></div></li><li style='break-before:page'>Next</li></ul>"
    );
    let trace = render_html(&html, &local_options())
        .unwrap()
        .layout
        .unwrap();
    assert_eq!(
        trace.pages.len(),
        2,
        "a painted marker is page content even when its child block has zero height"
    );
    assert_eq!(
        trace.pages[0]
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>(),
        ["• "]
    );
    assert_eq!(
        trace.pages[1]
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>(),
        ["• ", "Next"]
    );
}

#[test]
fn list_shorthand_resets_inherited_values_and_respects_important_longhands() {
    let html = format!(
        "<style>{PROFILE}ul{{margin:0;list-style:upper-roman inside}}li{{list-style-position:outside!important;list-style:lower-alpha inside}}li.reset{{list-style:none}}</style><ul><li>A</li><li class='reset'>B</li><li>C</li></ul>"
    );
    let trace = render_html(&html, &local_options())
        .expect("list shorthand must expand before cascading individual properties")
        .layout
        .unwrap();
    let lines = &trace.pages[0];
    assert_eq!(
        lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>(),
        ["a. ", "A", "B", "c. ", "C"],
        "none removes a marker but still increments the ordinal"
    );
    assert!(
        lines[0].x < HTML_LIST_CONTENT_X_PT,
        "important outside position survives an ordinary shorthand"
    );
    assert_eq!(lines[1].x, HTML_LIST_CONTENT_X_PT);
}

#[test]
fn list_numbering_handles_alphabet_rollover_roman_limits_and_wide_ordinals() {
    let cases = [
        ("lower-alpha", 26, "z. "),
        ("lower-alpha", 27, "aa. "),
        ("lower-greek", 24, "ω. "),
        ("lower-greek", 25, "αα. "),
        ("upper-roman", 3999, "MMMCMXCIX. "),
        ("upper-roman", 4000, "4000. "),
        ("lower-roman", 0, "0. "),
        ("decimal-leading-zero", 1, "01. "),
        ("decimal-leading-zero", -2, "-2. "),
    ];
    let content: String = cases
        .iter()
        .map(|(kind, ordinal, _)| {
            format!("<ol start='{ordinal}' style='margin:0;list-style-type:{kind}'><li>X</li></ol>")
        })
        .collect();
    let trace = render_html(
        &format!("<style>{PROFILE}</style>{content}"),
        &local_options(),
    )
    .unwrap()
    .layout
    .unwrap();
    let markers: Vec<_> = trace
        .pages
        .iter()
        .flatten()
        .filter(|line| line.text.ends_with(". "))
        .map(|line| line.text.as_str())
        .collect();
    assert_eq!(
        markers,
        cases
            .iter()
            .map(|(_, _, expected)| *expected)
            .collect::<Vec<_>>(),
        "numbering formats must cover their range boundaries and decimal fallback"
    );
    render_html(
        &format!(
            "<style>{PROFILE}</style><ol start='{}'><li>Minimum</li></ol>",
            i64::MIN
        ),
        &local_options(),
    )
    .expect("the first ordinal must support the complete declared signed range");
    let wide = render_html(
        &format!(
            "<style>{PROFILE}</style><ol start='{}'><li>A</li><li>B</li></ol>",
            i64::MAX
        ),
        &local_options(),
    );
    let wide = wide
        .expect("counter values beyond i64 must not wrap or abort rendering")
        .layout
        .unwrap();
    let markers: Vec<_> = wide
        .pages
        .iter()
        .flatten()
        .filter(|line| line.text.ends_with(". "))
        .map(|line| line.text.clone())
        .collect();
    assert_eq!(
        markers,
        [
            format!("{}. ", i64::MAX),
            format!("{}. ", i128::from(i64::MAX) + 1)
        ]
    );
}

#[test]
fn preformatted_content_preserves_spaces_tabs_and_empty_lines() {
    let html = format!(
        "<style>{PROFILE}pre{{font-family:Spike;margin:0;tab-size:4}}</style><pre>\n  A\tB\n\nC  </pre><p>Next</p>"
    );
    let trace = render_html(&html, &local_options())
        .unwrap()
        .layout
        .unwrap();
    let lines = &trace.pages[0];
    assert_eq!(
        lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>(),
        ["  A    B", "", "C  ", "Next"],
        "HTML removes the initial pre newline; CSS preserves indentation, expanded tabs, blank lines and final spaces"
    );
    assert!(
        (lines[3].baseline - lines[0].baseline - 54.0).abs() < 0.0001,
        "the empty preserved line consumes its real line height"
    );
}

#[test]
fn identical_inline_styles_preserve_source_boundaries_for_shaping() {
    let html = format!(
        "<style>{PROFILE}</style><p>of<span>f</span>ice</p><p dir='rtl' style='font-size:24pt'>ل<span>ا</span></p>"
    );
    let trace = render_html(&html, &local_options())
        .unwrap()
        .layout
        .unwrap();
    assert_eq!(
        trace.pages[0][0]
            .runs
            .iter()
            .map(|run| run.text.as_str())
            .collect::<Vec<_>>(),
        ["of", "f", "ice"],
        "matching styles must not erase independent source-node shaping boundaries"
    );
    assert!(
        (trace.pages[0][1].width - 24.105_469).abs() < 0.0001,
        "isolated lam/alef forms are used across this source boundary, not the paragraph's lam-alef ligature"
    );
}

#[test]
fn soft_line_breaks_end_arabic_joining_context() {
    let html = format!(
        "<style>{PROFILE}p{{width:20pt;font-size:24pt;line-height:36pt;margin:0}}</style><p dir='rtl'>ب\u{200b}ل</p><p dir='rtl'>ب</p><p dir='rtl'>ل</p>"
    );
    let trace = render_html(&html, &local_options())
        .expect("Arabic joining must stop at a chosen soft line break")
        .layout
        .unwrap();
    let lines = &trace.pages[0];
    assert_eq!(lines.len(), 4, "the zero-width space must permit two lines");
    for (wrapped, isolated) in lines[..2].iter().zip(&lines[2..]) {
        assert_eq!(
            wrapped.width, isolated.width,
            "a glyph at a soft line boundary must have the isolated form's advance: {}",
            wrapped.text
        );
        assert_eq!(
            wrapped.runs[0].glyphs()[0].id,
            isolated.runs[0].glyphs()[0].id,
            "joining context must not cross into the adjacent line"
        );
        assert_eq!(
            wrapped.x, isolated.x,
            "RTL placement uses the isolated width"
        );
    }
}

#[test]
fn excessive_preserved_tab_expansion_is_rejected_before_allocating_the_text() {
    let html =
        format!("<style>{PROFILE}p{{white-space:pre;tab-size:1000000000}}</style><p>A\tB</p>");
    let result = render_html(&html, &local_options());
    assert!(
        matches!(result, Err(crate::Error::Document(ref message)) if message.contains("formatted inline text exceeds")),
        "a small input must not allocate a billion glyph spaces before reporting the expansion limit"
    );
}

#[test]
fn nowrap_inline_spans_keep_internal_spaces_without_blocking_boundary_breaks() {
    let html = format!(
        "<style>{PROFILE}p{{width:40pt}}</style><p>A <span style='white-space:nowrap'>B C D </span>E F G H</p>"
    );
    let trace = render_html(&html, &local_options())
        .unwrap()
        .layout
        .unwrap();
    assert_eq!(
        trace.pages[0]
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>(),
        ["A", "B C D", "E F G", "H"],
        "nowrap suppresses internal opportunities while the outer prose still wraps"
    );
}

#[test]
fn prewrap_trailing_spaces_hang_for_wrapping_and_still_affect_alignment() {
    let html = format!(
        "<style>{PROFILE}p{{white-space:pre-wrap;width:70pt;text-align:center}}</style><p>  One   two    three  </p>"
    );
    let trace = render_html(&html, &local_options())
        .unwrap()
        .layout
        .unwrap();
    let lines = &trace.pages[0];
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0].text, "  One   two    ");
    assert!(
        lines[0].width > 70.0,
        "preserved spaces may hang beyond the available line width"
    );
    assert_eq!(
        lines[0].x, 36.0,
        "the full preserved width makes the line overfull for alignment"
    );
    assert_eq!(lines[1].text, "three  ");
}

#[test]
fn root_negative_margin_moves_the_root_content_box() {
    let html = format!("<style>{PROFILE} html{{margin-top:-10pt}}</style><p>A</p>");
    let trace = render_html(&html, &local_options())
        .unwrap()
        .layout
        .unwrap();
    const EXPECTED_BASELINE_PT: f32 = 39.1543;
    assert!(
        (trace.pages[0][0].baseline - EXPECTED_BASELINE_PT).abs() < 0.001,
        "root margins must reach pagination: {}",
        trace.pages[0][0].baseline
    );
}

#[test]
fn block_padding_stays_monolithic_at_page_edges() {
    for property in ["padding-top", "padding-bottom"] {
        let html = format!(
            "<style>{PROFILE}@page{{size:360pt 200pt;margin:20pt}}p{{margin:0}}</style><div style='{property}:150pt'><p>A</p></div><p>B</p>"
        );
        let trace = render_html(&html, &local_options())
            .unwrap()
            .layout
            .unwrap();
        assert_eq!(
            trace.pages.len(),
            2,
            "{property} must produce the same two-page flow"
        );
        assert!(
            !trace.pages[0].is_empty(),
            "{property} must not create a blank first page instead of keeping the first line with its padding"
        );
        assert_eq!(
            trace.pages[0][0].text, "A",
            "first content after large top padding must stay attached to that padding"
        );
        const NEXT_PAGE_BASELINE_PT: f32 = 33.1543;
        assert!(
            (trace.pages[1][0].baseline - NEXT_PAGE_BASELINE_PT).abs() < 0.001,
            "{property} must not spill padding onto the next page: {}",
            trace.pages[1][0].baseline
        );
    }
}

#[test]
fn heading_defaults_use_html_relative_sizes_weight_and_margins() {
    let html = format!(
        "<style>{PROFILE}@page{{size:360pt 200pt;margin:20pt}}p{{margin:0}}</style><h1>A</h1><h2>B</h2><p>C</p>"
    );
    let trace = render_html(&html, &local_options())
        .unwrap()
        .layout
        .unwrap();
    let lines = &trace.pages[0];
    assert_eq!(
        lines.iter().map(|l| l.font_size).collect::<Vec<_>>(),
        [24.0, 18.0, 12.0],
        "heading defaults must be computed before author overrides"
    );
    assert!(
        lines[0].runs[0].synthesis.bold && lines[1].runs[0].synthesis.bold,
        "the fixture's regular-only face needs synthetic bold for heading defaults"
    );
    // h1's .67em margin is 16.08pt, winning the collapsed gap before h2.
    // h2's .83em bottom margin is 14.94pt; font metrics and the 18pt leading are unchanged.
    const HTML_BASELINES_PT: [f32; 3] = [37.3086, 69.3114, 100.1743];
    for (line, baseline) in lines.iter().zip(HTML_BASELINES_PT) {
        assert!(
            (line.baseline - baseline).abs() < 0.001,
            "{} heading margin baseline {}, expected {baseline}",
            line.text,
            line.baseline
        );
    }
}

#[test]
fn headings_stay_with_the_start_of_the_following_block() {
    let leads = "<p>Lead</p>".repeat(6);
    let html = format!(
        "<style>{PROFILE}@page{{size:360pt 200pt;margin:20pt}}p{{margin:0}}</style>{leads}<h1>Heading</h1><p>Next body line</p>"
    );
    let trace = render_html(&html, &local_options())
        .unwrap()
        .layout
        .unwrap();
    assert!(
        trace.pages[0].iter().all(|l| l.text == "Lead"),
        "a heading must move with its following content instead of being stranded at the bottom"
    );
    assert_eq!(
        trace.pages[1]
            .iter()
            .map(|l| l.text.as_str())
            .collect::<Vec<_>>(),
        ["Heading", "Next body line"]
    );
}

#[test]
fn avoided_sibling_breaks_can_move_the_tail_of_a_long_container() {
    let children = (0..7).map(|i| format!("<p>A{i}</p>")).collect::<String>();
    let html = format!(
        "<style>{PROFILE}@page{{size:360pt 200pt;margin:20pt}}p{{margin:0}}</style><p>Lead</p><div style='break-after:avoid'>{children}</div><p>B</p>"
    );
    let trace = render_html(&html, &local_options())
        .expect("break avoidance applies at a container's final fragment, not just its start")
        .layout
        .unwrap();
    assert_eq!(trace.pages[0].last().unwrap().text, "A5");
    assert_eq!(
        trace.pages[1]
            .iter()
            .map(|l| l.text.as_str())
            .collect::<Vec<_>>(),
        ["A6", "B"]
    );
}

#[test]
fn break_before_avoid_and_forced_break_after_apply_between_boxes() {
    let leads = "<p>Lead</p>".repeat(7);
    let html = format!(
        "<style>{PROFILE}@page{{size:360pt 200pt;margin:20pt}}p{{margin:0}}</style>{leads}<p>A</p><p style='break-before:avoid;break-after:page'>B</p><p style='break-after:page'>C</p>"
    );
    let trace = render_html(&html, &local_options())
        .unwrap()
        .layout
        .unwrap();
    assert_eq!(
        trace.pages.len(),
        3,
        "a trailing break-after must not add an extra empty page"
    );
    assert!(trace.pages[0].iter().all(|l| l.text == "Lead"));
    assert_eq!(
        trace.pages[1]
            .iter()
            .map(|l| l.text.as_str())
            .collect::<Vec<_>>(),
        ["A", "B"]
    );
    assert_eq!(trace.pages[2][0].text, "C");
}

#[test]
fn padding_beyond_the_page_does_not_create_empty_automatic_pages() {
    let html = format!(
        "<style>{PROFILE}@page{{size:360pt 200pt;margin:20pt}}p{{margin:0}}</style><div style='padding-top:350pt'><p>A</p></div><p>B</p>"
    );
    let trace = render_html(&html, &local_options())
        .unwrap()
        .layout
        .unwrap();
    assert_eq!(
        trace.pages.len(),
        1,
        "fully out-of-page content must not create an empty automatic page; only B's page is output"
    );
    assert_eq!(
        trace.pages[0]
            .iter()
            .map(|l| l.text.as_str())
            .collect::<Vec<_>>(),
        ["B"]
    );
}

#[test]
fn an_inkless_box_inside_the_page_keeps_its_page_when_the_next_box_breaks() {
    let html = format!(
        "<style>{PROFILE}@page{{size:360pt 200pt;margin:20pt}}div{{orphans:2}}</style><div style='height:140pt'></div><div>A<br>B</div>"
    );
    let trace = render_html(&html, &local_options())
        .unwrap()
        .layout
        .unwrap();
    let texts: Vec<Vec<&str>> = trace
        .pages
        .iter()
        .map(|page| page.iter().map(|l| l.text.as_str()).collect())
        .collect();
    assert_eq!(
        texts,
        [vec![], vec!["A", "B"]],
        "the empty 140pt box fills page 1 and orphans move both lines to page 2"
    );
}

#[test]
fn legacy_page_break_aliases_share_cascade_priority_and_source_order() {
    for (declarations, separate) in [
        ("break-before:page!important;page-break-before:auto", true),
        ("page-break-before:auto!important;break-before:page", false),
        ("break-before:auto;page-break-before:always", true),
        ("page-break-before:always;break-before:auto", false),
    ] {
        for property in ["before", "after"] {
            let declarations = declarations.replace("before", property);
            let body = if property == "before" {
                format!("<p>Lead</p><p style='{declarations}'>Tail</p>")
            } else {
                format!("<p style='{declarations}'>Lead</p><p>Tail</p>")
            };
            let html = format!("<style>{PROFILE}</style>{body}");
            let trace = render_html(&html, &local_options())
                .unwrap()
                .layout
                .unwrap();
            assert_eq!(
                trace.pages.len(),
                if separate { 2 } else { 1 },
                "legacy and modern break declarations must have one cascade winner: {declarations}"
            );
        }
    }
    let html = format!(
        "<style>{PROFILE}@page{{size:360pt 200pt;margin:20pt}}p{{margin:0}}</style>{}<div style='page-break-inside:avoid'><p>A</p><p>B</p></div>",
        "<p>Lead</p>".repeat(7)
    );
    let trace = render_html(&html, &local_options())
        .expect("legacy page-break-inside must use the same fragmentation rule as break-inside")
        .layout
        .unwrap();
    assert_eq!(
        trace.pages[1]
            .iter()
            .map(|l| l.text.as_str())
            .collect::<Vec<_>>(),
        ["A", "B"]
    );
}

#[test]
fn block_display_line_breaks_create_their_own_line_boxes() {
    let html = format!(
        "<style>{PROFILE}</style>A<br style='display:block'>B<br style='display:block'><br style='display:block'>C"
    );
    let trace = render_html(&html, &local_options())
        .unwrap()
        .layout
        .unwrap();
    let visible = trace.pages[0]
        .iter()
        .filter(|l| !l.text.is_empty())
        .collect::<Vec<_>>();
    assert_eq!(
        visible.iter().map(|l| l.text.as_str()).collect::<Vec<_>>(),
        ["A", "B", "C"]
    );
    const EXPECTED_BASELINE_GAPS_PT: [f32; 2] = [36.0, 54.0];
    for (pair, expected) in visible.windows(2).zip(EXPECTED_BASELINE_GAPS_PT) {
        assert!(
            (pair[1].baseline - pair[0].baseline - expected).abs() < 0.001,
            "a block BR occupies a line independently of preceding anonymous text: {} → {}",
            pair[0].baseline,
            pair[1].baseline
        );
    }
}

#[test]
fn avoidance_respects_nested_forced_boundaries_and_traverses_empty_fragments() {
    for (following, expected_pages) in [
        (
            "<div><div style='break-before:page'>N</div></div>",
            vec![vec!["P", "P", "P", "P", "P", "H"], vec!["N"]],
        ),
        (
            "<div style='padding-top:10pt'><div style='break-before:page'>N</div></div>",
            vec![vec!["P", "P", "P", "P", "P", "H"], vec!["N"]],
        ),
        (
            "<div><div></div><div>N</div></div>",
            vec![vec!["P", "P", "P", "P", "P"], vec!["H", "N"]],
        ),
        (
            "<div><div></div><div style='break-before:page'>N</div></div>",
            vec![vec!["P", "P", "P", "P", "P"], vec!["H"], vec!["N"]],
        ),
    ] {
        let html = format!(
            "<style>{PROFILE}@page{{size:360pt 180pt;margin:36pt}}p{{margin:0}}</style><p>P<br>P<br>P<br>P<br>P</p><div style='break-after:avoid'>H</div>{following}"
        );
        let trace = render_html(&html, &local_options())
            .unwrap()
            .layout
            .unwrap();
        let pages = trace
            .pages
            .iter()
            .map(|page| page.iter().map(|l| l.text.as_str()).collect::<Vec<_>>())
            .collect::<Vec<_>>();
        assert_eq!(
            pages, expected_pages,
            "avoid lookahead must follow the same nested boundaries: {following}"
        );
        if following.contains("padding-top") {
            const EXPECTED_PADDED_BASELINE_PT: f32 = 59.1543;
            assert!(
                (trace.pages[1][0].baseline - EXPECTED_PADDED_BASELINE_PT).abs() < 0.001,
                "a propagated forced break must happen before the wrapper's padding: {}",
                trace.pages[1][0].baseline
            );
        }
    }
}

#[test]
fn overflowing_percentage_margins_fail_before_pdf_serialization() {
    let html = format!(
        "<style>{PROFILE}</style><div style='width:3e38pt'><p style='margin-top:200%'>A</p></div>"
    );
    let result = render_html(&html, &local_options());
    assert!(
        matches!(&result, Err(crate::Error::Layout(message)) if message.contains("non-finite")),
        "percentage resolution must reject overflow as a layout error, before coordinates reach the writer: {:?}",
        result.err()
    );
}

#[test]
fn an_empty_trailing_box_cannot_create_an_automatic_blank_page() {
    let html = format!(
        "<style>{PROFILE}@page{{size:360pt 200pt;margin:20pt}}p{{margin:0}}</style><div style='padding-top:150pt'><p>A</p></div><div></div>"
    );
    let trace = render_html(&html, &local_options())
        .unwrap()
        .layout
        .unwrap();
    assert_eq!(
        trace.pages.len(),
        1,
        "zero-height boxes must not trigger automatic pagination after overflowing content"
    );
}

#[test]
fn missing_image_resources_cannot_vanish_when_display_is_block() {
    let result = render_html(
        "<img style='display:block' src='image.png'>",
        &RenderOptions::default(),
    );
    let output = result.unwrap();
    assert!(
        matches!(output.diagnostics.as_slice(), [crate::RenderDiagnostic::UnavailableImage(failure)]
        if matches!(&failure.error, crate::ImageError::Resource(ResourceError::Read { path, .. }) if path.ends_with("image.png"))),
        "missing images must retain the resource failure in diagnostics"
    );
}

#[cfg(not(feature = "http"))]
#[test]
fn without_the_http_feature_remote_images_are_an_unsupported_scheme() {
    let result = render_html(
        "<img style='display:block' src='https://example.com/image.png'>",
        &RenderOptions::default(),
    );
    let output = result.unwrap();
    assert!(
        matches!(output.diagnostics.as_slice(), [crate::RenderDiagnostic::UnavailableImage(failure)]
        if matches!(&failure.error, crate::ImageError::Resource(crate::ResourceError::UnsupportedScheme(scheme)) if scheme == "https")),
        "remote images must retain the unsupported-scheme diagnostic without network access"
    );
}

#[test]
fn renders_a_plain_html_string_without_filesystem_font_configuration() {
    let result = render_html(
        "<p>Hello from Rust.</p>",
        &RenderOptions {
            include_layout: true,
            ..Default::default()
        },
    )
    .expect("plain HTML must use an embedded default font");
    assert!(
        result.pdf.starts_with(b"%PDF-"),
        "public API must return a complete PDF"
    );
    assert_eq!(result.page_count, 1);
    assert_eq!(result.layout.unwrap().pages[0][0].text, "Hello from Rust.");
}

#[test]
fn inline_importance_and_unitless_leading_affect_real_measured_layout() {
    let html = format!(
        "<style>{PROFILE} #lead {{font-size:16pt!important}} body{{line-height:1.5}}</style><p id='lead' style='font-size:8pt'>Alpha beta gamma</p><p>Next paragraph</p>"
    );
    let result = render_html(&html, &local_options()).unwrap();
    let trace = result.layout.unwrap();
    assert_eq!(
        trace.pages[0][0].font_size, 16.0,
        "important author CSS must beat ordinary inline CSS"
    );
    const EXPECTED_BASELINE_DELTA_PT: f32 = 31.615_234;
    assert!(
        (trace.pages[0][1].baseline - trace.pages[0][0].baseline - EXPECTED_BASELINE_DELTA_PT)
            .abs()
            < 0.01,
        "mixed-size line boxes and collapsed margins must determine the next baseline: {:?}",
        trace.pages[0]
            .iter()
            .map(|l| l.baseline)
            .collect::<Vec<_>>()
    );
}

#[test]
fn linked_stylesheet_uses_its_own_base_for_fonts_in_memory() {
    let assets=Assets(HashMap::from([
        (PathBuf::from("/document/theme/style.css"),b"@font-face{font-family:Custom;src:url(font.ttf)} @page{size:200pt 300pt;margin:20pt} body{font-family:Custom;font-size:11pt} p{margin:0}".to_vec()),
        (PathBuf::from("/document/theme/font.ttf"),FONT.to_vec()),
    ]));
    let output = render_with_resolver(
        "<link rel='stylesheet' href='theme/style.css'><p>In memory</p>",
        &RenderOptions {
            base: "/document".into(),
            include_layout: true,
            ..Default::default()
        },
        &assets,
    )
    .unwrap();
    let trace = output.layout.unwrap();
    assert_eq!((trace.page_width, trace.page_height), (200.0, 300.0));
    assert_eq!(trace.pages[0][0].font_size, 11.0);
}

#[test]
fn empty_block_margins_collapse_through_without_creating_a_line() {
    let html = format!(
        "<style>{PROFILE} p{{margin:12pt 0}} .empty{{margin-top:-6pt;margin-bottom:18pt}}</style><p>A</p><p class='empty'></p><p>B</p>"
    );
    let trace = render_html(&html, &local_options())
        .unwrap()
        .layout
        .unwrap();
    assert_eq!(
        trace.pages[0].len(),
        2,
        "an empty paragraph must not synthesize content"
    );
    assert!(
        (trace.pages[0][1].baseline - trace.pages[0][0].baseline - 30.0).abs() < 0.001,
        "expected a 30pt baseline separation"
    );
}

#[test]
fn nested_inline_styles_wrap_and_expand_the_line_box() {
    let html = format!(
        "<style>{PROFILE} body{{line-height:1.5}} p{{width:150pt}}</style><p>Small <span style='font-size:24pt;color:navy'>large <span>words</span></span> finish.</p>"
    );
    let output =
        render_html(&html, &local_options()).expect("inline spans must participate in text layout");
    let trace = output.layout.unwrap();
    assert_eq!(
        trace.pages[0].len(),
        2,
        "the 24pt run must wrap at the paragraph's computed width"
    );
    assert!(
        trace.pages[0]
            .iter()
            .flat_map(|l| &l.runs)
            .any(|r| r.font_size == 24.0 && r.color.rgb == [0, 0, 128]),
        "shaping and painting must retain each span's font size and colour"
    );
    assert!(
        trace.pages[0][1].baseline - trace.pages[0][0].baseline >= 24.0,
        "large inline text must expand the line box instead of overlapping adjacent lines"
    );
}

#[test]
fn inline_direction_is_not_silently_flattened_into_paragraph_direction() {
    let output = render_html(
        "<p>Start <span dir='rtl'>123 456</span> end</p>",
        &RenderOptions {
            include_layout: true,
            ..Default::default()
        },
    )
    .expect("inline directional isolation must render");
    let trace = output.layout.unwrap();
    let line = &trace.pages[0][0];
    assert_eq!(
        line.text, "Start 123 456 end",
        "logical source must remain unchanged"
    );
    let visual: String = line
        .runs
        .iter()
        .flat_map(|run| {
            run.glyphs()
                .iter()
                .map(|glyph| &run.text[glyph.text_range.clone()])
        })
        .collect();
    assert_eq!(
        visual, "Start 456 123 end",
        "RTL isolation must reorder the two numeric runs without reversing either number"
    );
}

#[test]
fn emphasis_selects_real_bold_and_italic_faces() {
    let output = render_html(
        "<p>Wide words</p><p><strong>Wide words</strong></p><p><em>Wide words</em></p><p><strong><em>Wide words</em></strong></p>",
        &RenderOptions { include_layout: true, ..Default::default() },
    ).expect("semantic emphasis must select actual font faces");
    let trace = output.layout.unwrap();
    let lines = &trace.pages[0];
    assert_eq!(lines.len(), 4);
    assert!(
        lines[1].width > lines[0].width,
        "bold outlines must be measured using their own advances: regular {}, bold {}",
        lines[0].width,
        lines[1].width
    );
    for name in [
        "LiberationSerif-Bold",
        "LiberationSerif-Italic",
        "LiberationSerif-BoldItalic",
    ] {
        assert!(
            output.pdf.windows(name.len()).any(|w| w == name.as_bytes()),
            "the PDF must embed {name}, not paint every run with the regular font"
        );
    }
}

#[test]
fn monospace_uses_courier_new_compatible_advances_then_dejavu_sans_mono_coverage() {
    // Courier New and Liberation Mono advance every WGL4 character by 1229 of 2048 units.
    const COURIER_ADVANCE_EM: f32 = 1229.0 / 2048.0;
    let output = render_html(
        "<style>p{font-family:monospace;font-size:10pt}</style><p>AVATAR iii WWW 0123 <b>Bold</b> <i>Italic</i> \u{203d}</p>",
        &RenderOptions {
            include_layout: true,
            ..Default::default()
        },
    )
    .unwrap();
    let trace = output.layout.unwrap();
    let runs: Vec<_> = trace.pages[0][0]
        .runs
        .iter()
        .map(|run| {
            let face = &trace.fonts[run.font_id.index()];
            let (min, max) = face.weight.bounds(crate::FontWeight::NORMAL);
            assert_eq!(min, max, "the bundled monospace faces have fixed weights");
            (run.text.as_str(), face.family.as_str(), min.value())
        })
        .collect();
    assert_eq!(
        runs,
        [
            ("AVATAR iii WWW 0123 ", "Liberation Mono", 400.0),
            ("Bold", "Liberation Mono", 700.0),
            (" ", "Liberation Mono", 400.0),
            ("Italic", "Liberation Mono", 400.0),
            (" ", "Liberation Mono", 400.0),
            ("\u{203d}", "DejaVu Sans Mono", 400.0),
        ],
        "monospace expands to Liberation Mono's own bold and italic faces, and U+203D, which it lacks, comes from DejaVu Sans Mono"
    );
    for run in trace.pages[0][0]
        .runs
        .iter()
        .filter(|run| trace.fonts[run.font_id.index()].family == "Liberation Mono")
    {
        for glyph in run.glyphs() {
            assert!(
                (glyph.x_advance - COURIER_ADVANCE_EM).abs() < 1e-6,
                "{:?} in {:?} advances {} em, Courier New advances {COURIER_ADVANCE_EM} em",
                &run.text[glyph.text_range.clone()],
                run.text,
                glyph.x_advance
            );
        }
    }
    for name in [
        "LiberationMono",
        "LiberationMono-Bold",
        "LiberationMono-Italic",
    ] {
        assert!(
            output.pdf.windows(name.len()).any(|w| w == name.as_bytes()),
            "the PDF must embed {name}"
        );
    }
}

#[test]
fn family_lists_skip_missing_families_and_keep_graphemes_during_fallback() {
    let output = render_html(
        "<style>body{font-family:'Missing font', DejaVu Serif, sans-serif}</style><p>Latin العربية A͆</p>",
        &RenderOptions { include_layout: true, ..Default::default() },
    ).expect("CSS family lists must fall back to available faces with glyph coverage");
    let trace = output.layout.unwrap();
    assert_eq!(trace.pages[0][0].text, "Latin العربية A͆");
    let runs: Vec<_> = trace.pages[0][0]
        .runs
        .iter()
        .map(|r| {
            (
                r.text.as_str(),
                trace.fonts[r.font_id.index()].family.as_str(),
            )
        })
        .collect();
    for (text, family) in [
        ("Latin", "DejaVu Serif"),
        ("العربية", "DejaVu Sans"),
        ("A͆", "Liberation Sans"),
    ] {
        assert!(
            runs.iter()
                .any(|(run, face)| run.contains(text) && *face == family),
            "{text} must use {family}: DejaVu Serif lacks Arabic and the bridge mark, sans-serif tries Liberation Sans before DejaVu Sans; runs {runs:?}"
        );
    }
}

#[test]
fn glyphs_missing_from_an_author_family_use_the_next_listed_family_or_the_initial_serif_face() {
    let assets = Assets(HashMap::from([(
        PathBuf::from("/document/serif.ttf"),
        include_bytes!("../../crates/fonts/assets/fonts/DejaVuSerif.ttf").to_vec(),
    )]));
    // DejaVu Serif lacks U+0460, which Arial, Times New Roman and both Liberation faces cover.
    for (families, expected) in [
        ("Probe", "Liberation Serif"),
        ("Probe, sans-serif", "Liberation Sans"),
        ("Probe, 'DejaVu Sans'", "DejaVu Sans"),
    ] {
        let html = format!(
            "<style>@font-face{{font-family:Probe;src:url(serif.ttf)}}body{{font-family:{families}}}</style><p>Omega \u{460}</p>"
        );
        let trace = render_with_resolver(
            &html,
            &RenderOptions {
                base: "/document".into(),
                include_layout: true,
                ..Default::default()
            },
            &assets,
        )
        .unwrap()
        .layout
        .unwrap();
        let runs: Vec<_> = trace.pages[0][0]
            .runs
            .iter()
            .map(|r| {
                (
                    r.text.as_str(),
                    trace.fonts[r.font_id.index()].family.as_str(),
                )
            })
            .collect();
        assert_eq!(
            runs,
            [("Omega ", "Probe"), ("\u{460}", expected)],
            "font-family:{families}: a missing glyph comes from the next listed family, else from its initial serif family"
        );
    }
}

#[test]
fn multiple_author_faces_resolve_weight_and_style_against_each_stylesheet() {
    let assets = Assets(HashMap::from([
        (PathBuf::from("/document/fonts/regular.ttf"), FONT.to_vec()),
        (
            PathBuf::from("/document/fonts/bold.ttf"),
            include_bytes!("../../crates/fonts/assets/fonts/DejaVuSans-Bold.ttf").to_vec(),
        ),
    ]));
    let output = render_with_resolver(
        "<style>@font-face{font-family:Custom Face;src:url(fonts/regular.ttf) format('truetype')} @font-face{font-family:'Custom Face';font-weight:700;src:url(fonts/bold.ttf)} body{font-family:'Custom Face',sans-serif}</style><p>Wide words</p><p style='font-weight:bold'>Wide words</p>",
        &RenderOptions { base: "/document".into(), include_layout: true, ..Default::default() },
        &assets,
    ).expect("multiple @font-face rules must register distinct matching faces");
    let lines = &output.layout.unwrap().pages[0];
    assert!(
        lines[1].width > lines[0].width,
        "the declared bold font must control shaping, not just PDF font metadata"
    );
}

#[test]
fn missing_emphasis_faces_synthesize_visible_marks_without_changing_advances() {
    let render = |content: &str| {
        render_html(
            &format!("<style>{PROFILE}</style><p>{content}</p>"),
            &local_options(),
        )
        .unwrap()
    };
    let plain = render("Wide words");
    for content in [
        "<strong>Wide words</strong>",
        "<em>Wide words</em>",
        "<strong><em>Wide words</em></strong>",
    ] {
        let emphasized = render(content);
        assert!(
            plain.pdf != emphasized.pdf,
            "a family with only a regular face must still paint emphasis: {content}"
        );
        assert_eq!(
            plain.layout.as_ref().unwrap().pages[0][0].width,
            emphasized.layout.unwrap().pages[0][0].width,
            "synthesis must preserve the face's shaped advances"
        );
    }
}

#[test]
fn unused_font_sources_are_not_opened_and_failed_primary_fonts_fall_back() {
    use std::cell::RefCell;
    struct RecordingAssets(RefCell<Vec<String>>);
    impl ResourceResolver for RecordingAssets {
        fn resolve(&self, reference: &str, base: &ResourceBase) -> Result<Resource, ResourceError> {
            self.0.borrow_mut().push(reference.into());
            Err(ResourceError::Read {
                path: asset_path(base, reference),
                source: std::io::ErrorKind::NotFound.into(),
            })
        }
    }
    let assets = RecordingAssets(RefCell::new(Vec::new()));
    let options = RenderOptions {
        include_layout: true,
        ..Default::default()
    };
    let css = "@font-face{font-family:Unused;src:url(unused.ttf)} @font-face{font-family:Missing;src:url(missing.ttf)}";
    render_with_resolver(
        &format!("<style>{css}</style><p>Hello</p>"),
        &options,
        &assets,
    )
    .expect("unused font declarations must not block unrelated text");
    assert!(
        assets.0.borrow().is_empty(),
        "unused font sources must not be opened: {:?}",
        assets.0.borrow()
    );
    let output = render_with_resolver(
        &format!("<style>{css} body{{font-family:Missing,sans-serif}}</style><p>Hello</p>"),
        &options,
        &assets,
    )
    .expect("missing primary font must allow the next CSS family");
    assert_eq!(
        *assets.0.borrow(),
        ["missing.ttf"],
        "load a failed source only once and leave unrelated faces unopened"
    );
    let trace = output.layout.unwrap();
    let run = &trace.pages[0][0].runs[0];
    assert_eq!(trace.fonts[run.font_id.index()].family, "Liberation Sans");
}

#[test]
fn unused_fallback_sources_do_not_trigger_resource_policy_failures() {
    struct Denied;
    impl ResourceResolver for Denied {
        fn resolve(&self, reference: &str, base: &ResourceBase) -> Result<Resource, ResourceError> {
            Err(ResourceError::OutsideRoot(asset_path(base, reference)))
        }
    }
    let html = "<style>@font-face{font-family:Later;src:url(denied.ttf)} @font-face{font-family:'DejaVu Serif';src:url(denied-serif.ttf)}body{font-family:sans-serif,Later}</style><p>Only ASCII, covered by the first font.</p>";
    render_with_resolver(html, &RenderOptions::default(), &Denied).expect("unused later families, including automatic fallbacks, must not be opened when the primary face covers the text");
}

#[test]
fn unavailable_composite_faces_retry_earlier_rules_before_the_next_family() {
    let assets = Assets(HashMap::from([(
        PathBuf::from("/document/serif.ttf"),
        include_bytes!("../../crates/fonts/assets/fonts/DejaVuSerif.ttf").to_vec(),
    )]));
    let output = render_with_resolver(
        "<style>@font-face{font-family:Custom;src:url(serif.ttf)}@font-face{font-family:Custom;src:url(missing.ttf)}body{font-family:Custom,sans-serif}</style><p>Wide words</p>",
        &RenderOptions{base:"/document".into(),include_layout:true,..Default::default()}, &assets,
    ).unwrap();
    let trace = output.layout.unwrap();
    let run = &trace.pages[0][0].runs[0];
    assert_eq!(
        trace.fonts[run.font_id.index()].family,
        "Custom",
        "an unavailable later rule must allow an earlier face in the same family"
    );
    assert!(
        output
            .pdf
            .windows("DejaVuSerif".len())
            .any(|w| w == b"DejaVuSerif"),
        "the earlier font's actual outlines must be embedded"
    );
}

#[test]
fn unavailable_italic_faces_rematch_regular_faces_with_or_without_synthesis() {
    let assets = Assets(HashMap::from([(
        PathBuf::from("/document/serif.ttf"),
        include_bytes!("../../crates/fonts/assets/fonts/DejaVuSerif.ttf").to_vec(),
    )]));
    for (synthesis, enabled) in [("weight style", true), ("none", false)] {
        let html = format!(
            "<style>@font-face{{font-family:Custom;src:url(serif.ttf)}}@font-face{{font-family:Custom;font-style:italic;src:url(missing.ttf)}}body{{font-family:Custom,sans-serif;font-style:italic;font-synthesis:{synthesis}}}</style><p>Wide words</p>"
        );
        let output = render_with_resolver(
            &html,
            &RenderOptions {
                base: "/document".into(),
                include_layout: true,
                ..Default::default()
            },
            &assets,
        )
        .unwrap();
        let trace = output.layout.unwrap();
        let run = &trace.pages[0][0].runs[0];
        assert_eq!(
            trace.fonts[run.font_id.index()].family,
            "Custom",
            "an unavailable italic resource must rematch the regular face before leaving the family, synthesis={synthesis}"
        );
        assert_eq!(
            run.synthesis.oblique, enabled,
            "rematching must preserve synthesis permissions"
        );
    }
}

#[test]
fn synthesis_shorthand_participates_in_the_cascade_and_leaves_real_faces_enabled() {
    let html = format!(
        "<style>{PROFILE} p{{font-synthesis:none!important}} p{{font-synthesis-weight:auto}}</style><p><strong><em>Wide words</em></strong></p>"
    );
    let output = render_html(&html, &local_options()).unwrap();
    let trace = output.layout.unwrap();
    assert!(
        trace.pages[0][0]
            .runs
            .iter()
            .all(|r| !r.synthesis.bold && !r.synthesis.oblique),
        "important shorthand must disable both synthetic operations despite later ordinary longhand"
    );
    let output = render_html(
        "<p style='font-synthesis:none'><strong>Wide words</strong></p>",
        &RenderOptions {
            include_layout: true,
            ..Default::default()
        },
    )
    .unwrap();
    let trace = output.layout.unwrap();
    let run = &trace.pages[0][0].runs[0];
    assert_eq!(
        trace.fonts[run.font_id.index()].weight,
        ferri_pdf_fonts::FontRange::single(crate::FontWeight::BOLD),
        "disabling synthesis must still select a real bold face"
    );
}

#[test]
fn source_lists_try_supported_fonts_in_order_and_preserve_policy_errors() {
    let assets = Assets(HashMap::from([(
        PathBuf::from("/document/good.ttf"),
        FONT.to_vec(),
    )]));
    let html = "<style>@font-face{font-family:Custom;src:local(Uninstalled),url(unsupported.woff2) format('woff2'),url(missing.ttf),url(good.ttf) format('truetype')} body{font-family:Custom}</style><p>Loaded font</p>";
    let output = render_with_resolver(
        html,
        &RenderOptions {
            base: "/document".into(),
            include_layout: true,
            ..Default::default()
        },
        &assets,
    )
    .expect("unavailable sources and unsupported formats must allow the next source");
    let trace = output.layout.unwrap();
    let run = &trace.pages[0][0].runs[0];
    assert_eq!(
        trace.fonts[run.font_id.index()].family,
        "Custom",
        "a working later URL must be selected before moving to the next family"
    );
    let result = render_with_resolver(
        "<style>@font-face{font-family:Custom;src:url(crates/fonts/assets/fonts/DejaVuSans.ttf)}body{font-family:Custom}</style><p>Blocked font</p>",
        &RenderOptions {
            base: env!("CARGO_MANIFEST_DIR").into(),
            ..Default::default()
        },
        &crate::FileResolver {
            max_resource_bytes: 4,
            ..Default::default()
        },
    );
    assert!(
        matches!(
            result,
            Err(crate::Error::Css(crate::StyleError::Resource(
                ResourceError::TooLarge { limit: 4, .. }
            )))
        ),
        "resource policy errors must not be downgraded into font availability failures"
    );
}
use super::common;
use common::{PROFILE, local_options};
