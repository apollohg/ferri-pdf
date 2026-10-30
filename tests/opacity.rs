mod common;
use common::{PROFILE, local_options};
use ferri_pdf::{DisplayItem, LayoutTrace, render_html};

fn trace(css: &str, body: &str) -> LayoutTrace {
    let html = format!("<style>{PROFILE}{css}</style>{body}");
    render_html(&html, &local_options())
        .unwrap()
        .layout
        .unwrap()
}

/// Each painted item with the effects enclosing it, outermost first: `O<alpha>` opacity and
/// `B<mode>` blend groups, `C` clips and `T` transforms. Boxes are named by their background, text by its line text.
/// Backdrop Roots only bound backdrop filters and are left out.
fn effect_stacks(trace: &LayoutTrace, page: usize) -> Vec<(String, Vec<String>)> {
    const BACKDROP_ROOT: &str = "R";
    let mut stack: Vec<String> = Vec::new();
    let mut painted = Vec::new();
    for item in &trace.display_lists[page] {
        let stack_here = || -> Vec<String> {
            stack
                .iter()
                .filter(|effect| *effect != BACKDROP_ROOT)
                .cloned()
                .collect()
        };
        match item {
            DisplayItem::PushBackdropRoot => stack.push(BACKDROP_ROOT.into()),
            DisplayItem::Backdrop(_) => painted.push(("backdrop".into(), stack_here())),
            DisplayItem::PushOpacity(alpha) => stack.push(format!("O{alpha}")),
            DisplayItem::PushBlend(mode) => stack.push(format!("B{mode:?}")),
            DisplayItem::PushFilter(filter) => stack.push(format!("F{:?}", filter.functions)),
            DisplayItem::PushMask(mask) => stack.push(format!("M{}", mask.fragments.len())),
            DisplayItem::PushClip(_) => stack.push("C".into()),
            DisplayItem::PushTransform(_) => stack.push("T".into()),
            DisplayItem::PushProjection(_) => stack.push("P".into()),
            DisplayItem::Pop => {
                stack.pop().expect("every pop closes a push");
            }
            DisplayItem::Box(paint) => {
                let [r, g, b] = paint.decoration.background.rgb;
                painted.push((format!("box #{r:02x}{g:02x}{b:02x}"), stack_here()));
            }
            DisplayItem::Text { line } => painted.push((
                format!("text {}", trace.pages[page][*line].text.trim()),
                stack_here(),
            )),
            DisplayItem::Frame(_) => painted.push(("flow frame".into(), stack_here())),
            DisplayItem::Scene(_) => painted.push(("scene".into(), stack_here())),
            DisplayItem::Image(_) => painted.push(("image".into(), stack_here())),
            DisplayItem::TextShadow(_) => painted.push(("text shadow".into(), stack_here())),
            DisplayItem::Decoration(decoration) => {
                painted.push((format!("{:?} decoration", decoration.line), stack_here()));
            }
        }
    }
    assert!(stack.is_empty(), "unbalanced effects: {stack:?} left open");
    painted
}

#[test]
fn blending_boxes_isolate_their_stacking_context_and_nest_inside_its_group() {
    let trace = trace(
        ".ctx{transform:translateX(1pt);background:#ff0000}.b{mix-blend-mode:screen;opacity:.5;background:#00ff00}\
         span{mix-blend-mode:multiply;background:#0000ff}.free{mix-blend-mode:darken;background:#ffff00}",
        "<div class='ctx'><div class='b'>Both</div><p>In <span>Span</span></p></div><div class='free'>Free</div>",
    );
    let painted = effect_stacks(&trace, 0);
    let expect = |name: &str, effects: &[&str]| {
        assert_eq!(
            stack_of(&painted, name),
            effects,
            "{name} effects in: {painted:#?}"
        );
    };
    expect("box #ff0000", &["BNormal", "T"]);
    expect("box #00ff00", &["BNormal", "BScreen", "O0.5", "T"]);
    expect("text Both", &["BNormal", "BScreen", "O0.5", "T"]);
    expect("box #0000ff", &["BNormal", "BMultiply", "T"]);
    for effects in [&["BNormal", "BMultiply", "T"][..], &["BNormal", "T"]] {
        assert!(
            painted
                .iter()
                .any(|(name, stack)| name == "text In Span" && stack == effects),
            "the line's runs split by the blending span: {effects:?} in {painted:#?}"
        );
    }
    expect("box #ffff00", &["BDarken"]);
}

