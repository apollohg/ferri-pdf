use super::{
    Arc, Component, Keyword, Label, LineNames, MAX_GRID_LINE, Production, Serialize, StyleError,
    line_names, repeat_arguments, unsupported,
};

/// An auto-fill repetition of line-name sets, without track sizes.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct NameRepeat {
    pub at: usize,
    pub lines: Vec<LineNames>,
}

/// Local line names on a subgridded axis; adjacent sets name distinct lines.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Subgrid {
    pub lines: Vec<LineNames>,
    pub repeat: Option<NameRepeat>,
}

impl Subgrid {
    /// Local names for a used line, after expanding complete auto-fill repetitions.
    pub fn line_names(&self, tracks: usize, line: usize) -> &[Arc<str>] {
        if line > tracks {
            return &[];
        }
        let mut index = line;
        if let Some(repeat) = &self.repeat {
            let count = repeat.lines.len();
            if count != 0 && repeat.at <= self.lines.len() {
                let available = tracks.saturating_add(1).saturating_sub(self.lines.len());
                let filled = available - available % count;
                if let Some(offset) = line.checked_sub(repeat.at) {
                    if offset < filled {
                        return &repeat.lines[offset % count];
                    }
                    index -= filled;
                }
            }
        }
        self.lines.get(index).map(Vec::as_slice).unwrap_or_default()
    }

    pub(super) fn parse(
        value: &Component,
        relative: &crate::units::Relative<'_>,
    ) -> Result<Self, StyleError> {
        let mut output = Self::default();
        let mut parts = Vec::new();
        value.references(
            &[
                Label::Production(Production::LineNames),
                Label::Production(Production::NameRepeat),
            ],
            &mut parts,
        );
        for (label, part) in parts {
            match label {
                Label::Production(Production::LineNames) => {
                    if output.lines.len() < MAX_GRID_LINE as usize {
                        let names: LineNames = line_names(part).collect();
                        relative.work.charge(names.len().max(1))?;
                        output.lines.push(names);
                    }
                }
                Label::Production(Production::NameRepeat) => {
                    let (count, body) = repeat_arguments(part)?;
                    let mut names = Vec::new();
                    for item in body {
                        item.references(&[Label::Production(Production::LineNames)], &mut names);
                    }
                    let names: Vec<LineNames> = names
                        .into_iter()
                        .map(|(_, names)| line_names(names).collect())
                        .collect();
                    if count.keyword() == Some(Keyword::AutoFill) {
                        if output.repeat.is_some() {
                            return Err(unsupported(value));
                        }
                        relative
                            .work
                            .charge(names.iter().map(|line| line.len().max(1)).sum())?;
                        output.repeat = Some(NameRepeat {
                            at: output.lines.len(),
                            lines: names,
                        });
                    } else {
                        let count = usize::try_from(crate::values::integer(count, relative)?)
                            .map_err(|_| unsupported(count))?;
                        for names in names.iter().cycle().take(count.saturating_mul(names.len())) {
                            if output.lines.len() == MAX_GRID_LINE as usize {
                                break;
                            }
                            relative.work.charge(names.len().max(1))?;
                            output.lines.push(names.clone());
                        }
                    }
                }
                _ => return Err(unsupported(part)),
            }
        }
        Ok(output)
    }
}

