//! Every way of nesting content renders at its nesting limit and past it on a library thread:
//! the limits flatten deeper content before its nesting can exhaust the stack.
use crate::document::formatting::{
    ANONYMOUS_TABLE_DEPTH, ATOMIC_INLINE_DEPTH, BOX_DEPTH, FILTER_DEPTH, FLEX_DEPTH, FLOAT_DEPTH,
    FORMATTING_NESTING_RESOURCE, GRID_DEPTH, MATHML_NESTING_RESOURCE, MAX_FORMATTING_DEPTH,
    MULTICOL_DEPTH, ORTHOGONAL_DEPTH, POSITIONED_DEPTH, TABLE_DEPTH,
};
use crate::{Error, RenderDiagnostic, RenderOptions, render_html};

/// The root's formatting depth, below which the body and its content nest.
const BODY_DEPTH: usize = 1;
/// The formatting depth below the body that content may reach.
const CONTENT_DEPTH: usize = MAX_FORMATTING_DEPTH - BODY_DEPTH;
const NAVIGABLE_DEPTH: usize = crate::document::frames::MAX_NAVIGABLE_DEPTH;
const MATH_DEPTH: usize = crate::document::mathml::MAX_DEPTH;
const SHADOW_DEPTH: usize = crate::dom::MAX_SHADOW_DEPTH;
/// Library users render on 2 MiB threads, such as tokio's workers.
const LIBRARY_STACK_BYTES: usize = 2 * 1024 * 1024;
const CONSTRUCT_ENV: &str = "FERRI_DEEP_CONSTRUCT";
/// Sizes the stack of the thread the test harness runs each test on.
const MIN_STACK_ENV: &str = "RUST_MIN_STACK";
const HARNESS_TEST: &str = "document::tests::deep_nesting::every_nesting_construct_renders_at_and_past_its_limit_on_a_library_thread";
const LEAF_TEXT: &str = "Deepest";

/// Whether a document nests its construct to its limit or one level past it.
#[derive(Clone, Copy, Debug)]
enum Boundary {
    At,
    Past,
}

/// A way of nesting content, with the documents nesting it to its limit and the diagnostic
/// that reports flattening past it.
struct Construct {
    name: &'static str,
    document: fn(Boundary) -> String,
    flattened: fn(&RenderDiagnostic) -> bool,
    /// Whether what its limit does leaves content out, failing the default render.
    loses_content: bool,
}

/// Levels of `open` and `close`, each taking `depth` formatting depth, nested below `wrapper`
/// so that the deepest box, `trailing` below the innermost level, lies exactly at the
/// formatting limit; sections below the wrapper pad the depth the levels leave. `Past`
/// adds one more level.
struct FormattingNest {
    css: &'static str,
    wrapper: (&'static str, &'static str),
    wrapper_depth: usize,
    open: &'static str,
    close: &'static str,
    depth: usize,
    trailing: usize,
}

impl FormattingNest {
    const fn level(open: &'static str, close: &'static str, depth: usize) -> Self {
        Self {
            css: "",
            wrapper: ("", ""),
            wrapper_depth: 0,
            open,
            close,
            depth,
            trailing: 0,
        }
    }

    const fn styled(self, css: &'static str) -> Self {
        Self { css, ..self }
    }

    const fn wrapped(self, open: &'static str, close: &'static str, depth: usize) -> Self {
        Self {
            wrapper: (open, close),
            wrapper_depth: depth,
            ..self
        }
    }

    const fn trailing(self, trailing: usize) -> Self {
        Self { trailing, ..self }
    }

    fn document(&self, boundary: Boundary) -> String {
        let available = CONTENT_DEPTH - self.wrapper_depth - self.trailing;
        let levels = available / self.depth;
        let padding = (available - levels * self.depth) / BOX_DEPTH;
        let levels = match boundary {
            Boundary::At => levels,
            Boundary::Past => levels + 1,
        };
        format!(
            "<!doctype html><style>{}</style><body>{}{}{}{LEAF_TEXT}{}{}{}",
            self.css,
            self.wrapper.0,
            "<section>".repeat(padding),
            self.open.repeat(levels),
            self.close.repeat(levels),
            "</section>".repeat(padding),
            self.wrapper.1,
        )
    }
}