#[test]
fn block_content_splitting_a_blending_inline_blends_in_its_group() {
    let trace = trace(
        "span{mix-blend-mode:multiply}.block{background:#ff00ff}",
        "<div><span>A<div class='block'>Block</div>B</span></div>",
    );
    let painted = effect_stacks(&trace, 0);
    for name in ["text A", "box #ff00ff", "text Block", "text B"] {
        assert_eq!(
            stack_of(&painted, name),
            ["BMultiply"],
            "{name} blends as part of the split span: {painted:#?}"
        );
    }
    let opened = trace.display_lists[0]
        .iter()
        .filter(|item| matches!(item, DisplayItem::PushBlend(ferri_pdf::BlendMode::Multiply)))
        .count();
    assert_eq!(
        opened, 1,
        "the span's parts and the block it splits form one blend group: {painted:#?}"
    );
}

fn stack_of<'a>(painted: &'a [(String, Vec<String>)], name: &str) -> &'a [String] {
    &painted
        .iter()
        .find(|(item, _)| item == name)
        .unwrap_or_else(|| panic!("{name} is not painted: {painted:#?}"))
        .1
}

#[test]
fn groups_nest_by_stacking_context_while_clips_and_transforms_reopen_inside_them() {
    let trace = trace(
        ".cb{position:relative}.o{opacity:.5;background:#ff0000}.clip{overflow:hidden;height:20pt;background:#00ff00}.abs{position:absolute;top:0;left:200pt;background:#0000ff}.t{opacity:.25;transform:rotate(5deg);background:#ffff00}.after{background:#00ffff;margin-top:-10pt}",
        "<div class='cb'><div class='o'>Outer<div class='clip'>Clipped<div class='abs'>Abs</div></div><div class='t'>Turned</div></div><div class='after'>After</div></div>",
    );
    let painted = effect_stacks(&trace, 0);
    let expect = |name: &str, effects: &[&str]| {
        assert_eq!(
            stack_of(&painted, name),
            effects,
            "{name} effects; everything painted: {painted:#?}"
        );
    };
    expect("box #ff0000", &["O0.5"]);
    expect("text Outer", &["O0.5"]);
    expect("box #00ff00", &["O0.5"]);
    expect("text Clipped", &["O0.5", "C"]);
    // The absolute box's containing block is outside the clip, but it belongs to the
    // translucent box's stacking context, so it stays in the group without the clip.
    expect("box #0000ff", &["O0.5"]);
    expect("text Abs", &["O0.5"]);
    // A nested translucent, transformed box opens its group first and transforms inside it.
    expect("box #ffff00", &["O0.5", "O0.25", "T"]);
    expect("text Turned", &["O0.5", "O0.25", "T"]);
    expect("box #00ffff", &[]);
    let opened: Vec<_> = trace.display_lists[0]
        .iter()
        .filter(|item| matches!(item, DisplayItem::PushOpacity(alpha) if *alpha == 0.5))
        .collect();
    assert_eq!(
        opened.len(),
        1,
        "the stacking context is contiguous in paint order, so its group opens once"
    );
    let later = painted
        .iter()
        .position(|(name, _)| name == "box #00ffff")
        .unwrap();
    let group = painted
        .iter()
        .position(|(name, _)| name == "box #ff0000")
        .unwrap();
    assert!(
        later < group,
        "opacity makes a z-index 0 stacking context, painted after later in-flow backgrounds"
    );
}

