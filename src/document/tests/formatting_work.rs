//! Formatting a document charges every subtree it formats, copies included, to one budget.
use crate::document::{BlockBox, BlockContent, DocumentError, StyledDocument};

const NESTING: usize = 3;
const SCROLLERS: usize = 16;

fn parse(html: &str, limit: usize) -> Result<StyledDocument, DocumentError> {
    StyledDocument::parse_with_work_limit(html, limit)
}

/// Whether formatting left out what exceeds a limit of `document`.
fn degraded(document: &StyledDocument) -> bool {
    document.diagnostics.iter().any(|diagnostic| {
        matches!(
            diagnostic,
            crate::document::DocumentDiagnostic::FormattingLimit { .. }
                | crate::document::DocumentDiagnostic::SkippedFormattingWork { .. }
        )
    })
}

/// Whether `html` formats within `limit` work units, leaving nothing out.
fn fits(html: &str, limit: usize) -> bool {
    match parse(html, limit) {
        Ok(document) => !degraded(&document),
        Err(DocumentError::FormattingWorkLimit { limit: reported }) => {
            assert_eq!(reported, limit, "the error must name the exhausted budget");
            false
        }
        Err(error) => panic!("a budget of {limit} failed with {error} instead\n{html}"),
    }
}

/// The formatting work `html` needs, overestimated by at most a `1 / PRECISION` fraction,
/// found by searching the budget: formatting charges the same units under every budget.
fn required_work(html: &str) -> usize {
    const PRECISION: usize = 64;
    let (mut fails, mut fits_within) = (0, 1);
    while !fits(html, fits_within) {
        assert!(
            fits_within < crate::document::MAX_FORMATTING_WORK,
            "fixture must format within the document budget\n{html}"
        );
        fails = fits_within;
        fits_within *= 2;
    }
    while (fits_within - fails) * PRECISION > fits_within {
        let limit = fails + (fits_within - fails) / 2;
        if fits(html, limit) {
            fits_within = limit;
        } else {
            fails = limit;
        }
    }
    fits_within
}

/// `depth` inline-blocks, each the first thing on the first line of the block around it.
fn nested_atomics(depth: usize) -> String {
    format!(
        "{}x{}",
        "<div><span>".repeat(depth),
        "</span></div>".repeat(depth)
    )
}

fn document(rules: &str, body: &str) -> String {
    format!("<style>body{{margin:0}}span{{display:inline-block}}{rules}</style>{body}")
}

const FIRST_LINE: &str = "div::first-line{color:red}";

#[test]
fn first_line_copies_of_nested_atomic_inlines_charge_the_shared_budget_once() {
    let plain = required_work(&document("", &nested_atomics(NESTING)));
    let nested = plain - required_work(&document("", &nested_atomics(0)));
    let copies = required_work(&document(FIRST_LINE, &nested_atomics(NESTING))) - plain;
    assert!(
        2 * copies > nested && copies <= nested,
        "the first line restyles every nested atomic inline alike, so the copy of the outermost \
         one, which holds the others, is the copy each of them shares: the copies must charge \
         the budget for formatting that subtree once, {copies} units for a subtree of {nested}"
    );
}

/// `count` nested scroll containers, each placing its scroll marker group before it.
fn nested_scrollers(count: usize) -> String {
    document(
        "div{overflow:auto;scroll-marker-group:before}div::scroll-marker{content:'m'}",
        &format!("{}x{}", "<div>".repeat(count), "</div>".repeat(count)),
    )
}

#[test]
fn one_prepass_collects_the_markers_of_nested_containers() {
    let nested = required_work(&nested_scrollers(SCROLLERS));
    let doubled = required_work(&nested_scrollers(2 * SCROLLERS));
    assert!(
        doubled < 3 * nested,
        "the prepass of the outermost container collects the markers of every container inside \
         it, so doubling the nesting must about double the charged work, not quadruple it: \
         {SCROLLERS} containers need {nested} units, {} need {doubled}",
        2 * SCROLLERS
    );
}

/// `count` consecutive inline footnotes, which share one anonymous block (CSS GCPM 3 §2.3).
fn inline_footnotes(count: usize) -> String {
    document(
        ".fn{float:footnote;footnote-display:inline}",
        &format!(
            "<p>{}</p>",
            (0..count)
                .map(|index| format!("w{index}<span class=fn>note {index}</span> "))
                .collect::<String>()
        ),
    )
}

#[test]
fn each_inline_footnote_collects_only_its_own_events() {
    const FOOTNOTES: usize = 256;
    let single = required_work(&inline_footnotes(FOOTNOTES));
    let doubled = required_work(&inline_footnotes(2 * FOOTNOTES));
    assert!(
        doubled < 3 * single,
        "an inline footnote joining the block of those before it collects the events of its own \
         content, not of the whole block, so doubling the footnotes must about double the \
         charged work, not quadruple it: {FOOTNOTES} footnotes need {single} units, {} need \
         {doubled}",
        2 * FOOTNOTES
    );
}

#[test]
fn scroll_marker_prepasses_make_no_first_line_copies() {
    let container = |group, rules| {
        document(
            &format!(".s{{overflow:auto;scroll-marker-group:{group}}}{rules}"),
            &format!("<section class=s>{}</section>", nested_atomics(NESTING)),
        )
    };
    let prepassed = required_work(&container("before", FIRST_LINE));
    let collected = required_work(&container("after", FIRST_LINE));
    let plain = required_work(&container("after", ""));
    assert!(
        prepassed > collected && prepassed - collected <= plain,
        "a prepass only collects the container's scroll markers, which first-line copies do not \
         hold, so it must charge the budget, but no more than formatting the container without \
         copies: \
         {prepassed} units with the prepass, {collected} without, {plain} without copies"
    );
}

