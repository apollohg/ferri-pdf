mod common;
use common::{PROFILE, local_options};
use ferri_pdf::{DisplayItem, LayoutTrace, render_html};

const TOLERANCE_PT: f32 = 0.001;

#[test]
fn zoom_discards_shadow_lengths_that_overflow_used_coordinates() {
    for (property, shadow) in [
        ("box-shadow", "1e30pt 0 blue"),
        ("box-shadow", "0 1e30pt blue"),
        ("box-shadow", "0 0 1e30pt blue"),
        ("box-shadow", "0 0 0 1e30pt blue"),
        ("box-shadow", "0 0 0 -1e30pt blue inset"),
        ("text-shadow", "1e30pt 0 blue"),
        ("text-shadow", "0 1e30pt blue"),
        ("text-shadow", "0 0 1e30pt blue"),
    ] {
        for selector in ["div", "span", "div::before"] {
            let content = if selector.ends_with("::before") {
                "content:'X';"
            } else {
                ""
            };
            let document = |declaration: &str| {
                format!(
                    "<style>{PROFILE}{selector}{{{content}zoom:1e20;font-size:1e-19pt;line-height:1e-19pt;width:1e-19pt;height:1e-19pt;{declaration}}}</style><div><span>X</span></div>"
                )
            };
            let output = render_html(&document(&format!("{property}:{shadow}")), &local_options())
                .unwrap_or_else(|error| {
                    panic!("{selector}, {property}:{shadow}: overflowing paint lengths must not fail: {error}")
                });
            let reason = format!("non-finite zoomed {property} dimensions");
            assert!(
                output.diagnostics.iter().any(|diagnostic| matches!(
                    diagnostic,
                    ferri_pdf::RenderDiagnostic::Degraded(
                        ferri_pdf::Degradation::InvalidAtComputedValueTime { declaration, reason: found }
                    ) if declaration == property && *found == reason
                )),
                "{selector}, {property}:{shadow}: overflowing paint lengths must be reported: {:?}",
                output.diagnostics
            );
            let unshadowed = render_html(&document(""), &local_options())
                .expect("the unshadowed document renders")
                .layout
                .unwrap();
            assert_eq!(
                format!("{:?}", output.layout.unwrap().display_lists),
                format!("{:?}", unshadowed.display_lists),
                "{selector}, {property}:{shadow}: the shadow computes as unset, painting none"
            );
        }
        let finite = shadow.replace("1e30pt", "1e-19pt");
        let physical = shadow.replace("1e30pt", "10pt");
        same_layout(
            &render(
                "<div>X</div>",
                &format!(
                    "div{{zoom:1e20;font-size:1e-19pt;line-height:1e-19pt;width:1e-19pt;height:1e-19pt;{property}:{finite}}}"
                ),
            ),
            &render(
                "<div>X</div>",
                &format!(
                    "div{{font-size:10pt;line-height:10pt;width:10pt;height:10pt;{property}:{physical}}}"
                ),
            ),
        );
    }
}

#[test]
fn anchor_functions_use_the_consumers_unzoomed_coordinates() {
    let body = "<section><div class=anchor></div><div class=target>T</div></section>";
    for anchor_zoom in [0.5, 2.0] {
        for target_zoom in [0.5, 2.0] {
            let base = format!(
                "section{{position:relative;width:200pt;height:160pt}}.anchor{{anchor-name:--a;zoom:{anchor_zoom};width:30pt;height:20pt;margin-left:10pt;background:blue}}.target{{position:absolute;position-anchor:--a;zoom:{target_zoom};font-size:10pt;line-height:12pt;height:10pt;background:red;left:anchor(left);top:calc(anchor(bottom) + 5pt);width:anchor-size(width)}}"
            );
            let reference = format!(
                "{base}.target{{position-anchor:none;zoom:1;left:{}pt;top:{}pt;width:{}pt;height:{}pt;font-size:{}pt;line-height:{}pt}}",
                10.0 * anchor_zoom,
                20.0 * anchor_zoom + 5.0 * target_zoom,
                30.0 * anchor_zoom,
                10.0 * target_zoom,
                10.0 * target_zoom,
                12.0 * target_zoom
            );
            eprintln!("anchor zoom {anchor_zoom}, consumer zoom {target_zoom}");
            same_layout(&render(body, &base), &render(body, &reference));
        }
    }
}