#[test]
fn translucent_inline_boxes_split_their_runs_into_groups() {
    let trace = trace(
        "p{width:300pt}.o{opacity:.5;background:#ff0000;text-decoration:underline}.i{opacity:.5;background:#00ff00}.ib{display:inline-block;background:#0000ff}",
        "<p>Plain <span class='o'>half <b class='i'>quarter</b> <span class='ib'>atomic</span></span> tail</p>",
    );
    let painted = effect_stacks(&trace, 0);
    let runs = |effects: &[&str]| -> String {
        painted
            .iter()
            .filter(|(name, stack)| name.starts_with("text") && stack == effects)
            .count()
            .to_string()
    };
    assert_eq!(stack_of(&painted, "box #ff0000"), ["O0.5"]);
    assert_eq!(
        stack_of(&painted, "Underline decoration"),
        ["O0.5"],
        "the translucent box's own underline paints in its group"
    );
    assert_eq!(stack_of(&painted, "box #00ff00"), ["O0.5", "O0.5"]);
    assert_eq!(
        stack_of(&painted, "box #0000ff"),
        ["O0.5"],
        "an atomic inline paints in its inline box's group"
    );
    assert_eq!(
        [runs(&[]), runs(&["O0.5"]), runs(&["O0.5", "O0.5"])],
        ["1", "2", "1"],
        "plain runs, the half group's runs (its own and the inline-block's line), and the nested quarter group: {painted:#?}"
    );
    let line = &trace.pages[0];
    let plain = line
        .iter()
        .find(|line| line.runs.iter().any(|run| run.text.contains("Plain")))
        .expect("the plain part of the line");
    assert!(
        plain.runs.iter().all(|run| !run.text.contains("half")),
        "translucent text is removed from the plain line: {:?}",
        plain.runs.iter().map(|run| &run.text).collect::<Vec<_>>()
    );
}

#[test]
fn block_content_splitting_a_translucent_inline_composites_in_its_group() {
    let trace = trace(
        ".o{opacity:.5}.b{display:block;background:#ff0000;opacity:.5}",
        "<p>Start <span class='o'>in<span class='b'>Block</span>out</span></p>",
    );
    let painted = effect_stacks(&trace, 0);
    assert_eq!(
        stack_of(&painted, "box #ff0000"),
        ["O0.5", "O0.5"],
        "the block splitting the inline composites its own group inside the inline's: {painted:#?}"
    );
    assert_eq!(stack_of(&painted, "text Block"), ["O0.5", "O0.5"]);
    for (line, effects) in [
        ("text Start in", &[][..]),
        ("text Start in", &["O0.5"]),
        ("text out", &["O0.5"]),
    ] {
        assert!(
            painted
                .iter()
                .any(|(name, stack)| name == line && stack == effects),
            "{line} has runs in {effects:?}: {painted:#?}"
        );
    }
    let opened = trace.display_lists[0]
        .iter()
        .filter(|item| matches!(item, DisplayItem::PushOpacity(_)))
        .count();
    assert_eq!(
        opened, 2,
        "the inline's parts and the block share one group, which opens once: {painted:#?}"
    );
}

#[test]
fn opacity_values_clamp_and_accept_percentages() {
    for (value, expected) in [
        ("40%", 0.4),
        ("1.5", 1.0),
        ("-2", 0.0),
        ("calc(0.2 + 0.1)", 0.3),
    ] {
        let trace = trace(
            &format!("div{{opacity:{value};background:#ff0000}}"),
            "<div>A</div>",
        );
        let alphas: Vec<_> = trace.display_lists[0]
            .iter()
            .filter_map(|item| match item {
                DisplayItem::PushOpacity(alpha) => Some(*alpha),
                _ => None,
            })
            .collect();
        let expected_groups: Vec<f32> = if expected < 1.0 {
            vec![expected]
        } else {
            Vec::new()
        };
        assert_eq!(
            alphas.len(),
            expected_groups.len(),
            "opacity:{value} groups {alphas:?}"
        );
        for (alpha, expected) in alphas.iter().zip(expected_groups) {
            assert!(
                (alpha - expected).abs() < 1e-6,
                "opacity:{value} gives {alpha}, expected {expected}"
            );
        }
    }
}

