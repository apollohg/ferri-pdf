use crate::images::{Image, ImageCache, ImageError};
use crate::resources::{FileResolver, Resource, ResourceBase, ResourceError, ResourceResolver};
use std::cell::Cell;
use std::fmt::Write;

const SIZE: [f32; 2] = [200.0, 100.0];
const EPSILON: f32 = 0.01;
const SVG_OPEN: &str = "<svg xmlns='http://www.w3.org/2000/svg' width='200' height='100'>";

fn tree(body: &str) -> crate::svg::usvg::Tree {
    svg(&format!("{SVG_OPEN}{body}</svg>"))
}

fn svg(xml: &str) -> crate::svg::usvg::Tree {
    let image = ImageCache::default()
        .inline_svg(
            xml.to_owned(),
            &ResourceBase::default(),
            &FileResolver::default(),
        )
        .expect("SVG contents fixture must load");
    let Image::Svg(svg) = image.as_ref() else {
        panic!("inline SVG must use the vector pipeline")
    };
    svg.tree(SIZE).expect("SVG contents fixture must convert")
}

fn groups(tree: &crate::svg::usvg::Tree) -> Vec<&crate::svg::usvg::Group> {
    let mut output = Vec::new();
    let mut pending = vec![tree.root()];
    while let Some(group) = pending.pop() {
        output.push(group);
        for node in group.children() {
            match node {
                crate::svg::usvg::Node::Group(group) => pending.push(group.as_ref()),
                crate::svg::usvg::Node::Text(text) => pending.push(text.flattened()),
                crate::svg::usvg::Node::Path(_) | crate::svg::usvg::Node::Image(_) => {}
            }
        }
    }
    output
}

fn paths(tree: &crate::svg::usvg::Tree) -> Vec<&crate::svg::usvg::Path> {
    groups(tree)
        .into_iter()
        .flat_map(crate::svg::usvg::Group::children)
        .filter_map(|node| {
            if let crate::svg::usvg::Node::Path(path) = node {
                Some(path.as_ref())
            } else {
                None
            }
        })
        .collect()
}

fn assert_bounds(tree: &crate::svg::usvg::Tree, expected: [f32; 4]) {
    let rect = tree.root().abs_bounding_box();
    let actual = [rect.x(), rect.y(), rect.width(), rect.height()];
    for (actual, expected) in actual.into_iter().zip(expected) {
        assert!(
            (actual - expected).abs() <= EPSILON,
            "expected SVG bounds {expected}, got {actual}; full bounds {rect:?}"
        );
    }
}

fn assert_fill(tree: &crate::svg::usvg::Tree, expected: [u8; 3]) {
    let paths = paths(tree);
    assert_eq!(paths.len(), 1, "the selector fixture must render one shape");
    assert_eq!(paint_color(paths[0].fill().unwrap().paint()), expected);
}

fn paint_color(paint: &crate::svg::usvg::Paint) -> [u8; 3] {
    let crate::svg::usvg::Paint::Color(color) = paint else {
        panic!("SVG fixture must have a solid paint")
    };
    [color.red, color.green, color.blue]
}

#[test]
fn boxless_svg_groups_ignore_their_effects_without_changing_selector_ancestry() {
    let tree = tree(
        "<style>#box{display:contents}#box > .paint{fill:#123456}</style><g id='box' fill='red' transform='translate(50 0)' opacity='.2'><rect class='paint' x='10' y='20' width='30' height='40'/></g>",
    );
    assert_bounds(&tree, [10.0, 20.0, 30.0, 40.0]);
    assert_fill(&tree, [0x12, 0x34, 0x56]);
    for group in groups(&tree) {
        assert_eq!(
            group.opacity().get(),
            1.0,
            "boxless group opacity must not survive"
        );
    }
}