#[test]
fn first_line_copies_that_exhaust_the_budget_are_left_out() {
    let html = document(FIRST_LINE, &nested_atomics(NESTING));
    let plain = required_work(&document("", &nested_atomics(NESTING)));
    let styled = required_work(&html);
    let limit = usize::midpoint(plain, styled);
    let document = parse(&html, limit).unwrap_or_else(|error| {
        panic!(
            "a budget of {limit} units, enough without the first-line copies that {styled} \
             units format, must format without them: {error}"
        )
    });
    let diagnostics: Vec<String> = document
        .diagnostics
        .iter()
        .map(ToString::to_string)
        .collect();
    assert_eq!(
        diagnostics,
        [format!(
            "optional restyling past the {limit}-unit formatting work budget was skipped"
        )],
        "the copies left out are reported once"
    );
    let atomic = nested_atomic(document.root(), 0).expect("the outermost atomic inline formats");
    assert!(
        atomic.first_line_box().is_none(),
        "the outermost atomic inline keeps its own styles on the first line"
    );
}

/// The atomic inline nested `depth` levels inside `block`'s first one, if there is one.
fn nested_atomic(block: &BlockBox, depth: usize) -> Option<&BlockBox> {
    let mut atomic = block;
    for _ in 0..=depth {
        atomic = first_atomic(atomic)?;
    }
    Some(atomic)
}

/// The first atomic inline of the paragraphs inside `block`, outside other atomic inlines.
fn first_atomic(block: &BlockBox) -> Option<&BlockBox> {
    block.children().iter().find_map(|child| match child {
        BlockContent::Block(child) => first_atomic(child),
        BlockContent::Inline(paragraph) => paragraph
            .atomics()
            .first()
            .map(crate::document::Anchor::block),
        BlockContent::Events(_) | BlockContent::Positioned(_) => None,
    })
}

/// Checks that each of the `depth` nested atomic inlines of `document` keeps its parent's
/// colour and, on its block's first line, takes the colour of `FIRST_LINE` into its copy and
/// every atomic inline the copy holds, which the line leaves alone.
fn assert_restyled_at_every_level(document: &StyledDocument, depth: usize) {
    const RED: [u8; 3] = [255, 0, 0];
    for level in 0..depth {
        let atomic = nested_atomic(document.root(), level)
            .unwrap_or_else(|| panic!("level {level} of {depth} holds an atomic inline"));
        assert_eq!(
            atomic.style().color,
            crate::paint::Color::BLACK,
            "level {level} keeps its parent's colour off the first line"
        );
        let copy = atomic
            .first_line_box()
            .unwrap_or_else(|| panic!("the first line restyles level {level} of {depth}"));
        for inner in 0..depth - level {
            let nested = match inner.checked_sub(1) {
                Some(below) => nested_atomic(copy, below)
                    .unwrap_or_else(|| panic!("the copy of level {level} holds {inner} levels")),
                None => copy,
            };
            assert_eq!(
                nested.style().color.rgb,
                RED,
                "level {} inside the first-line copy of level {level} inherits the line's colour",
                level + inner
            );
            assert!(
                nested.first_line_box().is_none(),
                "the first line of level {} inside a copy leaves it alone, so it needs no copy",
                level + inner
            );
        }
    }
}

#[test]
fn deep_first_line_nesting_formats_in_linear_work_on_a_library_thread() {
    const LIBRARY_STACK_BYTES: usize = 2 * 1024 * 1024;
    const SHALLOW: usize = 16;
    const DEEP: usize = 64;
    // Each level nests a block and an atomic inline in it, below the root and the body.
    const DEEPEST: usize = (crate::document::formatting::MAX_FORMATTING_DEPTH - 1) / 2;
    std::thread::Builder::new()
        .stack_size(LIBRARY_STACK_BYTES)
        .spawn(|| {
            let [shallow, deep] = [SHALLOW, DEEP].map(|depth| {
                let html = document(FIRST_LINE, &nested_atomics(depth));
                let document = parse(&html, crate::document::MAX_FORMATTING_WORK)
                    .unwrap_or_else(|error| panic!("{depth} levels must format: {error}"));
                assert_restyled_at_every_level(&document, depth);
                required_work(&html)
            });
            assert!(
                deep * SHALLOW < shallow * DEEP,
                "formatting a level and its first-line copy is formatting work of its own, so \
                 nesting {DEEP} levels must cost less than {DEEP}/{SHALLOW} times nesting \
                 {SHALLOW}: {shallow} units for {SHALLOW} levels, {deep} for {DEEP}"
            );
            // Past the formatting depth, the only limit on first-line copies, the deeper
            // elements lie flattened in the deepest box.
            for (depth, flattened) in [(DEEPEST, false), (DEEPEST + 1, true)] {
                let html = document(FIRST_LINE, &nested_atomics(depth));
                let document = parse(&html, crate::document::MAX_FORMATTING_WORK)
                    .unwrap_or_else(|error| panic!("{depth} levels must format: {error}"));
                assert_eq!(
                    degraded(&document),
                    flattened,
                    "{depth} levels report whether they were flattened"
                );
            }
        })
        .expect("the library thread must start")
        .join()
        .expect("deep first-line nesting must format without overflowing the stack");
}