fn formatting_flattened(diagnostic: &RenderDiagnostic) -> bool {
    matches!(
        diagnostic,
        RenderDiagnostic::FormattingLimit { resource: FORMATTING_NESTING_RESOURCE, limit, .. }
            if *limit == MAX_FORMATTING_DEPTH
    )
}

fn math_flattened(diagnostic: &RenderDiagnostic) -> bool {
    matches!(
        diagnostic,
        RenderDiagnostic::FormattingLimit { resource: MATHML_NESTING_RESOURCE, limit, .. }
            if *limit == MATH_DEPTH
    )
}

fn navigable_flattened(diagnostic: &RenderDiagnostic) -> bool {
    matches!(diagnostic, RenderDiagnostic::NavigableDepth { limit } if *limit == NAVIGABLE_DEPTH)
}

fn shadow_flattened(diagnostic: &RenderDiagnostic) -> bool {
    matches!(
        diagnostic,
        RenderDiagnostic::TreeLimit(crate::dom::DomError::TreeLimit { limit, .. })
            if *limit == SHADOW_DEPTH
    )
}

macro_rules! formatting_construct {
    ($name:literal, $nest:expr) => {
        Construct {
            name: $name,
            document: |boundary| $nest.document(boundary),
            flattened: formatting_flattened,
            loses_content: false,
        }
    };
}

/// Levels of `open` and `close` nested in a `math` element so that the elements of `leaf`,
/// inside the innermost level, lie at the `MathML` nesting limit; `Past` adds one more level.
fn math_document(open: &str, close: &str, leaf: &str, boundary: Boundary) -> String {
    // The math element lies at depth zero, and the leaf one level below the innermost level.
    let levels = MATH_DEPTH - 2;
    let levels = match boundary {
        Boundary::At => levels,
        Boundary::Past => levels + 1,
    };
    format!(
        "<!doctype html><body><math>{}{leaf}{}</math>",
        open.repeat(levels),
        close.repeat(levels)
    )
}

/// Navigables nested to the navigable limit, each document nesting blocks to the formatting
/// limit around the next one.
fn navigable_document(boundary: Boundary) -> String {
    let navigables = match boundary {
        Boundary::At => NAVIGABLE_DEPTH,
        Boundary::Past => NAVIGABLE_DEPTH + 1,
    };
    // The iframe is the deepest box of each document.
    let blocks = CONTENT_DEPTH - BOX_DEPTH;
    let nest = |inner: &str| {
        format!(
            "<!doctype html><body>{}{inner}{}",
            "<div>".repeat(blocks),
            "</div>".repeat(blocks)
        )
    };
    (0..navigables).fold(nest(LEAF_TEXT), |inner, _| {
        nest(&format!(
            "<iframe srcdoc=\"{}\"></iframe>",
            inner.replace('&', "&amp;").replace('"', "&quot;")
        ))
    })
}

/// Declarative shadow roots nested to the shadow tree limit, each holding blocks that bring
/// the flat tree to the formatting limit.
fn shadow_document(boundary: Boundary) -> String {
    // The host and the shadow root's first block.
    const BOXES_PER_ROOT: usize = 2;
    let roots = match boundary {
        Boundary::At => SHADOW_DEPTH,
        Boundary::Past => SHADOW_DEPTH + 1,
    };
    let blocks_per_root = CONTENT_DEPTH / SHADOW_DEPTH - BOXES_PER_ROOT;
    let open = format!(
        "<div><template shadowrootmode=open><div>{}",
        "<div>".repeat(blocks_per_root)
    );
    let close = format!(
        "{}</div></template></div>",
        "</div>".repeat(blocks_per_root)
    );
    format!(
        "<!doctype html><body>{}{LEAF_TEXT}{}",
        open.repeat(roots),
        close.repeat(roots)
    )
}