#[test]
fn boxless_nested_svg_does_not_create_a_viewport_or_transform() {
    let tree = tree(
        "<svg display='contents' x='60' y='30' width='20' height='10' viewBox='0 0 2 1' overflow='hidden'><rect x='10' y='20' width='30' height='40'/></svg>",
    );
    assert_bounds(&tree, [10.0, 20.0, 30.0, 40.0]);
}

#[test]
fn boxless_use_keeps_its_instantiated_content_without_its_own_position_or_effects() {
    let tree = tree(
        "<defs><g id='source'><rect x='10' y='20' width='30' height='40'/></g></defs><use href='#source' display='contents' x='60' y='30' transform='scale(2)' opacity='.2'/>",
    );
    assert_bounds(&tree, [10.0, 20.0, 30.0, 40.0]);
    assert_eq!(paths(&tree).len(), 1, "use shadow content remains rendered");
}

#[test]
fn boxless_text_children_ignore_position_rotation_length_and_path_layout() {
    let reference = tree(
        "<text x='10' y='40' font-family='DejaVu Sans' font-size='12'><tspan fill='red'>Text</tspan></text>",
    );
    let rect = reference.root().abs_bounding_box();
    let expected = [rect.x(), rect.y(), rect.width(), rect.height()];
    for body in [
        "<text x='10' y='40' font-family='DejaVu Sans' font-size='12'><tspan display='contents' x='80' y='80' dx='20' dy='10' rotate='45' textLength='150' lengthAdjust='spacingAndGlyphs' fill='red'>Text</tspan></text>",
        "<defs><path id='route' d='M70 90H190'/></defs><text x='10' y='40' font-family='DejaVu Sans' font-size='12'><textPath display='contents' href='#route' startOffset='30' fill='red'>Text</textPath></text>",
    ] {
        let actual = tree(body);
        assert_bounds(&actual, expected);
        assert_eq!(paths(&actual).len(), paths(&reference).len());
    }
}

#[test]
fn contents_on_svg_shapes_images_and_text_roots_hides_their_content() {
    for body in [
        "<rect display='contents' width='100' height='100'/>",
        "<text display='contents' x='10' y='40'>Hidden</text>",
        "<text x='10' y='40'><tspan display='none'><tspan>Hidden</tspan></tspan></text>",
    ] {
        assert!(
            paths(&tree(body)).is_empty(),
            "non-container SVG contents must compute to none: {body}"
        );
    }
}

#[test]
fn unused_boxless_svg_resources_are_not_loaded_and_active_denials_stay_typed() {
    const DENIED: &str = "https://denied.example/image.png";
    #[derive(Default)]
    struct Policy {
        requests: Cell<usize>,
    }
    impl ResourceResolver for Policy {
        fn resolve(&self, reference: &str, base: &ResourceBase) -> Result<Resource, ResourceError> {
            self.requests.set(self.requests.get() + 1);
            Err(ResourceError::Denied(base.resolve(reference)?))
        }
    }
    for (body, active) in [
        (
            format!(
                "<style>.off{{display:contents}}</style><image class='off' href='{DENIED}' width='20' height='20'/>"
            ),
            false,
        ),
        (
            format!(
                "<defs><filter id='unused'><feImage href='{DENIED}'/></filter></defs><g display='contents' filter='url(#unused)'><rect width='10' height='10'/></g>"
            ),
            false,
        ),
        (
            format!("<g display='contents'><image href='{DENIED}' width='20' height='20'/></g>"),
            true,
        ),
        (
            format!(
                "<style>:host > image{{display:none}}</style><defs><image id='asset' href='{DENIED}' width='20' height='20'/></defs><use href='#asset'/>"
            ),
            false,
        ),
        (
            format!(
                "<style>.source image{{display:none}}</style><defs class='source'><image id='asset' href='{DENIED}' width='20' height='20'/></defs><use href='#asset'/>"
            ),
            true,
        ),
    ] {
        let policy = Policy::default();
        let result = ImageCache::default().inline_svg(
            format!("{SVG_OPEN}{body}</svg>"),
            &ResourceBase::default(),
            &policy,
        );
        if active {
            assert!(
                matches!(result, Err(ImageError::Resource(ResourceError::Denied(_)))),
                "active SVG resource denial must remain typed"
            );
            assert_eq!(policy.requests.get(), 1);
        } else {
            assert!(
                result.is_ok(),
                "unboxed or inactive SVG resources must not be requested: {:?}",
                result.err()
            );
            assert_eq!(policy.requests.get(), 0);
        }
    }
}