#[test]
fn table_wrapper_effects_enclose_captions_and_grid_exactly_once() {
    for (declaration, expected) in [
        ("opacity:.5", vec!["O0.5"]),
        ("transform:translate(12pt,7pt)", vec!["T"]),
        ("overflow:hidden", vec!["C"]),
        (
            "opacity:.5;transform:translate(12pt,7pt);overflow:hidden",
            vec!["O0.5", "T", "C"],
        ),
    ] {
        for side in ["top", "bottom"] {
            let output = trace(
                &format!(
                    "table{{width:160pt;border-spacing:0;background:red;{declaration}}}caption{{caption-side:{side}}}td{{padding:0}}p{{margin:0}}"
                ),
                "<table><caption>Caption</caption><tr><td>Cell</td></tr></table><p>After</p>",
            );
            let painted = effect_stacks(&output, 0);
            for name in ["text Caption", "text Cell", "box #ff0000"] {
                assert_eq!(
                    stack_of(&painted, name),
                    expected,
                    "{declaration}, caption {side}, {name}: {painted:?}"
                );
            }
            assert!(
                stack_of(&painted, "text After").is_empty(),
                "effects must close after the wrapper"
            );
        }
    }
}

#[test]
fn fragmented_captioned_tables_keep_one_wrapper_effect_scope_on_each_page() {
    let rows = (0..20)
        .map(|index| format!("<tr><td>Row{index}</td></tr>"))
        .collect::<String>();
    let output = trace(
        "@page{size:240pt 160pt;margin:20pt}table{width:160pt;border-spacing:0;opacity:.5;transform:translate(5pt,3pt);overflow:hidden}td{padding:0}p{margin:0}",
        &format!("<table><caption>Caption</caption>{rows}</table><p>After</p>"),
    );
    assert!(output.pages.len() > 1);
    let mut rows_seen = 0;
    for page in 0..output.pages.len() {
        let painted = effect_stacks(&output, page);
        for (name, effects) in &painted {
            if name.starts_with("text Row") || name == "text Caption" {
                assert_eq!(
                    effects,
                    &["O0.5", "T", "C"],
                    "page {page}, {name}: {painted:?}"
                );
                rows_seen += usize::from(name.starts_with("text Row"));
            } else if name == "text After" {
                assert!(
                    effects.is_empty(),
                    "following flow must leave the wrapper scope"
                );
            }
        }
    }
    assert_eq!(rows_seen, 20, "all rows must survive fragmentation");
}

#[test]
fn table_cell_background_and_content_share_the_cell_opacity_group() {
    let trace = trace(
        "table{border-spacing:0}td{padding:0;opacity:.5;background:red}",
        "<table><tr><td>Cell</td></tr></table><p>After</p>",
    );
    let painted = effect_stacks(&trace, 0);
    assert_eq!(stack_of(&painted, "box #ff0000"), &["O0.5"]);
    assert_eq!(stack_of(&painted, "text Cell"), &["O0.5"]);
    assert_eq!(stack_of(&painted, "text After"), [] as [&str; 0]);
}