#[test]
fn zoomed_border_images_keep_source_slices_and_scale_auto_widths() {
    for file in ["image-pattern.png", "image-pattern.svg"] {
        let actual = render(
            "<div></div>",
            &format!(
                "div{{zoom:2;width:100pt;height:70pt;border:1pt solid;border-image:url('../assets/{file}') 10 / auto}}"
            ),
        );
        let expected = render(
            "<div></div>",
            &format!(
                "div{{width:200pt;height:140pt;border:2pt solid;border-image:url('../assets/{file}') 10 / 15pt}}"
            ),
        );
        let pieces = |trace: &LayoutTrace| {
            trace
                .display_lists
                .iter()
                .flatten()
                .filter_map(|item| match item {
                    DisplayItem::Box(paint) => paint.border_image().unwrap(),
                    _ => None,
                })
                .flat_map(|image| {
                    image.pieces.into_iter().map(move |piece| {
                        (
                            piece.area,
                            piece.tiles,
                            [
                                piece.source.x / image.size[0],
                                piece.source.y / image.size[1],
                                piece.source.width / image.size[0],
                                piece.source.height / image.size[1],
                            ],
                        )
                    })
                })
                .collect::<Vec<_>>()
        };
        let actual = pieces(&actual);
        assert!(!actual.is_empty(), "border image must have painted slices");
        assert_eq!(actual, pieces(&expected), "border image pieces for {file}");
    }
}

#[test]
fn background_and_mask_natural_sizes_scale_with_zoom() {
    for (source, size) in [
        ("url('../assets/image-pattern.png')", "120pt 60pt"),
        ("url('../assets/image-pattern.svg')", "120pt 60pt"),
        (
            "image-set(url('../assets/image-pattern.png') 2x)",
            "60pt 30pt",
        ),
        (
            "image-set(url('../assets/image-pattern.svg') 2x)",
            "120pt 60pt",
        ),
    ] {
        for property in ["background", "mask"] {
            let body = "<div></div>";
            let image = format!("{property}-image:{source};{property}-repeat:no-repeat");
            let actual = render(
                body,
                &format!("div{{zoom:2;width:100pt;height:70pt;{image}}}"),
            );
            let expected = render(
                body,
                &format!("div{{width:200pt;height:140pt;{image};{property}-size:{size}}}"),
            );
            let tiles = |trace: &LayoutTrace| {
                trace
                    .display_lists
                    .iter()
                    .flatten()
                    .filter_map(|item| match item {
                        DisplayItem::Box(paint) => Some(paint),
                        _ => None,
                    })
                    .flat_map(|paint| {
                        if property == "background" {
                            paint
                                .background_layers(trace.page_area)
                                .unwrap()
                                .into_iter()
                                .map(|layer| layer.tile)
                                .collect::<Vec<_>>()
                        } else {
                            paint
                                .mask_layers(trace.page_area)
                                .unwrap()
                                .into_iter()
                                .map(|(_, layer)| layer.tile)
                                .collect::<Vec<_>>()
                        }
                    })
                    .collect::<Vec<_>>()
            };
            let actual = tiles(&actual);
            assert!(
                !actual.is_empty(),
                "{property} must paint the {source} image"
            );
            assert_eq!(
                actual,
                tiles(&expected),
                "{property} natural tile for {source}"
            );
        }
    }
}