#[test]
fn svg_display_keywords_use_css_case_and_escape_rules() {
    for display in ["CoNtEnTs", r"c\6f ntents", "contents /* trailing */"] {
        eprintln!("checking CSS display value {display:?}");
        let actual = tree(&format!(
            "<g style='display:{display}' transform='translate(50)'><rect x='10' y='20' width='30' height='40'/></g>"
        ));
        assert_bounds(&actual, [10.0, 20.0, 30.0, 40.0]);
        let sheet = tree(&format!(
            "<style>g{{DISPLAY:{display}}}</style><g transform='translate(50)'><rect x='10' y='20' width='30' height='40'/></g>"
        ));
        assert_bounds(&sheet, [10.0, 20.0, 30.0, 40.0]);
    }
}

#[test]
fn unboxing_preserves_computed_values_for_explicit_child_inheritance() {
    let actual = tree(
        "<g display='contents' transform='translate(50)' opacity='.2'><rect style='transform:inherit;opacity:inherit' x='10' y='20' width='30' height='40'/></g>",
    );
    assert_bounds(&actual, [60.0, 20.0, 30.0, 40.0]);
    let effects: Vec<_> = groups(&actual)
        .iter()
        .map(|group| group.opacity().get())
        .filter(|opacity| *opacity != 1.0)
        .collect();
    assert_eq!(
        effects,
        [0.2],
        "only the real child's explicitly inherited opacity paints"
    );
    let reset = tree(
        "<g display='contents'><rect transform='translate(90)' style='transform:inherit' x='10' y='20' width='30' height='40'/></g>",
    );
    assert_bounds(&reset, [10.0, 20.0, 30.0, 40.0]);
}

#[test]
fn boxless_use_ignores_its_dimensions_but_keeps_referenced_svg_viewports() {
    for (body, expected) in [
        (
            "<defs><symbol id='source' viewBox='0 0 200 100'><rect x='10' y='20' width='30' height='40'/></symbol></defs><use display='contents' href='#source' x='60' y='30' width='20' height='10'/>",
            [10.0, 20.0, 30.0, 40.0],
        ),
        (
            "<defs><svg id='source' x='5' y='7' width='100' height='50' viewBox='0 0 100 50'><rect x='10' y='20' width='30' height='40'/></svg></defs><use display='contents' href='#source' x='60' y='30' width='20' height='10'/>",
            [15.0, 27.0, 30.0, 40.0],
        ),
    ] {
        assert_bounds(&tree(body), expected);
    }
}

#[test]
fn boxless_text_drops_own_decorations_and_shift_but_retains_ancestor_decoration() {
    let plain = tree("<text x='10' y='40' font-family='DejaVu Sans' font-size='12'>Text</text>");
    let unboxed = tree(
        "<text x='10' y='40' font-family='DejaVu Sans' font-size='12'><tspan display='contents' text-decoration='underline' baseline-shift='20'>Text</tspan></text>",
    );
    assert_eq!(paths(&plain).len(), paths(&unboxed).len());
    let bounds = plain.root().abs_bounding_box();
    assert_bounds(
        &unboxed,
        [bounds.x(), bounds.y(), bounds.width(), bounds.height()],
    );
    let real_decoration = tree(
        "<text x='10' y='40' font-family='DejaVu Sans' font-size='12' text-decoration='underline'><tspan display='contents'>Text</tspan></text>",
    );
    assert!(
        paths(&real_decoration).len() > paths(&plain).len(),
        "a real ancestor's decoration crosses the boxless span"
    );
}