#[test]
fn row_and_group_opacity_enclose_only_their_own_table_layers() {
    let trace = trace(
        "table{border-spacing:0;background:blue}col{background:cyan}tbody{background:lime;opacity:.5}.row{background:red;opacity:.25}.cell{background:yellow;opacity:.75}td{padding:0}",
        "<table><col><col><tbody><tr class=row><td class=cell>A</td><td>B</td></tr><tr><td>C</td><td>D</td></tr></tbody></table><p>After</p>",
    );
    let painted = effect_stacks(&trace, 0);
    for (name, expected) in [
        ("box #0000ff", vec![]),
        ("box #00ffff", vec![]),
        ("box #00ff00", vec!["O0.5"]),
        ("box #ff0000", vec!["O0.5", "O0.25"]),
        ("box #ffff00", vec!["O0.5", "O0.25", "O0.75"]),
        ("text A", vec!["O0.5", "O0.25", "O0.75"]),
        ("text B", vec!["O0.5", "O0.25"]),
        ("text C", vec!["O0.5"]),
        ("text D", vec!["O0.5"]),
        ("text After", vec![]),
    ] {
        assert_eq!(stack_of(&painted, name), expected, "{name}: {painted:#?}");
    }
    for alpha in [0.5, 0.25, 0.75] {
        assert_eq!(
            trace.display_lists[0]
                .iter()
                .filter(|item| matches!(item, DisplayItem::PushOpacity(value) if *value == alpha))
                .count(),
            1,
            "opacity {alpha} must enclose its element once, not once per cell"
        );
    }
}

#[test]
fn column_non_background_effects_are_ignored_without_rejecting_the_table() {
    let trace = trace(
        "table{border-spacing:0}colgroup{opacity:.5}col{opacity:.25;background:red;box-shadow:10pt 10pt 5pt lime;border-image:url(../assets/border-image-nine.png) 10}td{padding:0}",
        "<table><colgroup><col></colgroup><tr><td>Cell</td></tr></table>",
    );
    let painted = effect_stacks(&trace, 0);
    assert_eq!(stack_of(&painted, "box #ff0000"), [] as [&str; 0]);
    assert_eq!(stack_of(&painted, "text Cell"), [] as [&str; 0]);
    for item in trace.display_lists.iter().flatten() {
        if let DisplayItem::Box(paint) = item {
            assert!(
                paint.decoration.shadows.is_empty(),
                "column shadow must not paint"
            );
            assert!(
                !paint.decoration.border_image.is_drawn(),
                "column border image must not paint"
            );
        }
    }
}

#[test]
fn static_table_cell_transform_keeps_its_background_and_content_together() {
    let trace = trace(
        "table{border-spacing:0}td{padding:0;background:red;transform:translate(10pt,3pt)}",
        "<table><tr><td>Cell</td></tr></table><p>After</p>",
    );
    let painted = effect_stacks(&trace, 0);
    assert_eq!(stack_of(&painted, "box #ff0000"), &["T"]);
    assert_eq!(stack_of(&painted, "text Cell"), &["T"]);
    assert_eq!(stack_of(&painted, "text After"), [] as [&str; 0]);
}

#[test]
fn table_group_opacity_survives_repeated_headers_footers_and_body_fragments() {
    let rows = (0..6)
        .map(|index| format!("<tr><td><div>Row{index}</div></td></tr>"))
        .collect::<String>();
    let trace = trace(
        "@page{size:240pt 100pt;margin:20pt}body{margin:0}table{width:160pt;border-spacing:0}td{padding:0;font:10pt/10pt Spike}thead{opacity:.25}tbody{opacity:.5}tfoot{opacity:.75}tbody div{height:30pt}p{margin:0}",
        &format!(
            "<table><thead><tr><td>Header</td></tr></thead><tbody>{rows}</tbody><tfoot><tr><td>Footer</td></tr></tfoot></table><p>After</p>"
        ),
    );
    assert!(trace.pages.len() > 1);
    let mut seen = vec![0; 6];
    for page in 0..trace.pages.len() {
        let painted = effect_stacks(&trace, page);
        for (name, effects) in &painted {
            if let Some(index) = name.strip_prefix("text Row") {
                let index: usize = index.parse().unwrap();
                seen[index] += 1;
                assert_eq!(effects, &["O0.5"], "page {page}, {name}");
            } else if name == "text Header" {
                assert_eq!(effects, &["O0.25"], "page {page}");
            } else if name == "text Footer" {
                assert_eq!(effects, &["O0.75"], "page {page}");
            } else if name == "text After" {
                assert!(effects.is_empty(), "table group leaked into following flow");
            }
        }
    }
    assert_eq!(
        seen,
        vec![1; 6],
        "source rows must paint once across fragments"
    );
}

