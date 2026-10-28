use crate::layout::LayoutError;
use std::{cell::Cell, rc::Rc};

#[derive(Clone)]
pub(super) struct Budget {
    limit: usize,
    remaining: Rc<Cell<Option<usize>>>,
}

impl Budget {
    pub(crate) fn new(limit: usize) -> Self {
        Self {
            limit,
            remaining: Rc::new(Cell::new(Some(limit))),
        }
    }

    pub(crate) fn spend(&self, units: usize) -> Result<(), LayoutError> {
        let remaining = self
            .remaining
            .get()
            .and_then(|left| left.checked_sub(units));
        self.remaining.set(remaining);
        remaining
            .map(|_| ())
            .ok_or(LayoutError::WorkLimit { limit: self.limit })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::{MAX_LAYOUT_PAGES, MAX_LAYOUT_WORK, layout_with_work_limit};
    use ferri_pdf_document::StyledDocument;
    use ferri_pdf_resources::{FileResolver, ResourceBase};

    #[test]
    fn target_stabilization_charges_all_passes_to_one_budget() {
        let resolver = FileResolver::default();
        let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/oracle/fixtures")
            .into();
        let document = StyledDocument::parse(
            "<style>@font-face{font-family:Fixed;src:url('../assets/DejaVuSans.ttf')}body{font-family:Fixed}p::after{content:target-counter('#target',page)}h2{break-before:page}</style><p>See </p><h2 id=target>Destination</h2>",
            &base, &[], &resolver,
        ).unwrap();
        let measurement = Budget::new(MAX_LAYOUT_WORK);
        crate::layout::layout_pass(&document, &resolver, measurement.clone()).unwrap();
        let first_pass_work = MAX_LAYOUT_WORK - measurement.remaining.get().unwrap();
        assert!(first_pass_work > 0);
        let result = crate::layout::stabilize(
            document,
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
        assert!(matches!(
            budget.spend(1),
            Err(LayoutError::WorkLimit { limit: 8 })
        ));
        assert!(matches!(
            budget.spend(0),
            Err(LayoutError::WorkLimit { limit: 8 })
        ));
        let budget = Budget::new(usize::MAX - 1);
        assert!(matches!(
            budget.spend(usize::MAX),
            Err(LayoutError::WorkLimit { .. })
        ));
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