#[test]
fn empty_invalid_display_declarations_do_not_replace_an_earlier_valid_value() {
    for body in [
        "<g display='none' style='display:;fill:red'><rect width='20' height='20'/></g>",
        "<style>g{display:none}g{display:;fill:red}</style><g><rect width='20' height='20'/></g>",
    ] {
        assert!(
            paths(&tree(body)).is_empty(),
            "invalid empty display must not unhide content: {body}"
        );
    }
}

#[test]
fn a_boxless_svg_document_root_has_no_rendered_content() {
    let tree = svg(
        "<svg xmlns='http://www.w3.org/2000/svg' width='200' height='100' display='contents'><rect width='100' height='100'/></svg>",
    );
    assert!(paths(&tree).is_empty());
}

#[test]
fn unboxed_switch_keeps_conditional_processing_without_its_own_transform() {
    for (condition, x) in [
        ("", 10.0),
        ("requiredExtensions='https://unsupported.example/'", 80.0),
    ] {
        let actual = tree(&format!(
            "<switch display='contents' transform='translate(50)'><rect {condition} x='10' y='20' width='30' height='40'/><rect x='80' y='20' width='30' height='40'/></switch>"
        ));
        assert_bounds(&actual, [x, 20.0, 30.0, 40.0]);
        assert_eq!(paths(&actual).len(), 1);
    }
}

#[test]
fn text_paths_can_be_hoisted_through_boxless_text_spans() {
    let route = "<defs><path id='route' d='M10 40H190'/></defs>";
    let reference = tree(&format!(
        "{route}<text font-family='DejaVu Sans' font-size='12'><textPath href='#route'>Text</textPath></text>"
    ));
    let actual = tree(&format!(
        "{route}<text font-family='DejaVu Sans' font-size='12'><tspan display='contents'><textPath href='#route'>Text</textPath></tspan></text>"
    ));
    let bounds = reference.root().abs_bounding_box();
    assert_bounds(
        &actual,
        [bounds.x(), bounds.y(), bounds.width(), bounds.height()],
    );
    assert_eq!(paths(&actual).len(), paths(&reference).len());
}

fn png_data_url() -> String {
    use std::fmt::Write;
    const PIXELS: [u8; 8] = [255, 0, 0, 255, 0, 0, 255, 255];
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, 2, 1);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&PIXELS).unwrap();
        writer.finish().unwrap();
    }
    let mut url = String::from("data:image/png,");
    for byte in bytes {
        write!(url, "%{byte:02X}").unwrap();
    }
    url
}

#[test]
fn styled_resource_resolution_reuses_real_images_across_resized_viewports() {
    #[derive(Default)]
    struct Observed {
        calls: Cell<usize>,
        files: FileResolver,
    }
    impl ResourceResolver for Observed {
        fn resolve(&self, reference: &str, base: &ResourceBase) -> Result<Resource, ResourceError> {
            self.calls.set(self.calls.get() + 1);
            self.files.resolve(reference, base)
        }
    }
    let resolver = Observed::default();
    let href = png_data_url();
    let xml = format!(
        "{SVG_OPEN}<style>image[href='{href}']{{opacity:.5}}</style><g display='contents' transform='translate(50)'><image href='{href}' width='100%' height='100%'/></g></svg>"
    );
    let image = ImageCache::default()
        .inline_svg(xml, &ResourceBase::default(), &resolver)
        .unwrap();
    let Image::Svg(svg) = image.as_ref() else {
        panic!("vector image expected")
    };
    for (size, bounds) in [
        (SIZE, [0.0, 0.0, 200.0, 100.0]),
        ([100.0, 100.0], [0.0, 25.0, 100.0, 50.0]),
        ([400.0, 100.0], [100.0, 0.0, 200.0, 100.0]),
    ] {
        let tree = svg.tree(size).unwrap();
        assert_bounds(&tree, bounds);
        assert!(
            groups(&tree)
                .iter()
                .any(|group| group.opacity().get() == 0.5),
            "href selectors must see the original reference at every viewport"
        );
    }
    assert_eq!(
        resolver.calls.get(),
        1,
        "native conversion resolves the original data URL once, not during every paint"
    );
}

