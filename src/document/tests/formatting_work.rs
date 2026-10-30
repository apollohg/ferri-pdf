//! Formatting a document charges every subtree it formats, copies included, to one budget.
use crate::document::{DocumentError, StyledDocument};

const NESTING: usize = 3;
const SCROLLERS: usize = 16;

fn parse(html: &str, limit: usize) -> Result<StyledDocument, DocumentError> {
    StyledDocument::parse_with_work_limit(html, limit)
}

/// Whether `html` formats within `limit` work units.
fn fits(html: &str, limit: usize) -> bool {
    match parse(html, limit) {
        Ok(_) => true,
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
fn first_line_copies_of_nested_atomic_inlines_charge_the_shared_budget() {
    let copies = |depth| {
        let plain = required_work(&document("", &nested_atomics(depth)));
        let styled = required_work(&document(FIRST_LINE, &nested_atomics(depth)));
        assert!(
            styled > 2 * plain,
            "the first-line copies of {depth} nested atomic inlines format the innermost one \
             {} times and must charge the budget: {styled} units, {plain} without the rule",
            1 << depth
        );
        styled - plain
    };
    let (shallow, deep) = (copies(NESTING), copies(NESTING + 1));
    assert!(
        deep >= 2 * shallow,
        "an atomic inline on a styled first line is formatted again inside every copy of its \
         ancestors, so each level of nesting must double the work the copies charge: {shallow} \
         units for {NESTING} levels, {deep} for {}",
        NESTING + 1
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
fn scroll_marker_prepasses_of_nested_containers_charge_the_shared_budget() {
    let nested = required_work(&nested_scrollers(SCROLLERS));
    let doubled = required_work(&nested_scrollers(2 * SCROLLERS));
    assert!(
        doubled >= 3 * nested,
        "each container is formatted ahead of itself for its markers and again in the prepass \
         of every container around it, so doubling the nesting must about quadruple the charged \
         work: {SCROLLERS} containers need {nested} units, {} need {doubled}",
        2 * SCROLLERS
    );
}

#[test]
fn nested_copies_fail_before_overflowing_a_library_thread() {
    const LIBRARY_STACK_BYTES: usize = 2 * 1024 * 1024;
    const RUNAWAY_NESTING: usize = 512;
    let limit = crate::document::formatting::MAX_COPY_NESTING;
    std::thread::Builder::new()
        .stack_size(LIBRARY_STACK_BYTES)
        .spawn(move || {
            let html = document(FIRST_LINE, &nested_atomics(limit));
            if let Err(error) = parse(&html, crate::document::MAX_FORMATTING_WORK) {
                panic!("{limit} nested first-line copies must format: {error}");
            }
            for nesting in [limit + 1, RUNAWAY_NESTING] {
                let html = document(FIRST_LINE, &nested_atomics(nesting));
                match parse(&html, crate::document::MAX_FORMATTING_WORK) {
                    Err(DocumentError::Formatting(message)) => assert_eq!(
                        message,
                        format!("formatting copy nesting exceeds {limit}"),
                        "{nesting} nested first-line copies"
                    ),
                    Err(error) => panic!("{nesting} nested first-line copies failed with {error}"),
                    Ok(_) => panic!(
                        "{nesting} nested first-line copies must fail on a \
                         {LIBRARY_STACK_BYTES}-byte stack"
                    ),
                }
            }
        })
        .expect("the library thread must start")
        .join()
        .expect("nested copies must fail without panicking");
}