const CONSTRUCTS: &[Construct] = &[
    formatting_construct!(
        "blocks",
        FormattingNest::level("<div>", "</div>", BOX_DEPTH)
    ),
    formatting_construct!(
        "inline spans",
        FormattingNest::level("<span>", "</span>", BOX_DEPTH)
    ),
    formatting_construct!(
        "inline-blocks",
        FormattingNest::level("<span>", "</span>", BOX_DEPTH + ATOMIC_INLINE_DEPTH)
            .styled("span{display:inline-block}")
    ),
    formatting_construct!(
        "inline-tables",
        FormattingNest::level(
            "<span>",
            "</span>",
            BOX_DEPTH + ATOMIC_INLINE_DEPTH + TABLE_DEPTH
        )
        .styled("span{display:inline-table}")
    ),
    // The parser inserts the table body.
    formatting_construct!(
        "tables in cells",
        FormattingNest::level(
            "<table><tr><td>",
            "</td></tr></table>",
            TABLE_DEPTH + 4 * BOX_DEPTH
        )
    ),
    formatting_construct!(
        "flex containers",
        FormattingNest::level("<div>", "</div>", BOX_DEPTH + FLEX_DEPTH)
            .styled("div{display:flex}")
    ),
    formatting_construct!(
        "inline flex containers",
        // A flex item, blockified, holds each inline flex container in its line.
        FormattingNest::level(
            "<i><b>",
            "</b></i>",
            BOX_DEPTH + ATOMIC_INLINE_DEPTH + FLEX_DEPTH + BOX_DEPTH
        )
        .styled("i{display:inline-flex}")
    ),
    // Each kind of nesting in turn, so that their stack adds up level after level.
    formatting_construct!(
        "mixed layouts",
        FormattingNest::level(
            "<x-i><x-l><x-p><x-f><x-g><div>",
            "</div></x-g></x-f></x-p></x-l></x-i>",
            6 * BOX_DEPTH
                + ATOMIC_INLINE_DEPTH
                + FLOAT_DEPTH
                + POSITIONED_DEPTH
                + FLEX_DEPTH
                + GRID_DEPTH
        )
        .styled(
            "x-i{display:inline-block}x-l{display:block;float:left}x-p{display:block;\
             position:absolute}x-f{display:flex}x-g{display:grid}"
        )
    ),
    formatting_construct!(
        "flow roots",
        FormattingNest::level("<div>", "</div>", BOX_DEPTH).styled("div{display:flow-root}")
    ),
    formatting_construct!(
        "scroll containers",
        FormattingNest::level("<div>", "</div>", BOX_DEPTH).styled("div{overflow:hidden}")
    ),
    // Each cell outside a table generates an anonymous table around it.
    formatting_construct!(
        "table cells",
        FormattingNest::level(
            "<div>",
            "</div>",
            BOX_DEPTH + TABLE_DEPTH + ANONYMOUS_TABLE_DEPTH
        )
        .styled("div{display:table-cell}")
    ),
    formatting_construct!(
        "stacking contexts",
        FormattingNest::level("<div>", "</div>", BOX_DEPTH)
            .styled("div{opacity:.9;position:relative;z-index:1}")
    ),
    formatting_construct!(
        "filters",
        FormattingNest::level("<div>", "</div>", BOX_DEPTH + FILTER_DEPTH)
            .styled("div{filter:blur(1px)}")
    ),
    formatting_construct!(
        "backdrop filters",
        FormattingNest::level("<div>", "</div>", BOX_DEPTH + FILTER_DEPTH)
            .styled("div{backdrop-filter:blur(1px)}")
    ),
    formatting_construct!(
        "masks",
        FormattingNest::level("<div>", "</div>", BOX_DEPTH)
            .styled("div{mask-image:linear-gradient(black,transparent)}")
    ),
    formatting_construct!(
        "blend modes",
        FormattingNest::level("<div>", "</div>", BOX_DEPTH).styled("div{mix-blend-mode:multiply}")
    ),
    formatting_construct!(
        "sized orthogonal flows",
        FormattingNest::level("<div><article>", "</article></div>", 2 * BOX_DEPTH).styled(
            "div,article{width:40pt;height:40pt}div{writing-mode:vertical-rl}\
             article{writing-mode:horizontal-tb}"
        )
    ),
    formatting_construct!(
        "orthogonal flows",
        FormattingNest::level(
            "<div><article>",
            "</article></div>",
            2 * (BOX_DEPTH + ORTHOGONAL_DEPTH)
        )
        .styled("div{writing-mode:vertical-rl}article{writing-mode:horizontal-tb}")
    ),
    formatting_construct!(
        "grid containers",
        FormattingNest::level("<div>", "</div>", BOX_DEPTH + GRID_DEPTH)
            .styled("div{display:grid}")
    ),
    // MathML elements with a CSS display are laid out as CSS boxes, inside math that each
    // level opens.
    formatting_construct!(
        "MathML grid containers",
        FormattingNest::level("<math><mrow>", "</mrow></math>", 2 * BOX_DEPTH + GRID_DEPTH)
            .styled("mrow{display:grid}")
    ),
    formatting_construct!(
        "MathML flex containers",
        FormattingNest::level("<math><mrow>", "</mrow></math>", 2 * BOX_DEPTH + FLEX_DEPTH)
            .styled("mrow{display:flex}")
    ),
    formatting_construct!(
        "multi-column containers",
        FormattingNest::level("<div>", "</div>", BOX_DEPTH + MULTICOL_DEPTH)
            .styled("div{columns:2}")
    ),
    formatting_construct!(
        "lists",
        FormattingNest::level("<ol><li>", "</li></ol>", 2 * BOX_DEPTH)
    ),
    formatting_construct!(
        "floats",
        FormattingNest::level("<div>", "</div>", BOX_DEPTH + FLOAT_DEPTH).styled("div{float:left}")
    ),
    formatting_construct!(
        "absolutely positioned boxes",
        FormattingNest::level("<div>", "</div>", BOX_DEPTH + POSITIONED_DEPTH)
            .styled("div{position:absolute}")
    ),
    formatting_construct!(
        "ruby",
        FormattingNest::level("<ruby>", "<rt>a</rt></ruby>", BOX_DEPTH).trailing(BOX_DEPTH)
    ),
    // The details element's internal shadow tree slots its content into a block, beside the
    // slot holding its summary.
    formatting_construct!(
        "details",
        FormattingNest::level(
            "<details open><summary>s</summary>",
            "</details>",
            2 * BOX_DEPTH
        )
        .trailing(BOX_DEPTH)
    ),
    formatting_construct!(
        "fieldsets",
        FormattingNest::level("<fieldset><legend>l</legend>", "</fieldset>", BOX_DEPTH)
            .trailing(BOX_DEPTH)
    ),
    // A marquee, an inline-block, closes the button's scope, so the parser nests the next
    // button in it.
    formatting_construct!(
        "buttons",
        FormattingNest::level(
            "<button><marquee>",
            "</marquee></button>",
            2 * (BOX_DEPTH + ATOMIC_INLINE_DEPTH)
        )
    ),
    formatting_construct!(
        "display contents",
        FormattingNest::level("<div>", "</div>", BOX_DEPTH).styled("div{display:contents}")
    ),
    formatting_construct!(
        "3D transforms",
        FormattingNest::level("<div>", "</div>", BOX_DEPTH)
            .styled("div{transform:rotateY(10deg);transform-style:preserve-3d}")
    ),
    formatting_construct!(
        "footnotes",
        FormattingNest::level("<span>", "</span>", BOX_DEPTH).styled("span{float:footnote}")
    ),
    formatting_construct!(
        "running elements",
        FormattingNest::level("<div>", "</div>", BOX_DEPTH)
            .styled("@page{@top-center{content:element(top)}}header{position:running(top)}")
            .wrapped("<header>", "</header>", BOX_DEPTH)
    ),
    formatting_construct!(
        "first-line copies",
        FormattingNest::level("<span>", "</span>", BOX_DEPTH + ATOMIC_INLINE_DEPTH)
            .styled("span{display:inline-block}span::first-line{color:red}")
    ),
    Construct {
        name: "MathML rows",
        document: |boundary| math_document("<mrow>", "</mrow>", "<mi>x</mi>", boundary),
        flattened: math_flattened,
        loses_content: false,
    },
    Construct {
        name: "MathML fractions",
        document: |boundary| {
            math_document(
                "<mfrac><mi>x</mi>",
                "</mfrac>",
                "<mi>x</mi><mn>1</mn>",
                boundary,
            )
        },
        flattened: math_flattened,
        loses_content: false,
    },
    Construct {
        name: "iframe srcdoc",
        document: navigable_document,
        flattened: navigable_flattened,
        loses_content: true,
    },
    Construct {
        name: "shadow roots",
        document: shadow_document,
        flattened: shadow_flattened,
        loses_content: true,
    },
];