#[test]
fn active_svg_data_urls_obey_the_callers_resource_budget() {
    const RESOURCE_LIMIT: usize = 1;
    let xml = format!(
        "{SVG_OPEN}<image href='{}' width='20' height='10'/></svg>",
        png_data_url()
    );
    let resolver = FileResolver {
        max_resource_bytes: RESOURCE_LIMIT,
        ..Default::default()
    };
    let result = ImageCache::default().inline_svg(xml, &ResourceBase::default(), &resolver);
    assert!(
        matches!(
            result,
            Err(ImageError::Resource(ResourceError::TooLarge {
                limit: RESOURCE_LIMIT,
                ..
            }))
        ),
        "data references must pass through ResourceResolver before native decoding"
    );
}

#[test]
fn svg_css_component_depth_is_bounded_before_native_conversion() {
    const EXCESSIVE_DEPTH: usize = 256;
    for (depth, allowed) in [(EXCESSIVE_DEPTH - 1, true), (EXCESSIVE_DEPTH, false)] {
        let value = format!("{}x{}", "f(".repeat(depth), ")".repeat(depth));
        for body in [
            format!(
                "<style>g{{--value:{value};display:contents}}</style><g transform='translate(50)'><rect width='20' height='20'/></g>"
            ),
            format!(
                "<g style='--value:{value};display:contents' transform='translate(50)'><rect width='20' height='20'/></g>"
            ),
        ] {
            let xml = format!("{SVG_OPEN}{body}</svg>");
            let result = ImageCache::default().inline_svg(
                xml,
                &ResourceBase::default(),
                &FileResolver::default(),
            );
            assert_eq!(
                result.is_ok(),
                allowed,
                "CSS component depth {depth} must respect the established source bound"
            );
            if allowed {
                let image = result.unwrap();
                let Image::Svg(svg) = image.as_ref() else {
                    panic!("vector image expected")
                };
                assert_bounds(&svg.tree(SIZE).unwrap(), [0.0, 0.0, 20.0, 20.0]);
            } else {
                assert!(
                    matches!(
                        result,
                        Err(ImageError::Svg(
                            crate::svg::usvg::Error::CssComponentDepthLimit {
                                limit: EXCESSIVE_DEPTH
                            }
                        ))
                    ),
                    "the CSS depth limit must remain typed, not recover as malformed content"
                );
            }
        }
    }
}

#[test]
fn resized_svg_viewports_keep_authored_root_attributes_for_selectors() {
    let xml = format!(
        "{SVG_OPEN}<style>svg[width='200'] g{{display:contents}}svg[width='200'] image{{display:none}}</style><g transform='translate(50)'><rect x='10' y='20' width='30' height='40'/></g><image href='https://denied.example/unused.png' width='20' height='20'/></svg>"
    );
    let image = ImageCache::default()
        .inline_svg(xml, &ResourceBase::default(), &FileResolver::default())
        .unwrap();
    let Image::Svg(svg) = image.as_ref() else {
        panic!("vector image expected")
    };
    for size in [SIZE, [100.0, 100.0]] {
        let tree = svg.tree(size).unwrap();
        assert_eq!(
            [tree.size().width(), tree.size().height()],
            size,
            "the actual viewport must resize"
        );
        assert_bounds(&tree, [10.0, 20.0, 30.0, 40.0]);
    }
}

