use crate::layout::LayoutError;
use std::{cell::Cell, rc::Rc};

/// A work budget shared by every clone, each charging the units of work it does.
#[derive(Clone)]
pub(crate) struct Budget {
    limit: usize,
    remaining: Rc<Cell<Option<usize>>>,
}

/// A budget of `limit` units ran out.
#[derive(Debug)]
pub(crate) struct Exhausted {
    pub(crate) limit: usize,
}

impl From<Exhausted> for LayoutError {
    fn from(Exhausted { limit }: Exhausted) -> Self {
        Self::WorkLimit { limit }
    }
}

impl Budget {
    pub(crate) fn new(limit: usize) -> Self {
        Self {
            limit,
            remaining: Rc::new(Cell::new(Some(limit))),
        }
    }

    pub(crate) fn spend(&self, units: usize) -> Result<(), Exhausted> {
        let remaining = self
            .remaining
            .get()
            .and_then(|left| left.checked_sub(units));
        self.remaining.set(remaining);
        remaining.map(|_| ()).ok_or(Exhausted { limit: self.limit })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::StyledDocument;
    use crate::layout::{MAX_LAYOUT_PAGES, MAX_LAYOUT_WORK, layout_with_work_limit};
    use crate::resources::{FileResolver, ResourceBase};

    #[test]
    fn target_stabilization_charges_all_passes_to_one_budget() {
        let resolver = FileResolver::default();
        let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/oracle/fixtures")
            .into();
        let mut document = StyledDocument::parse(
            "<style>@font-face{font-family:Fixed;src:url('../assets/DejaVuSans.ttf')}body{font-family:Fixed}p::after{content:target-counter('#target',page)}h2{break-before:page}</style><p>See </p><h2 id=target>Destination</h2>",
            &base, &[], &resolver,
        ).unwrap();
        let measurement = Budget::new(MAX_LAYOUT_WORK);
        crate::layout::layout_pass(&document, &resolver, measurement.clone()).unwrap();
        let first_pass_work = MAX_LAYOUT_WORK - measurement.remaining.get().unwrap();
        assert!(first_pass_work > 0);
        let result = crate::layout::stabilize(
            &mut document,
            &resolver,
            &Budget::new(first_pass_work),
            Vec::new(),
        );
        assert!(
            matches!(result, Err(LayoutError::WorkLimit { limit }) if limit == first_pass_work),
            "a budget that only fits the provisional pass must not publish that pass or reset for the next one"
        );
    }

    #[test]
    fn exhaustion_is_inclusive_overflow_safe_and_sticky() {
        let budget = Budget::new(8);
        budget.spend(3).unwrap();
        budget.spend(5).unwrap();
        assert!(matches!(budget.spend(1), Err(Exhausted { limit: 8 })));
        assert!(matches!(budget.spend(0), Err(Exhausted { limit: 8 })));
        let budget = Budget::new(usize::MAX - 1);
        assert!(matches!(budget.spend(usize::MAX), Err(Exhausted { .. })));
        assert!(budget.spend(1).is_err());
    }

    #[test]
    fn column_candidates_share_the_jobs_budget_without_poisoning_later_jobs() {
        const LIMIT: usize = 4096;
        let resolver = FileResolver::default();
        let content = "<div class=item></div>".repeat(320);
        let parse = |columns| {
            StyledDocument::parse(
                &format!(
                    "<!doctype html><style>@page{{size:200pt 200pt;margin:10pt}}html,body{{margin:0}}.item{{height:10pt}}section{{{columns}}}</style><section>{content}</section>"
                ),
                &ResourceBase::default(),
                &[],
                &resolver,
            ).unwrap()
        };
        let ordinary = parse("");
        let columns = parse("column-count:2;column-gap:10pt");
        let expected = layout_with_work_limit(&ordinary, &resolver, LIMIT).unwrap();
        match layout_with_work_limit(&columns, &resolver, LIMIT) {
            Err(LayoutError::WorkLimit { limit }) => assert_eq!(limit, LIMIT),
            Err(error) => panic!("unexpected column failure: {error}"),
            Ok(trace) => panic!(
                "column probes escaped the shared budget: {} pages",
                trace.pages.len()
            ),
        }
        let completed = layout_with_work_limit(&columns, &resolver, MAX_LAYOUT_WORK).unwrap();
        assert!(!completed.pages.is_empty());
        let next = layout_with_work_limit(&ordinary, &resolver, LIMIT).unwrap();
        assert_eq!(expected.pages.len(), next.pages.len());
    }

    #[test]
    fn automatic_columns_resume_each_row_within_the_shared_work_budget() {
        const LIMIT: usize = 16_384;
        const BLOCKS: usize = 256;
        const BLOCKS_PER_PAGE: usize = 8;
        let resolver = FileResolver::default();
        let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/oracle/fixtures")
            .into();
        let content = (0..BLOCKS)
            .map(|index| {
                format!("<div id=item{index}><a href=\"#item{index}\">B{index:03}</a></div>")
            })
            .collect::<String>();
        let document = StyledDocument::parse(
            &format!("<!doctype html><style>@font-face{{font-family:Fixed;src:url('../assets/DejaVuSans.ttf')}}@page{{size:200pt 40pt;margin:0}}html,body{{margin:0;font:10pt/10pt Fixed;widows:1;orphans:1}}section{{column-count:2;column-gap:10pt;column-fill:auto}}div{{height:10pt;background:red}}</style><section>{content}</section>"),
            &base, &[], &resolver,
        ).unwrap();
        assert_eq!(
            document.navigation().destinations,
            BLOCKS,
            "fixture must register each linked target"
        );
        let output = layout_with_work_limit(&document, &resolver, LIMIT).unwrap_or_else(|error| {
            panic!("each physical row must resume its source checkpoint: {error}")
        });
        assert_eq!(output.pages.len(), BLOCKS / BLOCKS_PER_PAGE);
        assert_eq!(
            output.destinations.iter().flatten().count(),
            BLOCKS,
            "every source target must be placed exactly once"
        );
        let actual = output
            .pages
            .iter()
            .flatten()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>();
        let expected = (0..BLOCKS)
            .map(|index| format!("B{index:03}"))
            .collect::<Vec<_>>();
        assert_eq!(
            actual, expected,
            "each visible source label must be preserved once"
        );
        for (index, destination) in output.destinations.iter().enumerate() {
            let destination = destination.as_ref().expect("every block keeps its target");
            assert_eq!(
                destination.page,
                index / BLOCKS_PER_PAGE,
                "item{index} was replayed or placed on the wrong physical page"
            );
        }
    }

    #[test]
    fn mixed_native_css_layout_and_raw_paint_share_a_bounded_measurement_cache() {
        const SEGMENTS: usize = 8;
        const OVER_BUDGET_SEGMENTS: usize = 12;
        const LIMIT: usize = 1_000_000;
        let resolver = FileResolver::default();
        let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/oracle/fixtures")
            .into();
        let open = "<mrow><mtext><span style='display:block;writing-mode:vertical-rl'><math>";
        let close = "</math></span></mtext></mrow>";
        let parse = |segments| {
            StyledDocument::parse(
            &format!("<!doctype html><style>@font-face{{font-family:Fixed;src:url('../assets/DejaVuSans.ttf')}}math{{font:10pt Fixed}}</style><math>{}<mn>1</mn>{}</math>", open.repeat(segments), close.repeat(segments)),
            &base, &[], &resolver,
        ).unwrap()
        };
        let document = parse(SEGMENTS);
        let check = || {
            let output = layout_with_work_limit(&document, &resolver, LIMIT)
                .unwrap_or_else(|error| panic!("{SEGMENTS} auto-sized mixed native/CSS boundaries must reuse measurement dependencies through raw paint within {LIMIT} shared units: {error}"));
            assert_eq!(
                output
                    .pages
                    .iter()
                    .flatten()
                    .filter(|line| line.text == "1")
                    .count(),
                1,
                "the single retained token must survive every native/CSS transport exactly once"
            );
        };
        check();
        assert!(
            matches!(
                layout_with_work_limit(&parse(OVER_BUDGET_SEGMENTS), &resolver, LIMIT),
                Err(LayoutError::WorkLimit { limit: LIMIT })
            ),
            "more auto-sized orthogonal candidates must exhaust the same shared budget with the typed error"
        );
        check();
    }

    #[test]
    fn wide_native_math_batches_siblings_under_one_linear_work_budget() {
        const TOKENS: usize = 8192;
        const WORK_PER_TOKEN: usize = 512;
        const LIMIT: usize = TOKENS * WORK_PER_TOKEN;
        let resolver = FileResolver::default();
        let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/oracle/fixtures")
            .into();
        let content = (0..TOKENS)
            .map(|index| format!("<mn id=token{index}>1</mn>"))
            .collect::<String>();
        let document = StyledDocument::parse(
            &format!("<!doctype html><style>@font-face{{font-family:Fixed;src:url('../assets/DejaVuSans.ttf')}}@page{{size:100000pt 100pt;margin:0}}html,body{{margin:0}}math{{font:10pt Fixed}}</style><math>{content}</math>"),
            &base, &[], &resolver,
        ).unwrap();
        assert_eq!(document.navigation().destinations, TOKENS);
        for limit in [MAX_LAYOUT_WORK, LIMIT] {
            let output = layout_with_work_limit(&document, &resolver, limit)
                .unwrap_or_else(|error| panic!("{TOKENS} native siblings must batch within {limit} shared work units: {error}"));
            assert_eq!(
                output.pages.len(),
                1,
                "the wide physical page must hold one native row"
            );
            let lines = output.pages[0]
                .iter()
                .filter(|line| line.text == "1")
                .collect::<Vec<_>>();
            assert_eq!(
                lines.len(),
                TOKENS,
                "every canonical token must paint exactly once"
            );
            assert!(
                output.pages[0].iter().all(|line| line.text == "1"
                    || line
                        .text
                        .chars()
                        .all(|ch| ch == crate::document::OBJECT_REPLACEMENT_CHARACTER)),
                "the host atomic placeholder must not introduce additional visible text"
            );
            assert!(
                lines
                    .windows(2)
                    .all(|pair| pair[0].paragraph < pair[1].paragraph)
                    && lines.iter().all(|line| line.width > 0.0),
                "batch replay must preserve canonical sibling order and positive glyph advances"
            );
            let mut paragraphs = lines.iter().map(|line| line.paragraph).collect::<Vec<_>>();
            paragraphs.sort_unstable();
            paragraphs.dedup();
            assert_eq!(
                paragraphs.len(),
                TOKENS,
                "native siblings must retain distinct canonical paragraph identities"
            );
            assert_eq!(
                output.destinations.iter().flatten().count(),
                TOKENS,
                "each canonical target must retain one page location"
            );
        }
        const EXHAUSTED_LIMIT: usize = TOKENS;
        assert!(
            matches!(
                layout_with_work_limit(&document, &resolver, EXHAUSTED_LIMIT),
                Err(LayoutError::WorkLimit {
                    limit: EXHAUSTED_LIMIT
                })
            ),
            "native frame requests must consume the shared job budget"
        );
        assert!(
            layout_with_work_limit(&document, &resolver, LIMIT).is_ok(),
            "a bounded failure must not poison the next render of the same retained source"
        );
    }

    #[test]
    fn balancing_stops_when_adjacent_heights_cannot_be_bisected() {
        const LIMIT: usize = 4096;
        let resolver = FileResolver::default();
        let document = StyledDocument::parse(
            "<!doctype html><style>@page{size:200pt 2000000pt;margin:0}html,body{margin:0}section{column-count:2;column-gap:0}div{height:1000000pt}</style><section><div></div><div></div></section>",
            &ResourceBase::default(),
            &[],
            &resolver,
        ).unwrap();
        let output = layout_with_work_limit(&document, &resolver, LIMIT)
            .unwrap_or_else(|error| panic!("finite two-column balance failed: {error}"));
        assert_eq!(output.pages.len(), 1);
    }

    #[test]
    fn repeated_table_geometry_consumes_the_same_budget() {
        const LIMIT: usize = 1024;
        const ROWS: usize = 32;
        let resolver = FileResolver::default();
        let body = "<tr><td><div></div></td></tr>".repeat(ROWS);
        let document = StyledDocument::parse(
            &format!("<!doctype html><style>@page{{size:200pt 200pt;margin:10pt}}html,body{{margin:0}}table{{border-spacing:0}}td{{padding:0}}thead div{{height:40pt}}tbody div{{height:100pt}}</style><table><thead><tr><td><div></div></td></tr></thead><tbody>{body}</tbody></table>"),
            &ResourceBase::default(), &[], &resolver,
        ).unwrap();
        assert!(
            matches!(
                layout_with_work_limit(&document, &resolver, LIMIT),
                Err(LayoutError::WorkLimit { limit: LIMIT })
            ),
            "repeated table measurement escaped the job budget"
        );
        let output = layout_with_work_limit(&document, &resolver, MAX_LAYOUT_WORK).unwrap();
        assert_eq!(
            output.pages.len(),
            ROWS,
            "each page fits one body row and its repeated header"
        );
    }

    #[test]
    fn generated_margin_text_shares_the_body_budget() {
        const LIMIT: usize = 4096;
        const PAGES: usize = 8;
        let resolver = FileResolver::default();
        let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/oracle/fixtures")
            .into();
        let body = "<div></div>".repeat(PAGES);
        let margin = "A ".repeat(256);
        let parse = |margin_rule| {
            StyledDocument::parse(
            &format!("<!doctype html><style>@font-face{{font-family:Fixed;src:url('../assets/DejaVuSans.ttf')}}@page{{size:200pt 200pt;margin:20pt;{margin_rule}}}html,body{{margin:0}}div{{height:160pt;background:#eee}}</style>{body}"),
            &base, &[], &resolver,
        ).unwrap()
        };
        let ordinary = parse("");
        assert_eq!(
            layout_with_work_limit(&ordinary, &resolver, LIMIT)
                .unwrap()
                .pages
                .len(),
            PAGES
        );
        let margins = parse(&format!(
            "@top-center{{font:1pt/1pt Fixed;content:'{margin}'}}"
        ));
        assert!(
            matches!(
                layout_with_work_limit(&margins, &resolver, LIMIT),
                Err(LayoutError::WorkLimit { limit: LIMIT })
            ),
            "generated text escaped the job budget"
        );
        let output = layout_with_work_limit(&margins, &resolver, MAX_LAYOUT_WORK).unwrap();
        assert_eq!(output.pages.len(), PAGES);
        for (page, lines) in output.pages.iter().enumerate() {
            let count = lines
                .iter()
                .flat_map(|line| line.text.split_whitespace())
                .count();
            assert_eq!(count, 256, "margin text lost on page {}", page + 1);
        }
    }
    /// The layout work `document` needs, overestimated by at most a `1 / PRECISION` fraction:
    /// layout charges the same units under every budget.
    fn required_work(document: &StyledDocument, resolver: &FileResolver) -> usize {
        const PRECISION: usize = 64;
        let fits = |limit| match layout_with_work_limit(document, resolver, limit) {
            Ok(_) => true,
            Err(LayoutError::WorkLimit { .. }) => false,
            Err(error) => panic!("layout failed with {error} instead of exhausting its budget"),
        };
        let (mut fails, mut fits_within) = (0, 1);
        while !fits(fits_within) {
            assert!(
                fits_within < MAX_LAYOUT_WORK,
                "the document must lay out within the layout budget"
            );
            fails = fits_within;
            fits_within *= 2;
        }
        while (fits_within - fails) * PRECISION > fits_within {
            let limit = fails + (fits_within - fails) / 2;
            if fits(limit) {
                fits_within = limit;
            } else {
                fails = limit;
            }
        }
        fits_within
    }

    #[test]
    fn footnote_parts_copy_only_the_lines_they_show() {
        const FOOTNOTES: usize = 96;
        let resolver = FileResolver::default();
        let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/oracle/fixtures")
            .into();
        // One inline footnote body, laid out once, continues across every page; each page's
        // part of it copies the lines it shows, not the whole body.
        let work = |count: usize| {
            let calls = (0..count)
                .map(|index| format!("w{index}<span class=fn>note {index}</span> "))
                .collect::<String>();
            let document = StyledDocument::parse(
                &format!("<!doctype html><style>@font-face{{font-family:Fixed;src:url('../assets/DejaVuSans.ttf')}}@page{{size:120pt 120pt;margin:10pt}}html,body{{margin:0;font:10pt/10pt Fixed}}.fn{{float:footnote;footnote-display:inline}}</style><p>{calls}</p>"),
                &base, &[], &resolver,
            )
            .unwrap();
            required_work(&document, &resolver)
        };
        let (single, doubled) = (work(FOOTNOTES), work(2 * FOOTNOTES));
        assert!(
            doubled < 3 * single,
            "doubling the footnotes doubles the pages and the body's lines, so copying each part \
             whole would quadruple the charged work: {FOOTNOTES} footnotes need {single} units, \
             {} need {doubled}",
            2 * FOOTNOTES
        );
    }

    #[test]
    fn fixed_column_overflow_without_room_for_cloned_edges_fails_before_exhausting_work() {
        const LIMIT: usize = 4096;
        let resolver = FileResolver::default();
        let document = StyledDocument::parse(
            "<!doctype html><style>@page{size:200pt 200pt;margin:10pt}html,body{margin:0}section{column-count:2;column-gap:10pt;column-fill:auto;height:10pt}div{height:100pt;padding:10pt;box-decoration-break:clone;background:blue}</style><section><div></div></section>",
            &ResourceBase::default(), &[], &resolver,
        ).unwrap();
        match layout_with_work_limit(&document, &resolver, LIMIT) {
            Err(LayoutError::PageLimit { limit }) => assert_eq!(limit, MAX_LAYOUT_PAGES),
            Err(error) => panic!("a fixed column cannot make progress past cloned edges: {error}"),
            Ok(trace) => panic!(
                "impossible fixed columns must not return {} pages",
                trace.pages.len()
            ),
        }
    }
}