#[test]
fn positioned_descendants_and_nested_tables_keep_structural_opacity_ancestors() {
    let trace = trace(
        ".outer{position:relative;border-spacing:0}.group{opacity:.5}.row{opacity:.25}.inner{opacity:.75}.abs{position:absolute;top:0;left:100pt;z-index:-1}td{padding:0}",
        "<table class=outer><tbody class=group><tr class=row><td>A<div class=abs>Absolute</div><table class=inner><tr><td>Nested</td></tr></table></td><td>B</td></tr></tbody></table><p>After</p>",
    );
    let painted = effect_stacks(&trace, 0);
    for name in ["text A", "text B", "text Absolute"] {
        assert_eq!(
            stack_of(&painted, name),
            &["O0.5", "O0.25"],
            "{name}: {painted:#?}"
        );
    }
    assert_eq!(
        stack_of(&painted, "text Nested"),
        &["O0.5", "O0.25", "O0.75"]
    );
    assert_eq!(stack_of(&painted, "text After"), [] as [&str; 0]);
}

#[test]
fn spanning_cells_keep_origin_row_opacity_across_named_pages_and_hidden_empty_cells() {
    let output = trace(
        "@page{size:240pt 140pt;margin:20pt}@page wide{size:400pt 140pt;margin:20pt}table{width:100%;border-spacing:0}tbody{opacity:.75}.first{opacity:.5}.wide{page:wide;break-before:page;opacity:.25}td{padding:0;vertical-align:top}.empty{empty-cells:hide;opacity:0;background:red}.tail{break-before:page}p{margin:0}",
        "<table><tbody><tr class=first><td rowspan=2><p>Span</p><p class=tail>Tail</p></td><td>A</td><td class=empty></td></tr><tr class=wide><td>B</td><td>C</td></tr></tbody></table><p>After</p>",
    );
    assert!(output.pages.len() > 1);
    let mut seen = std::collections::BTreeMap::new();
    for page in 0..output.pages.len() {
        for (name, effects) in effect_stacks(&output, page) {
            assert_ne!(
                name, "box #ff0000",
                "hidden empty cell must not paint its background"
            );
            let expected: &[&str] = match name.as_str() {
                "text Span" | "text Tail" | "text A" => &["O0.75", "O0.5"],
                "text B" | "text C" => &["O0.75", "O0.25"],
                "text After" => &[],
                _ => continue,
            };
            assert_eq!(effects, expected, "page {page}: {name}");
            *seen.entry(name).or_insert(0) += 1;
        }
    }
    for name in ["Span", "Tail", "A", "B", "C", "After"] {
        assert_eq!(
            seen.get(&format!("text {name}")),
            Some(&1),
            "source {name}: {seen:?}"
        );
    }
}

#[test]
fn table_structural_transforms_follow_background_ancestry() {
    let trace = trace(
        "table{width:160pt;border-spacing:0;background:blue}col{background:cyan}tbody{transform:translate(10pt,3pt);background:green}tr{transform:scale(.5);background:red}td{padding:0}.cell{background:yellow;transform:translate(2pt,1pt)}",
        "<table><col><col><tbody><tr><td class=cell>A</td><td>B</td></tr></tbody></table><p>After</p>",
    );
    let painted = effect_stacks(&trace, 0);
    for name in ["box #0000ff", "box #00ffff", "text After"] {
        assert!(stack_of(&painted, name).is_empty(), "{name}: {painted:#?}");
    }
    assert_eq!(stack_of(&painted, "box #008000"), &["T"]);
    for name in ["box #ff0000", "text B"] {
        assert_eq!(stack_of(&painted, name), &["T", "T"], "{name}");
    }
    for name in ["box #ffff00", "text A"] {
        assert_eq!(stack_of(&painted, name), &["T", "T", "T"], "{name}");
    }
}