#[test]
fn svg_selectors_use_shared_attribute_structural_and_functional_matching() {
    for selector in [
        ":root > g > rect",
        "g[data-value^='sha'] > rect[data-value$='DOW' i]",
        "g[data-value*='ado'] > rect[data-value='SHADOW' s]",
        "g:has(> rect:not(.missing)) > :is(.target, .missing)",
        "g > :where(.target):nth-child(2 of .skip, .target)",
        "g > rect:nth-last-of-type(1)",
        r"g > .tar\67 et",
        "g:lang(fr) > rect",
    ] {
        eprintln!("checking SVG selector {selector}");
        let actual = tree(&format!(
            "<style>{selector}{{fill:#123456}}</style><g xml:lang='fr-CA' data-value='shadow'><defs class='skip'/><rect class='target' data-value='SHADOW' width='20' height='10' fill='blue'/></g>"
        ));
        assert_fill(&actual, [0x12, 0x34, 0x56]);
    }
}

#[test]
fn svg_selectors_keep_namespace_aliases_separate_from_xml_prefix_spelling() {
    let actual = tree(
        "<style>@namespace s 'http://www.w3.org/2000/svg';@namespace role 'urn:svg-test-role';s|g > s|rect[role|kind='target']{fill:#123456}</style><g xmlns:a='urn:svg-test-role'><rect a:kind='target' width='20' height='10' fill='blue'/></g>",
    );
    assert_fill(&actual, [0x12, 0x34, 0x56]);
}

#[test]
fn svg_selectors_preserve_xml_case_even_for_foreign_html_namespace_elements() {
    let actual = tree(
        "<style>@namespace h 'http://www.w3.org/1999/xhtml';g:has(> h|DIV[DATA-role='x']) > rect{fill:#123456}g:has(> h|div) > rect{fill:red}</style><g xmlns:h='http://www.w3.org/1999/xhtml'><h:DIV DATA-role='x'/><rect width='20' height='10' fill='blue'/></g>",
    );
    assert_fill(&actual, [0x12, 0x34, 0x56]);
}

#[test]
fn svg_namespace_declaration_attributes_are_local_including_redeclarations() {
    for declaration in ["", "xmlns:s='http://www.w3.org/2000/svg'"] {
        let selector = if declaration.is_empty() {
            ":root[ns|s] > g:not([ns|s]) > rect"
        } else {
            ":root[ns|s] > g[ns|s] > rect"
        };
        let actual = svg(&format!(
            "<svg xmlns='http://www.w3.org/2000/svg' xmlns:s='http://www.w3.org/2000/svg' width='200' height='100'><style>@namespace ns 'http://www.w3.org/2000/xmlns/';{selector}{{fill:#123456}}</style><g {declaration}><rect width='20' height='10' fill='blue'/></g></svg>"
        ));
        assert_fill(&actual, [0x12, 0x34, 0x56]);
    }
}

#[test]
fn svg_selector_list_recovery_and_functional_specificity_follow_the_shared_engine() {
    for (rules, expected) in [
        ("rect, missing|rect{fill:red}", [0, 0, 255]),
        (
            ":is(missing|rect,.target){fill:#123456}",
            [0x12, 0x34, 0x56],
        ),
        (
            "#target{fill:red}:where(#target){fill:green}g:has(> #target) > rect{fill:#123456}",
            [0x12, 0x34, 0x56],
        ),
        ("rect::before{fill:red}rect:hover{fill:red}", [0, 0, 255]),
    ] {
        eprintln!("checking SVG selector recovery and specificity: {rules}");
        let actual = tree(&format!(
            "<style>{rules}</style><g><rect id='target' class='target' width='20' height='10' fill='blue'/></g>"
        ));
        assert_fill(&actual, expected);
    }
}