pub(super) fn valid(value: &Component) -> bool {
    let mut lists = Vec::new();
    value.references(&[Label::Production(Production::LineNameList)], &mut lists);
    lists.into_iter().all(|(_, list)| {
        let mut repeats = Vec::new();
        list.references(&[Label::Production(Production::NameRepeat)], &mut repeats);
        repeats
            .into_iter()
            .filter(|(_, repeat)| {
                repeat_arguments(repeat)
                    .is_ok_and(|(count, _)| count.keyword() == Some(Keyword::AutoFill))
            })
            .take(2)
            .count()
            <= 1
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grammar::Property;
    use crate::grid_values::template;

    fn computed(source: &str) -> Subgrid {
        let value = crate::declaration::value(Property::GridTemplateColumns, source, &mut 0)
            .unwrap()
            .expect("valid subgrid grammar");
        template(&value, &crate::units::Relative::initial())
            .unwrap()
            .subgrid
            .expect("subgrid computed value")
    }

    fn names(value: &Subgrid) -> Vec<Vec<&str>> {
        value
            .lines
            .iter()
            .map(|line| line.iter().map(AsRef::as_ref).collect())
            .collect()
    }

    #[test]
    fn resolved_names_fill_whole_patterns_and_keep_suffix_positions() {
        let value = computed("subgrid [start] repeat(auto-fill, [a] [b]) [end]");
        let resolve = |tracks| {
            (0..=tracks)
                .map(|line| {
                    value
                        .line_names(tracks, line)
                        .iter()
                        .map(AsRef::as_ref)
                        .collect::<Vec<&str>>()
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(
            resolve(5),
            vec![
                vec!["start"],
                vec!["a"],
                vec!["b"],
                vec!["a"],
                vec!["b"],
                vec!["end"]
            ]
        );
        assert_eq!(
            resolve(4),
            vec![vec!["start"], vec!["a"], vec!["b"], vec!["end"], vec![]]
        );
        assert_eq!(resolve(1), vec![vec!["start"], vec!["end"]]);
        assert_eq!(resolve(2), vec![vec!["start"], vec!["end"], vec![]]);
        assert_eq!(value.line_names(4, 5), []);
        let value = computed("subgrid [] [a] [b] [c]");
        assert_eq!(value.line_names(1, 0), []);
        assert_eq!(value.line_names(1, 1)[0].as_ref(), "a");
        assert_eq!(value.line_names(1, 2), []);
    }

    #[test]
    fn fixed_subgrid_repeats_keep_distinct_and_empty_line_sets() {
        let value = computed("subgrid [a] [] repeat(2, [b] [c]) [d]");
        assert_eq!(
            names(&value),
            vec![
                vec!["a"],
                vec![],
                vec!["b"],
                vec!["c"],
                vec!["b"],
                vec!["c"],
                vec!["d"]
            ]
        );
        assert!(value.repeat.is_none());
        assert_eq!(computed("subgrid").lines, [] as [Vec<Arc<str>>; 0]);
    }

    #[test]
    fn auto_fill_retains_its_insertion_point_between_fixed_name_sets() {
        let value = computed("subgrid repeat(2, [a]) repeat(auto-fill, [] [b c]) [end]");
        assert_eq!(names(&value), vec![vec!["a"], vec!["a"], vec!["end"]]);
        let repeat = value.repeat.unwrap();
        assert_eq!(repeat.at, 2);
        assert_eq!(
            repeat.lines,
            vec![vec![], vec![Arc::<str>::from("b"), Arc::<str>::from("c")]]
        );
    }

    #[test]
    fn fixed_name_expansion_obeys_the_existing_grid_line_limit() {
        let value = computed("subgrid repeat(1000000, [a] []) [outside]");
        assert_eq!(value.lines.len(), MAX_GRID_LINE as usize);
        for (index, line) in value.lines.iter().enumerate() {
            assert_eq!(line.is_empty(), index % 2 == 1);
        }
    }

    #[test]
    fn repeated_name_work_is_bounded_before_excessive_expansion() {
        const NAMES_PER_LINE: usize = 200;
        let names = (0..NAMES_PER_LINE)
            .map(|index| format!("name{index}"))
            .collect::<Vec<_>>()
            .join(" ");
        let source = format!("subgrid repeat({MAX_GRID_LINE}, [{names}])");
        let value = crate::declaration::value(Property::GridTemplateColumns, &source, &mut 0)
            .unwrap()
            .unwrap();
        assert!(matches!(
            template(&value, &crate::units::Relative::initial()),
            Err(StyleError::GrammarLimit {
                resource: "value computation work",
                ..
            })
        ));
    }

    #[test]
    fn subgrid_support_matches_runtime_integration() {
        for source in ["subgrid", "subgrid [a]", "subgrid repeat(auto-fill, [a])"] {
            assert!(crate::declaration::supports_declaration(
                "grid-template-columns",
                source
            ));
        }
    }

    #[test]
    fn auto_fill_is_limited_per_axis_in_longhands_and_shorthands() {
        for (property, source, valid) in [
            (
                Property::GridTemplateColumns,
                "subgrid repeat(auto-fill, [a]) repeat(auto-fill, [b])",
                false,
            ),
            (
                Property::GridTemplate,
                "subgrid repeat(auto-fill, [a]) / subgrid repeat(auto-fill, [b])",
                true,
            ),
            (
                Property::GridTemplate,
                "subgrid repeat(auto-fill, [a]) repeat(auto-fill, [b]) / 1fr",
                false,
            ),
        ] {
            assert_eq!(
                crate::declaration::value(property, source, &mut 0)
                    .unwrap()
                    .is_some(),
                valid,
                "{source}"
            );
        }
    }
}