#[test]
fn structural_transform_effects_survive_repeated_table_groups() {
    let rows = (0..6)
        .map(|index| format!("<tr><td><div>Row{index}</div></td></tr>"))
        .collect::<String>();
    let output = trace(
        "@page{size:240pt 100pt;margin:20pt}body{margin:0}table{width:160pt;border-spacing:0}td{padding:0;font:10pt/10pt Spike}thead{transform:translate(2pt,0)}tbody{transform:translate(3pt,0);opacity:.5}tfoot{transform:translate(4pt,0)}tbody div{height:30pt}p{margin:0}",
        &format!(
            "<table><thead><tr><td>Header</td></tr></thead><tbody>{rows}</tbody><tfoot><tr><td>Footer</td></tr></tfoot></table><p>After</p>"
        ),
    );
    assert!(output.pages.len() > 1);
    let mut counts = [0; 6];
    for page in 0..output.pages.len() {
        for (name, stack) in effect_stacks(&output, page) {
            if let Some(index) = name.strip_prefix("text Row") {
                counts[index.parse::<usize>().unwrap()] += 1;
                assert_eq!(stack, ["O0.5", "T"], "page {page}: {name}");
            } else if name == "text Header" || name == "text Footer" {
                assert_eq!(stack, ["T"], "page {page}: {name}");
            } else if name == "text After" {
                assert_eq!(stack, [] as [&str; 0], "page {page}: {name}");
            }
        }
    }
    assert_eq!(counts, [1; 6]);
}

#[test]
fn singular_row_transform_omits_paint_and_preserves_following_flow() {
    let output = trace(
        "table{border-spacing:0}tr:first-child{transform:scale(0)}td{padding:0}",
        "<table><tr><td>Hidden</td></tr><tr><td>Visible</td></tr></table><p>After</p>",
    );
    let painted = effect_stacks(&output, 0);
    assert!(!painted.iter().any(|(name, _)| name == "text Hidden"));
    for name in ["text Visible", "text After"] {
        assert_eq!(stack_of(&painted, name), [] as [&str; 0]);
    }
}

#[test]
fn transformed_origin_row_keeps_its_spanning_cell_on_later_named_pages() {
    let output = trace(
        "@page{size:240pt 140pt;margin:20pt}@page wide{size:400pt 140pt;margin:20pt}table{width:100%;border-spacing:0}.first{transform:translate(12pt,3pt)}.wide{page:wide;break-before:page}td{padding:0;vertical-align:top}.tail{break-before:page}p{margin:0}",
        "<table><tbody><tr class=first><td rowspan=2><p>Span</p><p class=tail>Tail</p></td><td>A</td></tr><tr class=wide><td>B</td></tr></tbody></table><p>After</p>",
    );
    assert!(output.pages.len() > 1);
    let mut seen = 0;
    for page in 0..output.pages.len() {
        for (name, stack) in effect_stacks(&output, page) {
            if matches!(name.as_str(), "text Span" | "text Tail" | "text A") {
                seen += 1;
                assert_eq!(stack, ["T"], "{name} on page {page}");
            } else if name == "text B" || name == "text After" {
                assert_eq!(stack, [] as [&str; 0], "{name} on page {page}");
            }
        }
    }
    assert_eq!(seen, 3);
}