#[test]
fn control_internal_styles_use_zoomed_font_metrics() {
    const RESET: &str = "input,textarea,select,option,optgroup,input::file-selector-button{margin:0;padding:0;border:0;appearance:none}input,textarea,select{display:block}input::placeholder,textarea::placeholder,input::file-selector-button,option,optgroup{font:inherit}";
    for control in [
        "<input value='Value'>",
        "<input placeholder='Placeholder'>",
        "<textarea placeholder='Placeholder'></textarea>",
        "<input type='file'>",
        "<select><option>Selected option</option></select>",
        "<select size='3'><optgroup label='Group'><option>Option</option></optgroup></select>",
    ] {
        let body = format!("<section>{control}</section><p>After</p>");
        eprintln!("control zoom: {control}");
        same_layout(
            &render(
                &body,
                &format!(
                    "{RESET}section{{zoom:2}}input,textarea,select{{font-size:10pt;line-height:14pt}}"
                ),
            ),
            &render(
                &body,
                &format!("{RESET}input,textarea,select{{font-size:20pt;line-height:28pt}}"),
            ),
        );
    }
}

fn render(body: &str, css: &str) -> LayoutTrace {
    render_html(
        &format!("<style>{PROFILE}body,p{{margin:0}}{css}</style>{body}"),
        &local_options(),
    )
    .expect("CSS zoom must participate in native paged layout")
    .layout
    .unwrap()
}

fn image_rects(trace: &LayoutTrace) -> Vec<ferri_pdf::Rect> {
    trace
        .display_lists
        .iter()
        .flatten()
        .filter_map(|item| match item {
            DisplayItem::Image(paint) => Some(paint.rect),
            _ => None,
        })
        .collect()
}

#[test]
fn native_widgets_scale_their_natural_sizes_once() {
    for control in [
        "<input type='checkbox' checked>",
        "<input type='radio' checked>",
        "<progress value='.5'></progress>",
        "<meter value='.5'></meter>",
    ] {
        let body = format!("<section>{control}</section><p>After</p>");
        let actual = render(
            &body,
            "section{zoom:2}input,progress,meter{display:block;margin:0;font-size:10pt}",
        );
        let expected = render(
            &body,
            "input,progress,meter{display:block;margin:0;font-size:20pt}",
        );
        eprintln!("native widget zoom: {control}");
        same_layout(&actual, &expected);
        let actual = image_rects(&actual);
        assert!(!actual.is_empty(), "native widget must paint an image");
        assert_eq!(
            actual,
            image_rects(&expected),
            "widget image bounds for {control}"
        );
    }
}

