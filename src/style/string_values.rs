use crate::style::{
    GeneratedContent, StyleError,
    box_values::unsupported,
    component::Component,
    grammar::{Keyword, Label, Production},
};
use std::sync::{Arc, LazyLock};

#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub(crate) struct StringAssignment {
    pub name: Arc<str>,
    pub content: GeneratedContent,
}

static EMPTY_ASSIGNMENTS: LazyLock<Arc<[StringAssignment]>> = LazyLock::new(Arc::default);

pub(crate) fn empty() -> Arc<[StringAssignment]> {
    EMPTY_ASSIGNMENTS.clone()
}

pub(crate) fn assignments(value: &Component) -> Result<Arc<[StringAssignment]>, StyleError> {
    if value.keyword() == Some(Keyword::None) {
        return Ok(empty());
    }
    let mut parts = Vec::new();
    assignment_parts(value, &mut parts);
    let (pairs, remainder) = parts.as_chunks::<2>();
    let assignments = pairs
        .iter()
        .map(|[name, content]| {
            let Component::Ident(name) = name.unwrap() else {
                return Err(unsupported(name));
            };
            let Component::Reference(Label::Production(Production::NamedStringContent), content) =
                content
            else {
                return Err(unsupported(content));
            };
            Ok(StringAssignment {
                name: Arc::from(name.as_ref()),
                content: GeneratedContent::from_list(content)?,
            })
        })
        .collect::<Result<Vec<_>, StyleError>>()?;
    if !remainder.is_empty() || assignments.is_empty() {
        return Err(unsupported(value));
    }
    Ok(assignments.into())
}

fn assignment_parts<'a>(value: &'a Component, parts: &mut Vec<&'a Component>) {
    match value {
        Component::Reference(Label::Production(Production::NamedStringContent), _) => {
            parts.push(value);
        }
        Component::Reference(_, inner) => assignment_parts(inner, parts),
        Component::Group(items) => {
            for item in items {
                assignment_parts(item, parts);
            }
        }
        Component::Delim(',') => {}
        _ => parts.push(value),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::{
        QuoteDepth, Quotes,
        grammar::Property,
        matcher::{ValueContext, match_value},
    };

    const TEST_CONTENT_LIMIT: usize = 1024;

    fn parse(source: &str) -> Arc<[StringAssignment]> {
        let component = match_value(
            Property::StringSet.data().root,
            source,
            &mut 0,
            ValueContext::Element,
        )
        .expect("string-set grammar must match within its work budget")
        .unwrap_or_else(|| panic!("invalid string-set test input: {source}"));
        assignments(&component).unwrap_or_else(|error| panic!("cannot compute {source}: {error}"))
    }

    #[test]
    fn assignment_lists_keep_names_order_empty_values_and_counter_arguments() {
        let assignments = parse(
            "heading 'Chapter ' counter(chapter, upper-roman), footer counters(section, '.', lower-alpha), heading ''",
        );
        assert_eq!(
            assignments
                .iter()
                .map(|assignment| assignment.name.as_ref())
                .collect::<Vec<_>>(),
            ["heading", "footer", "heading"]
        );
        let values = assignments
            .iter()
            .map(|assignment| {
                assignment
                    .content
                    .element_text_with_counters(
                        TEST_CONTENT_LIMIT,
                        &mut |request| match request.name.as_ref() {
                            "chapter" => {
                                assert_eq!(request.style, crate::style::ListStyleType::UpperRoman);
                                assert_eq!(request.separator, None);
                                Ok(request.style.representation(4))
                            }
                            "section" => {
                                assert_eq!(request.style, crate::style::ListStyleType::LowerAlpha);
                                assert_eq!(request.separator.as_deref(), Some("."));
                                Ok("a.b".to_owned())
                            }
                            name => panic!("unexpected counter request {name}"),
                        },
                        &Quotes::default(),
                        &mut QuoteDepth::default(),
                    )
                    .expect("assignment text must preserve its counter expression")
            })
            .collect::<Vec<_>>();
        assert_eq!(values, ["Chapter IV", "a.b", ""]);
        assert!(parse("none").is_empty());
    }
    #[test]
    fn assignment_binding_captures_element_values_but_retains_page_counters() {
        let assignments =
            parse("heading content(before) counter(chapter) '/' counter(page) ':' counter(pages)");
        let mut requests = Vec::new();
        let bound = assignments[0]
            .content
            .bind_element_sources(
                TEST_CONTENT_LIMIT,
                &mut |source| {
                    assert_eq!(source, crate::style::ElementContent::Before);
                    Ok("Chapter ".into())
                },
                &mut |request| {
                    requests.push(request.name.to_string());
                    Ok("3".into())
                },
            )
            .expect("document values can be bound before pagination");
        assert_eq!(requests, ["chapter"]);
        assert_eq!(
            bound.page_text(2, 7, TEST_CONTENT_LIMIT).unwrap(),
            "Chapter 3/2:7"
        );
        assert_eq!(
            bound.page_text(4, 9, TEST_CONTENT_LIMIT).unwrap(),
            "Chapter 3/4:9"
        );
        let limit = "Chapter 3/2:7".len() - 1;
        assert!(
            matches!(bound.page_text(2, 7, limit), Err(StyleError::GeneratedContentLimit { limit: found }) if found == limit)
        );
        let assignment = parse("heading 'AB' content(text)");
        let limit = 4;
        assert!(matches!(assignment[0].content.bind_element_sources(
            limit, &mut |_| Ok("CDE".into()), &mut |_| unreachable!("no counter in fixture")
        ), Err(StyleError::GeneratedContentLimit { limit: found }) if found == limit));
    }
}