#[test]
fn positioned_spanning_descendant_keeps_transformed_group_on_continuation() {
    // One content-page offset brings the projected row's child back onto the page.
    let output = trace(
        "@page{size:240pt 140pt;margin:20pt}table{width:100%;border-spacing:0}tbody{transform:translate(12pt,3pt)}.first{position:relative}.next{break-before:page}td{padding:0;vertical-align:top}.tail{break-before:page}.absolute{position:absolute;left:0;top:100pt}p{margin:0}",
        "<table><tbody><tr class=first><td rowspan=2><p>Span</p><div class=tail>Tail<a class=absolute>Absolute</a></div></td><td>A</td></tr><tr class=next><td>B</td></tr></tbody></table><p>After</p>",
    );
    assert!(output.pages.len() > 1);
    let mut found = 0;
    for page in 0..output.pages.len() {
        for (name, stack) in effect_stacks(&output, page) {
            if name == "text Absolute" {
                found += 1;
                assert_eq!(stack, ["T"], "page {page}");
                let baseline = |text| {
                    output.pages[page]
                        .iter()
                        .find(|line| line.text == text)
                        .unwrap()
                        .baseline
                };
                common::assert_pt(baseline("Absolute"), baseline("Tail"));
            }
        }
    }
    assert_eq!(
        found,
        1,
        "lines: {:?}, paint: {:?}",
        output
            .pages
            .iter()
            .map(|lines| lines
                .iter()
                .map(|line| (&line.text, line.x, line.baseline))
                .collect::<Vec<_>>())
            .collect::<Vec<_>>(),
        (0..output.pages.len())
            .map(|page| effect_stacks(&output, page))
            .collect::<Vec<_>>()
    );
}

#[test]
fn filters_open_inside_their_element_clips_and_enclose_descendant_groups() {
    let trace = trace(
        ".clip{overflow:hidden;height:60pt;background:#ff0000}\
         .f{filter:blur(2pt);opacity:.5;background:#00ff00;transform:translateX(3pt)}\
         .inner{opacity:.25;background:#0000ff}.after{background:#ffff00}",
        "<div class='clip'><div class='f'>Filtered<div class='inner'>Inner</div></div></div>\
         <div class='after'>After</div>",
    );
    let painted = effect_stacks(&trace, 0);
    let expect = |name: &str, effects: &[&str]| {
        assert_eq!(
            stack_of(&painted, name),
            effects,
            "{name} effects in: {painted:#?}"
        );
    };
    let filter = "F[Blur(2.0)]";
    // Filter Effects 1 §5: the filter applies in the element's transformed coordinates,
    // before its ancestors' clips; opacity and the descendants' groups apply around and
    // inside it as one group.
    expect("box #ff0000", &[]);
    expect("box #00ff00", &["C", "T", "O0.5", filter]);
    expect("text Filtered", &["C", "T", "O0.5", filter]);
    expect("box #0000ff", &["C", "T", "O0.5", filter, "O0.25"]);
    expect("text Inner", &["C", "T", "O0.5", filter, "O0.25"]);
    expect("box #ffff00", &[]);
}

#[test]
fn filtered_inline_boxes_and_fixed_descendants_form_one_filter_group() {
    let trace = trace(
        "span{filter:invert(1)}.f{filter:sepia(1);margin-left:40pt;background:#ff0000}\
         .fixed{position:fixed;left:0;top:0;width:10pt;height:10pt;background:#00ff00}",
        "<p>Plain <span>Inverted</span></p><div class='f'>Host<div class='fixed'></div></div>",
    );
    let painted = effect_stacks(&trace, 0);
    for effects in [&["F[Invert(1.0)]"][..], &[]] {
        assert!(
            painted
                .iter()
                .any(|(name, stack)| name == "text Plain Inverted" && stack == effects),
            "the line's runs split by the filtered span: {effects:?} in {painted:#?}"
        );
    }
    assert_eq!(
        stack_of(&painted, "box #00ff00"),
        ["F[Sepia(1.0)]"],
        "a fixed descendant is contained by and painted within the filter (Filter Effects 1 §5): {painted:#?}"
    );
    let rect_of = |rgb: [u8; 3]| {
        trace.display_lists[0]
            .iter()
            .find_map(|item| match item {
                DisplayItem::Box(paint) if paint.decoration.background.rgb == rgb => {
                    Some(paint.rect)
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("box {rgb:?} painted: {painted:#?}"))
    };
    let (host, fixed) = (rect_of([255, 0, 0]), rect_of([0, 255, 0]));
    assert_eq!(
        [fixed.x, fixed.y],
        [host.x, host.y],
        "the filtered host is the fixed box's containing block"
    );
}