/// Renders `construct` at and past its limit on the current thread, whose stack the parent
/// sizes.
fn render_construct(construct: &Construct) {
    for (boundary, flattened) in [(Boundary::At, false), (Boundary::Past, true)] {
        let diagnostics = match render_html(
            &(construct.document)(boundary),
            &RenderOptions::default(),
        ) {
            Ok(output) => {
                assert!(
                    !(flattened && construct.loses_content),
                    "{} nested {boundary:?} the limit leaves content out, so the default render \
                     must fail: {:?}",
                    construct.name,
                    output.diagnostics
                );
                assert!(output.page_count > 0, "{} renders pages", construct.name);
                output.diagnostics
            }
            Err(Error::ContentLost { diagnostics }) if flattened && construct.loses_content => {
                diagnostics
            }
            Err(error) => panic!(
                "{} nested {boundary:?} the limit must render: {error}",
                construct.name
            ),
        };
        assert_eq!(
            diagnostics.iter().any(construct.flattened),
            flattened,
            "{} nested {boundary:?} the limit reports its limit exactly past it, failing the \
             default render only where that loses content: {diagnostics:?}",
            construct.name,
        );
    }
}

/// Each construct renders in its own process, so that a stack overflow, which aborts the
/// process, fails only that construct while the others still report.
#[test]
fn every_nesting_construct_renders_at_and_past_its_limit_on_a_library_thread() {
    if let Some(name) = std::env::var_os(CONSTRUCT_ENV) {
        let construct = CONSTRUCTS
            .iter()
            .find(|construct| name == construct.name)
            .unwrap_or_else(|| panic!("{name:?} names no construct"));
        render_construct(construct);
        return;
    }
    let executable = std::env::current_exe().expect("the test executable has a path");
    let children: Vec<_> = CONSTRUCTS
        .iter()
        .map(|construct| {
            let child = std::process::Command::new(&executable)
                .args(["--exact", HARNESS_TEST, "--nocapture"])
                .env(CONSTRUCT_ENV, construct.name)
                .env(MIN_STACK_ENV, LIBRARY_STACK_BYTES.to_string())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .unwrap_or_else(|error| panic!("{} child must start: {error}", construct.name));
            (construct.name, child)
        })
        .collect();
    let failures: Vec<_> = children
        .into_iter()
        .filter_map(|(name, child)| {
            let output = child
                .wait_with_output()
                .unwrap_or_else(|error| panic!("{name} child must finish: {error}"));
            (!output.status.success()).then(|| {
                format!(
                    "{name}: {}\n{}",
                    output.status,
                    String::from_utf8_lossy(&output.stderr)
                )
            })
        })
        .collect();
    assert!(
        failures.is_empty(),
        "every construct must render at and past its nesting limit on a \
         {LIBRARY_STACK_BYTES}-byte thread; failed:\n{}",
        failures.join("\n")
    );
}