#[test]
fn svg_namespace_bindings_are_local_to_each_stylesheet_and_its_header() {
    for sheets in [
        "<style>@namespace s 'http://www.w3.org/2000/svg';s|rect{fill:#123456}</style><style>s|rect{fill:red}</style>",
        "<style>@namespace s 'urn:wrong';@namespace s 'http://www.w3.org/2000/svg';s|rect{fill:#123456}</style>",
        "<style>@namespace '';rect{fill:red}*|rect{fill:#123456}</style>",
        "<style>rect{fill:#123456}@namespace s 'http://www.w3.org/2000/svg';s|rect{fill:red}</style>",
    ] {
        eprintln!("checking SVG namespace header and scope: {sheets}");
        let actual = tree(&format!(
            "{sheets}<rect width='20' height='10' fill='blue'/>"
        ));
        assert_fill(&actual, [0x12, 0x34, 0x56]);
    }
}

#[test]
fn svg_use_instances_match_their_own_ancestry_and_inherit_from_the_host() {
    let actual = tree(
        "<style>circle{stroke-opacity:.7}.special circle{stroke:green}use{stroke:purple;fill:orange}</style><g class='special' style='fill:blue'><circle id='source' cx='50' cy='50' r='40' stroke-width='20'/></g><use href='#source' x='100'/>",
    );
    let mut paths = paths(&actual);
    paths.sort_by(|a, b| {
        a.abs_bounding_box()
            .x()
            .total_cmp(&b.abs_bounding_box().x())
    });
    assert_eq!(paths.len(), 2);
    for (path, fill, stroke) in [
        (paths[0], [0, 0, 255], [0, 128, 0]),
        (paths[1], [255, 165, 0], [128, 0, 128]),
    ] {
        assert_eq!(paint_color(path.fill().unwrap().paint()), fill);
        let actual_stroke = path.stroke().unwrap();
        assert_eq!(paint_color(actual_stroke.paint()), stroke);
        assert!((actual_stroke.width().get() - 20.0).abs() < EPSILON);
        assert!((actual_stroke.opacity().get() - 0.7).abs() < EPSILON);
    }
}

#[test]
fn svg_use_instance_selectors_keep_only_cloned_ancestors_and_siblings() {
    for (selector, target) in [
        (".outside #target", "target"),
        (".before + #target", "target"),
        ("#target:nth-child(2)", "target"),
        ("#target:first-child", "branch"),
        ("#branch:has(> #target) > #target", "branch"),
        ("#target:last-child", "branch"),
        ("#target:only-child", "target"),
    ] {
        let expected = if matches!(
            selector,
            ".outside #target"
                | ".before + #target"
                | "#target:nth-child(2)"
                | "#target:first-child"
        ) {
            [0, 0, 255]
        } else {
            [0x12, 0x34, 0x56]
        };
        eprintln!("checking instance selector {selector}, referenced #{target}");
        let actual = tree(&format!(
            "<style>{selector}{{fill:#123456}}</style><defs class='outside'><g id='branch'><g class='before'/><rect id='target' width='20' height='10'/></g></defs><use href='#{target}' fill='blue'/>"
        ));
        assert_fill(&actual, expected);
    }
}

#[test]
fn svg_use_host_selectors_and_encapsulation_control_inherited_paint() {
    for (rules, inline, expected) in [
        (":host{fill:#123456}", "", [0x12, 0x34, 0x56]),
        (":host(.themed){fill:#123456}", "", [0x12, 0x34, 0x56]),
        (
            ":host-context(.theme) rect{fill:#123456}",
            "",
            [0x12, 0x34, 0x56],
        ),
        (":host > rect{fill:#123456}", "", [0x12, 0x34, 0x56]),
        ("use rect{fill:red}", "", [0, 0, 255]),
        ("use{fill:blue}:host{fill:red}", "", [0, 0, 255]),
        (
            "use{fill:blue!important}:host{fill:#123456!important}",
            "fill:red!important",
            [0x12, 0x34, 0x56],
        ),
        (
            ":host{fill:#123456!important}:host(.themed){fill:green!important}",
            "",
            [0, 128, 0],
        ),
    ] {
        eprintln!("checking host rules {rules}, inline {inline}");
        let actual = tree(&format!(
            "<style>{rules}</style><defs><rect id='shape' width='20' height='10'/></defs><g class='theme' fill='blue'><use class='themed' href='#shape' style='{inline}'/></g>"
        ));
        assert_fill(&actual, expected);
    }
}

