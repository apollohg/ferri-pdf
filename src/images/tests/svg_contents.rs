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

/// The raster and vector images `tree` draws.
fn image_count(tree: &crate::svg::usvg::Tree) -> usize {
    groups(tree)
        .into_iter()
        .flat_map(crate::svg::usvg::Group::children)
        .filter(|node| matches!(node, crate::svg::usvg::Node::Image(_)))
        .count()
}

/// `xml` loaded through `resolver`, with the image failures it reported.
fn svg_with_failures(
    xml: String,
    resolver: &dyn ResourceResolver,
) -> (crate::svg::usvg::Tree, Vec<crate::images::ImageFailure>) {
    let mut cache = ImageCache::default();
    let image = cache
        .inline_svg(xml, &ResourceBase::default(), resolver)
        .expect("an SVG whose resources fail still loads");
    let Image::Svg(svg) = image.as_ref() else {
        panic!("inline SVG must use the vector pipeline")
    };
    (
        svg.tree(SIZE).expect("SVG contents fixture must convert"),
        cache.take_failures(),
    )
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
fn unused_boxless_svg_resources_are_not_loaded_and_active_denials_are_reported() {
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
        let (tree, failures) = svg_with_failures(format!("{SVG_OPEN}{body}</svg>"), &policy);
        assert_eq!(
            image_count(&tree),
            0,
            "a denied image is in error and not rendered: {body}"
        );
        if active {
            assert!(
                matches!(
                    failures.as_slice(),
                    [failure] if matches!(failure.error, ImageError::Resource(ResourceError::Denied(_)))
                        && failure.reference == DENIED
                ),
                "an active SVG resource denial is reported once: {failures:?}"
            );
            assert_eq!(policy.requests.get(), 1);
        } else {
            assert!(
                failures.is_empty(),
                "unboxed or inactive SVG resources must not be requested: {failures:?}"
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
    let (tree, failures) = svg_with_failures(xml, &resolver);
    assert!(
        matches!(
            failures.as_slice(),
            [failure] if matches!(failure.error, ImageError::Resource(ResourceError::TooLarge {
                limit: RESOURCE_LIMIT,
                ..
            }))
        ),
        "data references must pass through ResourceResolver before native decoding: {failures:?}"
    );
    assert_eq!(
        image_count(&tree),
        0,
        "an image over the budget is in error and not rendered"
    );
}

#[test]
fn svg_css_component_depth_is_bounded_before_native_conversion() {
    const EXCESSIVE_DEPTH: usize = 256;
    for (depth, allowed) in [(EXCESSIVE_DEPTH - 1, true), (EXCESSIVE_DEPTH, false)] {
        for literal in ["x", r"\61", r#""var(""#] {
            let value = format!("{}{literal}{}", "f(".repeat(depth), ")".repeat(depth));
            for body in [
                format!(
                    "<style>g{{--value:{value};display:contents}}</style><g transform='translate(50)'><rect width='20' height='20'/></g>"
                ),
                format!(
                    "<g style='--value:{value};display:contents' transform='translate(50)'><rect width='20' height='20'/></g>"
                ),
                format!(
                    "<style>g{{--value:{value}}}</style><g style='--value:var(--missing,revert-layer);display:contents' transform='translate(50)'><rect width='20' height='20'/></g>"
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
                    "CSS component depth {depth} around {literal:?} must respect the established source bound"
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

#[test]
fn svg_zoom_products_reject_overflow_and_underflow_before_geometry() {
    for zoom in ["1e30", "1e-30"] {
        let xml = format!(
            "{SVG_OPEN}<g style='zoom:{zoom}'><g style='zoom:{zoom}'><rect width='1' height='1'/></g></g></svg>"
        );
        let result = crate::svg::usvg::Tree::from_str(&xml, &crate::svg::usvg::Options::default());
        assert!(
            matches!(result, Err(crate::svg::usvg::Error::Style(_))),
            "nested zoom {zoom} must report a style error, not silently drop geometry: {result:?}"
        );
        let result = ImageCache::default().inline_svg(
            xml,
            &ResourceBase::default(),
            &FileResolver::default(),
        );
        assert!(
            matches!(
                result,
                Err(ImageError::Svg(crate::svg::usvg::Error::Style(_)))
            ),
            "image loading must preserve the invalid effective zoom error: {result:?}"
        );
    }
    let actual =
        tree("<g style='zoom:1e10'><g style='zoom:1e-10'><rect width='20' height='10'/></g></g>");
    assert_bounds(&actual, [0.0, 0.0, 20.0, 10.0]);
}

#[test]
fn svg_font_relative_lengths_use_the_selected_face() {
    const FONT_SIZE: f32 = 10.0;
    const LENGTH: f32 = 8.0;
    for (family, bytes) in [
        (
            "DejaVu Sans",
            include_bytes!("../../../assets/fonts/DejaVuSans.ttf").as_slice(),
        ),
        (
            "DejaVu Serif",
            include_bytes!("../../../assets/fonts/DejaVuSerif.ttf").as_slice(),
        ),
    ] {
        let face = ttf_parser::Face::parse(bytes, 0).unwrap();
        let units = f32::from(face.units_per_em());
        let glyph_top = |ch| {
            f32::from(
                face.glyph_bounding_box(face.glyph_index(ch).unwrap())
                    .unwrap()
                    .y_max,
            ) / units
        };
        let measures = [
            ("ex", glyph_top('x')),
            ("cap", glyph_top('H')),
            (
                "ch",
                f32::from(
                    face.glyph_hor_advance(face.glyph_index('0').unwrap())
                        .unwrap(),
                ) / units,
            ),
            (
                "lh",
                (f32::from(face.ascender()) - f32::from(face.descender())
                    + f32::from(face.line_gap()))
                    / units,
            ),
        ];
        for zoom in [1.0, 2.0] {
            for (unit, em) in measures {
                for width in [format!("{LENGTH}{unit}"), format!("calc({LENGTH}{unit})")] {
                    let tree = tree(&format!(
                        "<rect font-family='{family}' font-size='{FONT_SIZE}' style='zoom:{zoom}' width='{width}' height='10'/>"
                    ));
                    let actual = tree.root().abs_bounding_box().width();
                    let expected = LENGTH * FONT_SIZE * zoom * em;
                    assert!(
                        (actual - expected).abs() < EPSILON,
                        "{family}, {width}, zoom {zoom}: expected selected-face width {expected}, got {actual}"
                    );
                }
            }
        }
    }
}

#[test]
fn svg_font_relative_sizes_and_root_units_keep_their_font_context() {
    const FONT_SIZE: f32 = 10.0;
    const ZOOM: f32 = 2.0;
    let face =
        ttf_parser::Face::parse(include_bytes!("../../../assets/fonts/DejaVuSans.ttf"), 0).unwrap();
    let height = f32::from(
        face.glyph_bounding_box(face.glyph_index('x').unwrap())
            .unwrap()
            .y_max,
    ) / f32::from(face.units_per_em());
    for (size, width, expected) in [
        ("20px", "8rex", 8.0 * FONT_SIZE * ZOOM * height),
        ("20px", "calc(8rex)", 8.0 * FONT_SIZE * ZOOM * height),
        ("2ex", "4em", 8.0 * FONT_SIZE * ZOOM * height),
        ("calc(2ex)", "4em", 8.0 * FONT_SIZE * ZOOM * height),
    ] {
        let tree = svg(&format!(
            "<svg xmlns='http://www.w3.org/2000/svg' width='200' height='100' font-family='DejaVu Sans' font-size='{FONT_SIZE}'><rect style='font-family:DejaVu Serif;font-size:{size};zoom:{ZOOM}' width='{width}' height='10'/></svg>"
        ));
        let actual = tree.root().abs_bounding_box().width();
        assert!(
            (actual - expected).abs() < EPSILON,
            "font-size {size}, width {width}: parent/root Sans metrics must survive the child's Serif family; expected {expected}, got {actual}"
        );
    }
}

#[test]
fn svg_natural_dimensions_use_cascaded_lengths_and_root_font_metrics() {
    let cases = [
        (
            "calculated font size",
            "width='4em' height='2em' style='font-size:calc(5px + 5px);zoom:2'",
            "",
            [Some(80.0), Some(40.0), Some(2.0)],
        ),
        (
            "root font calculation uses initial rem",
            "width='2em' height='1em' style='font-size:calc(1rem + 4px);zoom:2'",
            "",
            [Some(80.0), Some(40.0), Some(2.0)],
        ),
        (
            "calculated font dimensions",
            "style='width:calc(2em + 20px);height:calc(1em + 10px);font-size:10px;zoom:2'",
            "",
            [Some(80.0), Some(40.0), Some(2.0)],
        ),
        (
            "calculated percentage dimensions",
            "style='width:calc(50% + 20px);height:calc(25% + 10px);zoom:2' viewBox='0 0 40 20'",
            "",
            [None, None, Some(2.0)],
        ),
        (
            "font-relative",
            "width='4em' height='2em' style='font-size:10px;zoom:2'",
            "",
            [Some(80.0), Some(40.0), Some(2.0)],
        ),
        (
            "stylesheet dimensions",
            "width='10' height='10'",
            "svg{width:40px;height:20px;zoom:2}",
            [Some(80.0), Some(40.0), Some(2.0)],
        ),
        (
            "variables and font",
            "width='10' height='10'",
            "svg{--unit:4em;width:var(--unit);height:2em;font-size:10px;zoom:200%}",
            [Some(80.0), Some(40.0), Some(2.0)],
        ),
        (
            "CSS percentages replace attributes",
            "width='80' height='40' viewBox='0 0 40 20'",
            "svg{width:50%;height:25%;zoom:2}",
            [None, None, Some(2.0)],
        ),
        (
            "CSS auto replaces attributes",
            "width='80' height='40' viewBox='0 0 40 20'",
            "svg{width:auto;height:auto;zoom:2}",
            [None, None, Some(2.0)],
        ),
        (
            "ratio only",
            "viewBox='0 0 40 20' style='zoom:2'",
            "",
            [None, None, Some(2.0)],
        ),
        (
            "percentage",
            "width='50%' height='25%' viewBox='0 0 40 20' style='zoom:2'",
            "",
            [None, None, Some(2.0)],
        ),
        (
            "one dimension",
            "width='4em' viewBox='0 0 40 20' style='font-size:10px;zoom:2'",
            "",
            [Some(80.0), None, Some(2.0)],
        ),
    ];
    for (name, attributes, css, expected) in cases {
        let xml = format!(
            "<svg xmlns='http://www.w3.org/2000/svg' {attributes}><style>{css}</style><rect width='100%' height='100%' fill='green'/></svg>"
        );
        let image = ImageCache::default()
            .inline_svg(xml, &ResourceBase::default(), &FileResolver::default())
            .unwrap_or_else(|error| panic!("{name}: {error}"));
        let actual = image.natural_size();
        assert_eq!(
            [actual.width, actual.height, actual.ratio],
            expected,
            "{name}: intrinsic dimensions must use the same styled lengths as rendering"
        );
    }
}

#[test]
fn svg_calculated_root_dimensions_resolve_against_the_used_viewport() {
    for (attributes, expected) in [
        (
            "style='width:calc(2em + 20px);height:calc(1em + 10px);font-size:10px;zoom:2'",
            [80.0, 40.0],
        ),
        (
            "viewBox='0 0 40 20' style='width:calc(50% + 20px);height:calc(25% + 10px);zoom:2'",
            [80.0, 30.0],
        ),
    ] {
        let xml = format!(
            "<svg xmlns='http://www.w3.org/2000/svg' {attributes}><rect width='100%' height='100%'/></svg>"
        );
        let tree = crate::svg::usvg::Tree::from_str(&xml, &crate::svg::usvg::Options::default())
            .unwrap_or_else(|error| panic!("{attributes}: {error}"));
        assert_eq!(
            [tree.size().width(), tree.size().height()],
            expected,
            "{attributes}: native SVG conversion must use the calculated dimensions"
        );
    }
}

#[test]
fn svg_calculated_length_overflow_is_reported_instead_of_dropping_geometry() {
    for body in [
        "<rect style='zoom:100' width='calc(1e38px)' height='10'/>",
        "<rect style='zoom:100' width='10' height='calc(1e38px)'/>",
    ] {
        let xml = format!("{SVG_OPEN}{body}</svg>");
        let result = crate::svg::usvg::Tree::from_str(&xml, &crate::svg::usvg::Options::default());
        assert!(
            matches!(result, Err(crate::svg::usvg::Error::Style(_))),
            "{body}: zoomed calculation overflow must propagate a style error: {result:?}"
        );
    }
}

#[test]
fn svg_calculated_physical_units_honor_custom_dpi_without_scaling_pixels() {
    for dpi in [72.0, 96.0, 144.0] {
        for zoom in [1.0, 2.0] {
            for length in ["1in", "72pt", "6pc", "2.54cm", "25.4mm", "101.6q"] {
                for (width, expected) in [
                    (length.to_owned(), dpi * zoom),
                    (format!("calc({length})"), dpi * zoom),
                    (format!("calc({length} + 4px)"), (dpi + 4.0) * zoom),
                ] {
                    let xml = format!(
                        "<svg xmlns='http://www.w3.org/2000/svg' width='{width}' height='10px' style='zoom:{zoom}'><rect width='100%' height='100%'/></svg>"
                    );
                    let options = crate::svg::usvg::Options {
                        dpi,
                        ..Default::default()
                    };
                    let tree =
                        crate::svg::usvg::Tree::from_str(&xml, &options).unwrap_or_else(|error| {
                            panic!("dpi={dpi}, zoom={zoom}, width={width}: {error}")
                        });
                    assert!(
                        (tree.size().width() - expected).abs() < EPSILON,
                        "dpi={dpi}, zoom={zoom}, width={width}: expected {expected}, got {}",
                        tree.size().width()
                    );
                    assert_eq!(
                        tree.size().height(),
                        10.0 * zoom,
                        "px must remain independent of DPI"
                    );
                }
            }
        }
    }
}

#[test]
fn svg_font_and_transform_calculations_share_the_custom_dpi() {
    for dpi in [72.0, 96.0, 144.0] {
        for zoom in [1.0, 2.0] {
            let options = crate::svg::usvg::Options {
                dpi,
                ..Default::default()
            };
            let expected = (dpi + 4.0) * zoom;
            let xml = format!(
                "<svg xmlns='http://www.w3.org/2000/svg' width='1em' height='10px' style='font-size:calc(1in + 4px);zoom:{zoom}'><rect width='100%' height='100%'/></svg>"
            );
            let font =
                crate::svg::usvg::Tree::from_str(&xml, &options).expect("calculated SVG font size");
            assert!(
                (font.size().width() - expected).abs() < EPSILON,
                "dpi={dpi}, zoom={zoom}: font-relative width {}, expected {expected}",
                font.size().width()
            );
            let xml = format!(
                "<svg xmlns='http://www.w3.org/2000/svg' width='1000' height='1000'><g style='translate:calc(1in + 4px) 0;zoom:{zoom}'><rect width='10' height='10'/></g></svg>"
            );
            let transformed = crate::svg::usvg::Tree::from_str(&xml, &options)
                .expect("calculated SVG translation");
            assert_bounds(&transformed, [expected, 0.0, 10.0 * zoom, 10.0 * zoom]);
        }
    }
}

#[test]
fn svg_dash_lists_validate_the_whole_value_and_clamp_calculated_ranges() {
    for (source, expected) in [
        ("calc(2px + 2px), 2", Some(vec![4.0, 2.0])),
        ("calc(-2px) 2px", Some(vec![0.0, 2.0])),
        ("4 invalid 2", None),
        ("4,,2", None),
        ("4 -2", None),
        ("none", None),
    ] {
        let actual = tree(&format!(
            "<path d='M0 10H100' stroke='green' stroke-dasharray='{source}'/>"
        ));
        let paths = paths(&actual);
        let stroke = paths[0]
            .stroke()
            .expect("invalid dashes still retain the stroke");
        assert_eq!(
            stroke.dasharray(),
            expected.as_deref(),
            "{source}: a malformed dash list must not retain a valid prefix"
        );
    }
}

#[test]
fn svg_calculated_dash_lists_share_custom_dpi_and_percentage_bases() {
    for dpi in [72.0, 96.0, 144.0] {
        let options = crate::svg::usvg::Options {
            dpi,
            ..Default::default()
        };
        let xml = "<svg xmlns='http://www.w3.org/2000/svg' width='200' height='200'><g stroke='green' stroke-dasharray='calc(.1in + 2px) 10%'><path style='zoom:2' d='M0 10H80'/></g></svg>";
        let actual =
            crate::svg::usvg::Tree::from_str(xml, &options).expect("calculated SVG dash list");
        let paths = paths(&actual);
        let dashes = paths[0]
            .stroke()
            .expect("stroke")
            .dasharray()
            .expect("dash list");
        let expected = [(dpi * 0.1 + 2.0) * 2.0, 20.0];
        assert_eq!(dashes.len(), expected.len());
        for (value, expected) in dashes.iter().zip(expected) {
            assert!(
                (value - expected).abs() < EPSILON,
                "dpi={dpi}: expected dash {expected}, got {value}"
            );
        }
    }
}

#[test]
fn svg_text_glyphs_scale_by_the_size_adjust_descriptor() {
    let width = |descriptors: &str| {
        let sheet = crate::style::Stylesheet::parse(&[crate::style::StylesheetSource {
            text: format!(
                "@font-face{{font-family:Adjusted;src:local('DejaVu Sans');{descriptors}}}"
            ),
            base: ResourceBase::default(),
        }])
        .unwrap();
        let image = ImageCache::with_fonts(sheet.fonts)
            .inline_svg(
                format!("{SVG_OPEN}<text x='10' y='60' font-family='Adjusted' font-size='20'>HHH</text></svg>"),
                &ResourceBase::default(),
                &FileResolver::default(),
            )
            .unwrap();
        let Image::Svg(svg) = image.as_ref() else {
            panic!("inline SVG must use the vector pipeline")
        };
        svg.tree(SIZE).unwrap().root().abs_bounding_box().width()
    };
    let (whole, half) = (width(""), width("size-adjust:50%"));
    assert!(
        (half - whole / 2.0).abs() < EPSILON,
        "size-adjust:50% must halve the glyph run: {half} against {whole}"
    );
}

#[test]
fn svg_text_line_metrics_take_the_metric_override_descriptors() {
    let height = |descriptors: &str| {
        let sheet = crate::style::Stylesheet::parse(&[crate::style::StylesheetSource {
            text: format!(
                "@font-face{{font-family:Overridden;src:local('DejaVu Sans');{descriptors}}}"
            ),
            base: ResourceBase::default(),
        }])
        .unwrap();
        let image = ImageCache::with_fonts(sheet.fonts)
            .inline_svg(
                format!(
                    "{SVG_OPEN}<text x='10' y='40' font-family='Overridden' font-size='20' \
                     style='white-space:pre'>H\nH</text></svg>"
                ),
                &ResourceBase::default(),
                &FileResolver::default(),
            )
            .unwrap();
        let Image::Svg(svg) = image.as_ref() else {
            panic!("inline SVG must use the vector pipeline")
        };
        svg.tree(SIZE).unwrap().root().abs_bounding_box().height()
    };
    let plain = height("");
    let overridden = height("ascent-override:100%;descent-override:0%;line-gap-override:50%");
    // Each line's box spans its overridden ascent and descent, one em, and lines stand one
    // and a half em apart (CSS Fonts 4 §4.11).
    assert!(
        (overridden - 50.0).abs() < EPSILON,
        "two overridden lines, 1.5em apart and 1em tall, span 50 units: {overridden} \
         (the font's own metrics give {plain})"
    );
}

const RED: [u8; 3] = [255, 0, 0];
const GREEN: [u8; 3] = [0, 255, 0];
const BLUE: [u8; 3] = [0, 0, 255];

/// A laid out run of SVG text: its characters, font size, fill, and first glyph's origin.
#[derive(Debug)]
struct Run {
    text: String,
    size: f32,
    fill: [u8; 3],
    origin: [f32; 2],
}

/// The runs of the tree's text in layout order, adjacent spans of one size and fill on one
/// baseline merged.
fn runs(tree: &crate::svg::usvg::Tree) -> Vec<Run> {
    let mut runs: Vec<Run> = Vec::new();
    for group in groups(tree) {
        for node in group.children() {
            let crate::svg::usvg::Node::Text(text) = node else {
                continue;
            };
            for span in text.layouted() {
                let Some(first) = span.positioned_glyphs.first() else {
                    continue;
                };
                let transform = first.transform();
                let run = Run {
                    text: span
                        .positioned_glyphs
                        .iter()
                        .map(|glyph| glyph.text.as_str())
                        .collect(),
                    size: span.font_size.get(),
                    fill: paint_color(span.fill.as_ref().expect("filled text").paint()),
                    origin: [transform.tx, transform.ty],
                };
                match runs.last_mut() {
                    Some(last)
                        if last.size == run.size
                            && last.fill == run.fill
                            && (last.origin[1] - run.origin[1]).abs() < EPSILON =>
                    {
                        last.text.push_str(&run.text);
                    }
                    _ => runs.push(run),
                }
            }
        }
    }
    runs
}

fn summary(runs: &[Run]) -> Vec<(&str, f32, [u8; 3])> {
    runs.iter()
        .map(|run| (run.text.as_str(), run.size, run.fill))
        .collect()
}

#[test]
fn svg_text_first_letter_takes_its_punctuation_and_sub_pseudo_elements() {
    let runs = runs(&tree(
        "<style>text::first-letter{fill:#f00;font-size:40px}\
         text::first-letter::suffix{fill:#0f0}</style>\
         <text x='10' y='60' font-size='20' fill='#00f'>(A) rest</text>",
    ));
    assert_eq!(
        summary(&runs),
        [("(A", 40.0, RED), (")", 40.0, GREEN), (" rest", 20.0, BLUE)],
        "CSS Pseudo-Elements 4 §2.2.1: the first letter takes the opening punctuation before \
         it and its ::suffix the closing punctuation after it: {runs:#?}"
    );
}

#[test]
fn svg_text_first_line_restyles_its_contents_up_to_the_first_preserved_break() {
    let body = |rule: &str| {
        format!(
            "<style>{rule}</style><text x='10' y='20' font-size='10' fill='#00f' \
             style='white-space:pre'>One <tspan>two</tspan> <tspan fill='#0f0'>own</tspan>\n\
             Three</text>"
        )
    };
    let styled = runs(&tree(&body(
        "text::first-line{fill:#f00;font-size:20px;writing-mode:vertical-rl}",
    )));
    assert_eq!(
        summary(&styled),
        [
            ("One two ", 20.0, RED),
            ("own", 20.0, GREEN),
            ("Three", 10.0, BLUE)
        ],
        "the first line's contents inherit through ::first-line, which leaves the writing mode \
         and the line after the preserved break alone: {styled:#?}"
    );
    let plain = runs(&tree(&body("")));
    let gap = |runs: &[Run]| runs.last().unwrap().origin[1] - runs[0].origin[1];
    assert!(
        gap(&styled) > gap(&plain),
        "the larger first line pushes the next one down: {} against {}",
        gap(&styled),
        gap(&plain)
    );
}

#[test]
fn svg_text_root_inline_box_takes_the_first_line_font_on_that_line() {
    // Only 10px text sits on the first line, but its line box holds the strut of the root
    // inline box, which inherits the 40px font from ::first-line (CSS Pseudo-Elements 4 §2.1.1).
    let gap = |rule: &str| {
        let runs = runs(&tree(&format!(
            "<style>{rule}</style><text x='10' y='20' font-size='10' style='white-space:pre'>\
             <tspan font-size='10'>a</tspan>\nb</text>"
        )));
        assert_eq!(runs.len(), 2, "two lines: {runs:#?}");
        runs[1].origin[1] - runs[0].origin[1]
    };
    let (styled, plain) = (gap("text::first-line{font-size:40px}"), gap(""));
    assert!(
        styled > plain + 5.0,
        "a 40px strut on the first line must lower the second line well below the 10px one: \
         {styled} against {plain}"
    );
}

#[test]
fn wrapped_svg_text_breaks_its_first_line_as_the_first_line_styles_it() {
    let words = "aa bb cc dd ee ff gg hh ii jj kk ll";
    let runs = runs(&tree(&format!(
        "<style>text::first-line{{font-size:20px;fill:#f00}}</style>\
         <text x='10' y='20' font-size='10' fill='#00f' style='inline-size:70px'>{words}</text>"
    )));
    assert!(
        runs.len() >= 3,
        "the text wraps onto several lines: {runs:#?}"
    );
    assert_eq!(
        (runs[0].size, runs[0].fill),
        (20.0, RED),
        "the first line is set as ::first-line styles it: {runs:#?}"
    );
    assert!(
        runs[1..]
            .iter()
            .all(|run| (run.size, run.fill) == (10.0, BLUE)),
        "the lines after the first keep the text's own style: {runs:#?}"
    );
    let count = |run: &Run| run.text.split_whitespace().count();
    assert!(
        count(&runs[0]) < count(&runs[1]),
        "the first line breaks where its larger font fills the area, holding fewer words: \
         {runs:#?}"
    );
    let shown: Vec<_> = runs
        .iter()
        .flat_map(|run| run.text.split_whitespace())
        .collect();
    assert_eq!(
        shown,
        words.split_whitespace().collect::<Vec<_>>(),
        "every word shows once, in order"
    );
}

#[test]
fn svg_typographic_pseudo_elements_reach_only_rendered_text_of_text_elements() {
    for (body, expected) in [
        (
            "<style>::first-letter{fill:#f00}</style><rect width='5' height='5'/>\
             <text x='10' y='60' fill='#00f'><tspan display='none'>X</tspan>Yes</text>",
            vec![("Y", RED), ("es", BLUE)],
        ),
        (
            "<style>g::first-line{fill:#f00}text::first-line{}</style>\
             <g><text x='10' y='60' fill='#00f'>Yes</text></g>",
            vec![("Yes", BLUE)],
        ),
        (
            "<style>text::first-line{fill:#f00}</style><defs><path id='p' d='M10 60 H190'/>\
             </defs><text fill='#00f'><textPath href='#p'>On path</textPath></text>",
            vec![("On path", RED)],
        ),
    ] {
        let runs = runs(&tree(body));
        let actual: Vec<_> = runs
            .iter()
            .map(|run| (run.text.as_str(), run.fill))
            .collect();
        assert_eq!(actual, expected, "`{body}` lays out {runs:#?}");
    }
}

/// `body` loaded as inline SVG content, with the resources and values of the limits that
/// changed it.
fn limited_svg(
    body: &str,
    resolver: &dyn ResourceResolver,
) -> (crate::svg::usvg::Tree, Vec<(&'static str, usize)>) {
    let mut cache = ImageCache::default();
    let image = cache
        .inline_svg(
            format!("{SVG_OPEN}{body}</svg>"),
            &ResourceBase::default(),
            resolver,
        )
        .expect("limited SVG content renders rather than failing");
    let Image::Svg(svg) = image.as_ref() else {
        panic!("vector image expected")
    };
    let limits = cache
        .take_limited()
        .into_iter()
        .map(|diagnostic| match diagnostic {
            crate::document::DocumentDiagnostic::FormattingLimit {
                resource, limit, ..
            } => (resource, limit),
            other => panic!("SVG limits are formatting limits: {other:?}"),
        })
        .collect();
    (svg.tree(SIZE).unwrap(), limits)
}

#[test]
fn svg_elements_past_the_depth_limit_are_placed_beside_their_parents() {
    use crate::svg::usvg::MAX_SVG_DEPTH;
    // The root element is at depth zero, so the rectangle inside `groups` groups is at depth
    // `groups + 1`; past the limit the innermost group keeps no element and the rectangle
    // is placed beside it, outside its translation.
    for (groups, limited, x) in [
        (MAX_SVG_DEPTH - 1, Vec::new(), MAX_SVG_DEPTH - 1),
        (
            MAX_SVG_DEPTH,
            vec![(crate::images::SVG_NESTING_RESOURCE, MAX_SVG_DEPTH)],
            MAX_SVG_DEPTH - 1,
        ),
    ] {
        let body = format!(
            "{}<rect width='1' height='1'/>{}",
            "<g transform='translate(1)'>".repeat(groups),
            "</g>".repeat(groups)
        );
        let (tree, limits) = limited_svg(&body, &FileResolver::default());
        assert_eq!(limits, limited, "limits of {groups} nested groups");
        let paths = paths(&tree);
        assert_eq!(
            paths.len(),
            1,
            "the rectangle of {groups} nested groups renders"
        );
        let bounds = paths[0].abs_bounding_box();
        assert!(
            (bounds.x() - x as f32).abs() <= EPSILON,
            "the rectangle of {groups} nested groups is translated to {x}: {bounds:?}"
        );
    }
}

#[test]
fn svg_text_past_the_depth_limit_keeps_its_characters_in_order() {
    use crate::svg::usvg::MAX_SVG_DEPTH;
    // `text` is at depth one; its innermost `tspan` is at depth `spans + 1`.
    for (spans, limited) in [
        (MAX_SVG_DEPTH - 1, Vec::new()),
        (
            MAX_SVG_DEPTH,
            vec![(crate::images::SVG_NESTING_RESOURCE, MAX_SVG_DEPTH)],
        ),
        (
            MAX_SVG_DEPTH * 8,
            vec![(crate::images::SVG_NESTING_RESOURCE, MAX_SVG_DEPTH)],
        ),
    ] {
        let body = format!(
            "<text y='20'>{}x{}<tspan>y</tspan></text>",
            "<tspan>".repeat(spans),
            "</tspan>".repeat(spans)
        );
        let (tree, limits) = limited_svg(&body, &FileResolver::default());
        assert_eq!(limits, limited, "limits of {spans} nested spans");
        let text: String = groups(&tree)
            .into_iter()
            .flat_map(crate::svg::usvg::Group::children)
            .filter_map(|node| match node {
                crate::svg::usvg::Node::Text(text) => Some(text),
                _ => None,
            })
            .flat_map(|text| text.chunks())
            .map(|chunk| chunk.text.as_str())
            .collect();
        assert_eq!(text, "xy", "characters of {spans} nested spans");
    }
}

#[test]
fn svg_references_past_the_reference_depth_are_omitted() {
    use crate::svg::usvg::MAX_REFERENCE_DEPTH;
    // A chain of masks, each masking a rectangle in the last's content with the next.
    let chain = |masks: usize| {
        let mut body = String::new();
        for index in 0..masks {
            let next = if index + 1 < masks {
                format!(" mask='url(#m{})'", index + 1)
            } else {
                String::new()
            };
            let _ = write!(
                body,
                "<mask id='m{index}' maskUnits='userSpaceOnUse'><rect width='20' height='10' fill='white'{next}/></mask>"
            );
        }
        body + "<rect width='20' height='10' mask='url(#m0)'/>"
    };
    let (tree, limits) = limited_svg(&chain(MAX_REFERENCE_DEPTH), &FileResolver::default());
    assert!(
        limits.is_empty(),
        "the reference depth itself converts: {limits:?}"
    );
    assert_bounds(&tree, [0.0, 0.0, 20.0, 10.0]);
    // One further mask is omitted as an invalid reference is, which leaves each mask that
    // holds it empty, and so invalid, up to the rectangle.
    let (tree, limits) = limited_svg(&chain(MAX_REFERENCE_DEPTH + 1), &FileResolver::default());
    assert_eq!(
        limits,
        [(crate::images::SVG_REFERENCE_RESOURCE, MAX_REFERENCE_DEPTH)]
    );
    assert!(
        paths(&tree).is_empty(),
        "the masked rectangle must not render with an omitted mask"
    );
}

#[test]
fn svg_images_drawing_svg_images_nest_as_references() {
    use crate::svg::usvg::MAX_REFERENCE_DEPTH;
    /// Serves `n.svg` as a document drawing `n+1.svg`, so that the images nest without end.
    struct Chain {
        requests: Cell<usize>,
    }
    impl ResourceResolver for Chain {
        fn resolve(&self, reference: &str, base: &ResourceBase) -> Result<Resource, ResourceError> {
            self.requests.set(self.requests.get() + 1);
            let level: usize = reference
                .trim_end_matches(".svg")
                .parse()
                .map_err(|_| ResourceError::Denied(base.resolve(reference).unwrap()))?;
            Ok(Resource {
                bytes: format!(
                    "{SVG_OPEN}<rect width='1' height='1'/><image href='{}.svg' width='200' height='100'/></svg>",
                    level + 1
                )
                .into_bytes(),
                base: base.clone(),
            })
        }
    }
    let resolver = Chain {
        requests: Cell::new(0),
    };
    let (tree, limits) = limited_svg("<image href='1.svg' width='200' height='100'/>", &resolver);
    assert_eq!(
        resolver.requests.get(),
        MAX_REFERENCE_DEPTH,
        "each image converts as one further reference"
    );
    assert_eq!(
        limits,
        [(crate::images::SVG_REFERENCE_RESOURCE, MAX_REFERENCE_DEPTH)],
        "the document whose image would nest past the reference depth reports it once"
    );
    // Each document draws the next, down to the one whose image is omitted.
    let mut drawn = 0;
    let mut current = tree;
    while let Some(nested) = groups(&current)
        .into_iter()
        .flat_map(crate::svg::usvg::Group::children)
        .find_map(|node| match node {
            crate::svg::usvg::Node::Image(image) => match image.kind() {
                crate::svg::usvg::ImageKind::SVG(nested) => Some(nested.clone()),
                _ => None,
            },
            _ => None,
        })
    {
        drawn += 1;
        current = nested;
    }
    assert_eq!(drawn, MAX_REFERENCE_DEPTH, "nested SVG images drawn");
}