fn same_layout(actual: &LayoutTrace, expected: &LayoutTrace) {
    assert_eq!(actual.page_width, expected.page_width, "sheet width");
    assert_eq!(actual.page_height, expected.page_height, "sheet height");
    assert_eq!(actual.page_area, expected.page_area, "page content area");
    assert_eq!(actual.pages.len(), expected.pages.len(), "page count");
    for (page, (actual, expected)) in actual.pages.iter().zip(&expected.pages).enumerate() {
        assert_eq!(actual.len(), expected.len(), "page {page} line count");
        for (line, (actual, expected)) in actual.iter().zip(expected).enumerate() {
            assert_eq!(actual.text, expected.text, "page {page}, line {line}");
            for (name, actual, expected) in [
                ("x", actual.x, expected.x),
                ("baseline", actual.baseline, expected.baseline),
                ("width", actual.width, expected.width),
            ] {
                assert!(
                    (actual - expected).abs() < TOLERANCE_PT,
                    "page {page}, line {line}, {name}: {actual} versus reference {expected}"
                );
            }
            let sizes = |line: &ferri_pdf::TextLine| {
                line.runs
                    .iter()
                    .map(|run| run.font_size)
                    .collect::<Vec<_>>()
            };
            assert_eq!(sizes(actual), sizes(expected), "used font sizes");
        }
    }
    let boxes = |trace: &LayoutTrace| {
        trace
            .display_lists
            .iter()
            .flatten()
            .filter_map(|item| match item {
                DisplayItem::Box(paint) => Some(paint.rect),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    let (actual, expected) = (boxes(actual), boxes(expected));
    assert_eq!(actual.len(), expected.len(), "painted box count");
    for (actual, expected) in actual.iter().zip(expected) {
        for (actual, expected) in [
            (actual.x, expected.x),
            (actual.y, expected.y),
            (actual.width, expected.width),
            (actual.height, expected.height),
        ] {
            assert!(
                (actual - expected).abs() < TOLERANCE_PT,
                "painted box geometry: {actual} versus reference {expected}"
            );
        }
    }
}

#[test]
fn propagated_decoration_keeps_the_originating_zoom() {
    let body = "<section><p>Decorated text</p></section>";
    let actual = render(
        body,
        "section{zoom:2;text-decoration:underline;text-decoration-thickness:1pt}p{zoom:.5;font-size:10pt;line-height:12pt}",
    );
    let expected = render(
        body,
        "section{text-decoration:underline;text-decoration-thickness:2pt}p{font-size:10pt;line-height:12pt}",
    );
    same_layout(&actual, &expected);
    assert_eq!(
        serde_json::to_value(&actual.display_lists).unwrap(),
        serde_json::to_value(&expected.display_lists).unwrap(),
        "ancestor decoration thickness must retain its originating zoom across child cancellation"
    );
}

#[test]
fn zoom_scales_natural_image_sizes_and_preserves_ratio_only_sizing() {
    for (file, width, height) in [
        ("image-pattern.png", "120pt", "60pt"),
        ("image-pattern.svg", "120pt", "60pt"),
        ("image-ratio.svg", "200pt", "100pt"),
    ] {
        let body = format!("<section><img src='../assets/{file}'></section><p>After</p>");
        let actual = render(&body, "section{width:200pt}img{display:block;zoom:2}");
        let expected = render(
            &body,
            &format!("section{{width:200pt}}img{{display:block;width:{width};height:{height}}}"),
        );
        eprintln!("natural image zoom: {file}");
        same_layout(&actual, &expected);
        assert_eq!(
            image_rects(&actual),
            image_rects(&expected),
            "painted image bounds for {file}"
        );
    }
}

#[test]
fn zoom_scales_dimensions_edges_text_and_following_flow() {
    let body = "<div>Alpha beta</div><p>After</p>";
    same_layout(
        &render(
            body,
            "div{zoom:2;width:80pt;height:30pt;padding:3pt;border:1pt solid navy;margin:5pt;font-size:10pt;line-height:12pt}",
        ),
        &render(
            body,
            "div{width:160pt;height:60pt;padding:6pt;border:2pt solid navy;margin:10pt;font-size:20pt;line-height:24pt}",
        ),
    );
}

#[test]
fn containment_tabs_and_line_steps_use_projected_lengths() {
    for (name, body, actual, expected) in [
        (
            "intrinsic containment",
            "<section><div>Content</div></section><p style='clear:both'>After</p>",
            "section{float:left;contain:size;contain-intrinsic-size:30pt 40pt;background:blue}",
            "section{float:left;contain:size;contain-intrinsic-size:60pt 80pt;background:blue}",
        ),
        (
            "length tab stops",
            "<section>A\tB\tC<br>Long\tD</section><p>After</p>",
            "section{white-space:pre;tab-size:25pt}",
            "section{white-space:pre;tab-size:50pt}",
        ),
        (
            "line height steps",
            "<section>First<br>Second<br>Third</section><p>After</p>",
            "section{line-height-step:8pt}",
            "section{line-height-step:16pt}",
        ),
    ] {
        eprintln!("zoom additional layout: {name}");
        same_layout(
            &render(
                body,
                &format!("section{{zoom:2;font-size:10pt;line-height:12pt}}{actual}"),
            ),
            &render(
                body,
                &format!("section{{font-size:20pt;line-height:24pt}}{expected}"),
            ),
        );
    }
}

#[test]
fn font_relative_lengths_keep_canonical_current_and_root_metrics() {
    for (width, physical) in [
        ("10em", "160pt"),
        ("10rem", "200pt"),
        ("10lh", "200pt"),
        ("10rlh", "240pt"),
    ] {
        let body = "<section><div>Relative text</div></section><p>After</p>";
        let common = "html{font-size:10pt;line-height:12pt}div{font-size:calc(1em + 2pt);width:2em;background:blue}";
        eprintln!("zoom font-relative width: {width}");
        same_layout(
            &render(
                body,
                &format!(
                    "{common}section{{zoom:2;font-size:8pt;line-height:10pt;width:{width};border:1pt solid}}"
                ),
            ),
            &render(
                body,
                &format!(
                    "{common}section{{font-size:16pt;line-height:20pt;width:{physical};border:2pt solid}}div{{font-size:20pt}}"
                ),
            ),
        );
    }
}

#[test]
fn deeply_nested_zoom_cancels_without_reprojecting_inherited_lengths() {
    const PAIRS: usize = 32;
    let body = format!(
        "{}<div>Nested</div>{}<p>After</p>",
        "<section style='zoom:2'><section style='zoom:.5'>".repeat(PAIRS),
        "</section></section>".repeat(PAIRS)
    );
    let css = "section{display:contents;font-size:inherit;line-height:inherit}body{font-size:10pt;line-height:12pt}div{width:10em;border:1pt solid}";
    same_layout(
        &render(&body, css),
        &render("<div>Nested</div><p>After</p>", css),
    );
}

#[test]
fn nested_zoom_uses_unscaled_inherited_lengths() {
    let body = "<section><p>Outer</p><div>Inner</div></section><p>After</p>";
    same_layout(
        &render(
            body,
            "section{zoom:2;width:100pt;font-size:10pt;line-height:12pt}div{zoom:.5;width:inherit;font-size:inherit;line-height:inherit;border:1pt solid navy}",
        ),
        &render(
            body,
            "section{width:200pt;font-size:20pt;line-height:24pt}div{width:100pt;font-size:10pt;line-height:12pt;border:1pt solid navy}",
        ),
    );
}

#[test]
fn percentages_and_scalar_calc_preserve_their_containing_basis() {
    let body = "<section><div>Alpha beta gamma</div></section><p>After</p>";
    for (width, expected) in [
        ("50%", "100pt"),
        ("calc(2 * (10pt + 25%))", "140pt"),
        ("min(80pt, 60%)", "120pt"),
    ] {
        same_layout(
            &render(
                body,
                &format!(
                    "section{{width:200pt}}div{{zoom:2;width:{width};font-size:10pt;line-height:12pt;background:navy}}"
                ),
            ),
            &render(
                body,
                &format!(
                    "section{{width:200pt}}div{{width:{expected};font-size:20pt;line-height:24pt;background:navy}}"
                ),
            ),
        );
    }
}

#[test]
fn boxless_ancestors_accumulate_zoom_once() {
    let body = "<section><div><p>Nested text</p></div></section><p>After</p>";
    same_layout(
        &render(
            body,
            "section{display:contents;zoom:2;font-size:10pt;line-height:12pt}div{zoom:inherit}p{border:1pt solid navy}",
        ),
        &render(
            body,
            "section{display:contents;font-size:40pt;line-height:48pt}p{border:1pt solid navy}section p{border-width:4pt}",
        ),
    );
}

#[test]
fn columns_and_float_exclusions_preserve_zoomed_fragmentation() {
    let text = "Alpha beta gamma delta epsilon zeta. ".repeat(80);
    for (name, contents, actual_rules, reference_rules) in [
        (
            "balanced columns",
            format!("<p>{text}</p>"),
            "section{columns:2;column-gap:8pt;column-rule:1pt solid}",
            "section{columns:2;column-gap:16pt;column-rule:2pt solid}",
        ),
        (
            "fixed height columns",
            format!("<p>{text}</p>"),
            "section{columns:60pt;column-gap:8pt;height:300pt;column-fill:auto}",
            "section{columns:120pt;column-gap:16pt;height:600pt;column-fill:auto}",
        ),
        (
            "shaped float",
            format!("<aside></aside><p>{text}</p>"),
            "aside{float:left;width:40pt;height:80pt;margin:3pt;shape-outside:circle(30pt);shape-margin:2pt;background:blue}",
            "aside{float:left;width:80pt;height:160pt;margin:6pt;shape-outside:circle(60pt);shape-margin:4pt;background:blue}",
        ),
    ] {
        eprintln!("zoom fragmentation comparison: {name}");
        let body = format!("<section>{contents}</section><p>After</p>");
        let expected = render(
            &body,
            &format!("section{{width:280pt;font-size:16pt;line-height:20pt}}{reference_rules}"),
        );
        assert!(
            expected.pages.len() > 1,
            "{name} must cross a page boundary"
        );
        let actual = render(
            &body,
            &format!("section{{zoom:2;width:140pt;font-size:8pt;line-height:10pt}}{actual_rules}"),
        );
        same_layout(&actual, &expected);
    }
}

#[test]
fn root_zoom_changes_pagination_without_scaling_the_sheet() {
    let body = format!("<p>{}</p>", "Alpha beta gamma delta. ".repeat(50));
    let actual = render(&body, "html{zoom:2}p{font-size:10pt;line-height:12pt}");
    let expected = render(&body, "p{font-size:20pt;line-height:24pt}");
    assert!(
        actual.pages.len() > 1,
        "test must exercise page fragmentation"
    );
    same_layout(&actual, &expected);
}

#[test]
fn zero_normal_and_percentage_zoom_share_factor_semantics() {
    let body = "<section><p>Text</p></section><p>After</p>";
    let expected = render(body, "section{font-size:20pt;line-height:24pt}");
    for zoom in ["0", "0%", "normal", "initial", "unset", "calc(-1)"] {
        same_layout(
            &render(
                body,
                &format!(
                    "section{{zoom:200%;font-size:10pt;line-height:12pt}}section p{{zoom:{zoom}}}"
                ),
            ),
            &expected,
        );
    }
}

#[test]
fn generated_content_and_first_line_use_projected_typography() {
    let body = "<p>Alpha beta gamma delta epsilon</p><p>After</p>";
    for pseudo in [
        "::before{content:'Before ';font-size:11pt}",
        "::first-line{font-size:11pt}",
    ] {
        same_layout(
            &render(
                body,
                &format!("p{{zoom:2;font-size:10pt;line-height:12pt;width:80pt}}p{pseudo}"),
            ),
            &render(
                body,
                &format!(
                    "p{{font-size:20pt;line-height:24pt;width:160pt}}p{}",
                    pseudo.replace("11pt", "22pt")
                ),
            ),
        );
    }
}

#[test]
fn anonymous_writing_mode_boxes_preserve_zoomed_lengths() {
    for mode in ["horizontal-tb", "vertical-rl", "vertical-lr"] {
        for inner in ["horizontal-tb", "vertical-rl"] {
            eprintln!("zoom anonymous flow: outer {mode}, inner {inner}");
            let body = "<div class=sample><section><p>Alpha beta gamma delta epsilon zeta.</p><p>Second line.</p></section></div><p>After</p>";
            let common = format!(".sample{{writing-mode:{mode}}}section{{writing-mode:{inner}}}");
            same_layout(
                &render(
                    body,
                    &format!(
                        "{common}.sample{{zoom:2;font-size:10pt;line-height:12pt;width:100pt;height:80pt;padding:2pt;border:1pt solid;margin:3pt}}section{{width:70pt;height:60pt}}"
                    ),
                ),
                &render(
                    body,
                    &format!(
                        "{common}.sample{{font-size:20pt;line-height:24pt;width:200pt;height:160pt;padding:4pt;border:2pt solid;margin:6pt}}section{{width:140pt;height:120pt}}"
                    ),
                ),
            );
        }
    }
}

#[test]
fn ruby_and_initial_letter_used_styles_preserve_zoom() {
    for (name, contents, extra) in [
        (
            "ruby over",
            "<ruby>AB<rt>cd</rt>EF<rt>gh</rt></ruby>",
            "rt{ruby-position:over;font-size:50%}",
        ),
        (
            "ruby inter-character",
            "<ruby>AB<rt>cd</rt>EF<rt>gh</rt></ruby>",
            "rt{ruby-position:inter-character;font-size:50%}",
        ),
        (
            "initial letter",
            "<p>Alpha beta gamma delta epsilon zeta eta theta iota kappa lambda.</p>",
            ".sample p::first-letter{initial-letter:3;color:red}",
        ),
        (
            "first line and initial letter",
            "<p>Alpha beta gamma delta epsilon zeta eta theta iota kappa lambda.</p>",
            ".sample p::first-line{color:blue}.sample p::first-letter{initial-letter:2;color:red}",
        ),
        (
            "anonymous marker",
            "<ol><li>Alpha beta gamma</li><li>Delta epsilon</li></ol>",
            "ol{margin:0;padding:0;list-style-position:inside}li::marker{color:red}",
        ),
    ] {
        eprintln!("zoom used-style comparison: {name}");
        let body = format!("<div class=sample>{contents}</div><p>After</p>");
        same_layout(
            &render(
                &body,
                &format!("{extra}.sample{{zoom:2;font-size:10pt;line-height:12pt;width:120pt}}"),
            ),
            &render(
                &body,
                &format!("{extra}.sample{{font-size:20pt;line-height:24pt;width:240pt}}"),
            ),
        );
    }
}

#[test]
fn grid_tracks_flex_gaps_and_table_spacing_use_zoomed_lengths() {
    for (body, actual, expected) in [
        (
            "<div><p>A</p><p>B</p></div>",
            "div{display:grid;grid-template-columns:20pt 30pt;gap:5pt}",
            "div{display:grid;grid-template-columns:40pt 60pt;gap:10pt}",
        ),
        (
            "<div><p>A</p><p>B</p></div>",
            "div{display:flex;gap:5pt}div p{flex:0 0 20pt}",
            "div{display:flex;gap:10pt}div p{flex:0 0 40pt}",
        ),
        (
            "<div><table><tr><td>A</td><td>B</td></tr></table></div>",
            "table{border-spacing:5pt;font-size:inherit;line-height:inherit}td{padding:0}",
            "table{border-spacing:10pt;font-size:inherit;line-height:inherit}td{padding:0}",
        ),
    ] {
        eprintln!("zoom formatting comparison: {actual}");
        same_layout(
            &render(
                body,
                &format!("div{{zoom:2;font-size:10pt;line-height:12pt}}{actual}"),
            ),
            &render(
                body,
                &format!("div{{font-size:20pt;line-height:24pt}}{expected}"),
            ),
        );
    }
}

#[test]
fn zoom_scales_paint_lengths_without_scaling_angles_or_factors() {
    let body = "<div>A</div>";
    for (css, reference) in [
        (
            "box-shadow:3pt 4pt 2pt 1pt navy",
            "box-shadow:6pt 8pt 4pt 2pt navy",
        ),
        (
            "outline:99pt auto navy;outline-offset:2pt",
            "outline:3.375pt solid navy;outline-offset:4pt",
        ),
        (
            "text-shadow:1pt 2pt 1pt navy",
            "text-shadow:2pt 4pt 2pt navy",
        ),
        (
            "filter:blur(1pt) opacity(.8)",
            "filter:blur(2pt) opacity(.8)",
        ),
        (
            "transform:translate(3pt,5pt) rotate(30deg)",
            "transform:translate(6pt,10pt) rotate(30deg)",
        ),
        (
            "clip-path:inset(2pt round 3pt)",
            "clip-path:inset(4pt round 6pt)",
        ),
        (
            "text-decoration:underline;text-decoration-thickness:1pt;text-underline-offset:2pt",
            "text-decoration:underline;text-decoration-thickness:2pt;text-underline-offset:4pt",
        ),
        (
            "background:linear-gradient(red 10pt,blue 20pt)",
            "background:linear-gradient(red 20pt,blue 40pt)",
        ),
    ] {
        let actual = render(
            body,
            &format!("div{{zoom:2;width:40pt;height:30pt;font-size:10pt;line-height:12pt;{css}}}"),
        );
        let expected = render(
            body,
            &format!("div{{width:80pt;height:60pt;font-size:20pt;line-height:24pt;{reference}}}"),
        );
        same_layout(&actual, &expected);
        let actual = serde_json::to_value(&actual.display_lists).unwrap();
        let expected = serde_json::to_value(&expected.display_lists).unwrap();
        assert_eq!(actual, expected, "used paint for {css}");
    }
}