#[test]
fn svg_nested_use_instances_keep_distinct_hosts_and_restore_the_outer_context() {
    let actual = tree(
        "<style>:host(.outer) > g > use{fill:green}:host(.inner) > rect{fill:#123456}use:has(rect){fill:red}</style><defs><rect id='leaf' width='20' height='10'/><g id='branch'><use class='inner' href='#leaf'/></g></defs><use class='outer' href='#branch' fill='blue'/><rect x='30' width='10' height='10' fill='purple'/>",
    );
    let mut paths = paths(&actual);
    paths.sort_by(|a, b| {
        a.abs_bounding_box()
            .x()
            .total_cmp(&b.abs_bounding_box().x())
    });
    assert_eq!(paths.len(), 2);
    for (path, expected) in [(paths[0], [0x12, 0x34, 0x56]), (paths[1], [128, 0, 128])] {
        assert_eq!(paint_color(path.fill().unwrap().paint()), expected);
    }
}

#[test]
fn svg_use_presentation_attributes_keep_the_outer_hosts_cascade_priority() {
    let actual = tree(
        "<style>:host{fill:red}</style><defs><rect id='shape' width='20' height='10'/></defs><use href='#shape' fill='blue'/>",
    );
    assert_fill(&actual, [0, 0, 255]);
}

#[test]
fn svg_use_text_descendants_match_inside_the_instance_context() {
    let actual = tree(
        "<style>.outside tspan{fill:red}:host(.themed) > text > tspan{fill:#123456}</style><defs class='outside'><text id='label' y='30'><tspan>X</tspan></text></defs><use class='themed' href='#label' fill='blue'/>",
    );
    assert_fill(&actual, [0x12, 0x34, 0x56]);
}

#[test]
fn svg_instance_depth_uses_the_svg_budget_instead_of_the_html_shadow_budget() {
    const INSTANCE_DEPTH: usize = 70;
    let mut body = String::from(
        "<style>:host(.themed){fill:#123456}</style><defs><rect id='source0' width='20' height='10'/>",
    );
    for index in 1..=INSTANCE_DEPTH {
        let _ = write!(
            body,
            "<use id='source{index}' href='#source{}'/>",
            index - 1
        );
    }
    let _ = write!(
        body,
        "</defs><use class='themed' href='#source{INSTANCE_DEPTH}'/>"
    );
    assert_fill(&tree(&body), [0x12, 0x34, 0x56]);
}

#[test]
fn svg_functional_selector_depth_uses_the_existing_source_budget() {
    const EXCESSIVE_DEPTH: usize = 256;
    for depth in [EXCESSIVE_DEPTH - 1, EXCESSIVE_DEPTH] {
        let selector = format!("{}rect{}", ":is(".repeat(depth), ")".repeat(depth));
        let xml = format!(
            "{SVG_OPEN}<style>{selector}{{fill:#123456}}</style><rect width='20' height='10' fill='blue'/></svg>"
        );
        let result = ImageCache::default().inline_svg(
            xml,
            &ResourceBase::default(),
            &FileResolver::default(),
        );
        if depth < EXCESSIVE_DEPTH {
            let image = result.expect("the accepted selector depth must remain usable");
            let Image::Svg(svg) = image.as_ref() else {
                panic!("vector image expected")
            };
            assert_fill(&svg.tree(SIZE).unwrap(), [0x12, 0x34, 0x56]);
        } else {
            assert!(
                matches!(
                    result,
                    Err(ImageError::Svg(
                        crate::svg::usvg::Error::CssComponentDepthLimit {
                            limit: EXCESSIVE_DEPTH
                        }
                    ))
                ),
                "excessive selector nesting must retain the typed source limit"
            );
        }
    }
}
