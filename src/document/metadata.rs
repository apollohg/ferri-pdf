//! Document metadata from the HTML head (HTML §4.2.2, §4.2.5.1, §4.2.5.2) and the root
//! element's language (HTML §3.2.6.2).
use crate::document::content::collapse;
use crate::dom::{DocumentTree, HTML_NAMESPACE};

const TITLE_ELEMENT: &str = "title";
const META_ELEMENT: &str = "meta";
const NAME_ATTRIBUTE: &str = "name";
const CONTENT_ATTRIBUTE: &str = "content";
const LANG_ATTRIBUTE: &str = "lang";
const AUTHOR: &str = "author";
const SUBJECT: &str = "subject";
const KEYWORDS: &str = "keywords";
const GENERATOR: &str = "generator";
const KEYWORD_SEPARATOR: char = ',';

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct DocumentMetadata {
    /// The first `title` element's text with white space stripped and collapsed.
    pub title: Option<String>,
    /// Every `author` name, each naming one of the authors.
    pub authors: Vec<String>,
    pub subject: Option<String>,
    /// Every `keywords` name's comma-separated tokens.
    pub keywords: Vec<String>,
    /// The software that generated the document.
    pub generator: Option<String>,
    /// The root element's `lang`.
    pub language: Option<String>,
}
impl DocumentMetadata {
    pub(crate) fn from_html(html: &DocumentTree) -> Self {
        let elements: Vec<_> = html
            .elements()
            .filter(|element| element.namespace() == HTML_NAMESPACE)
            .collect();
        let meta = |name: &'static str| {
            elements
                .iter()
                .filter(|element| element.value().name() == META_ELEMENT)
                .filter(move |element| {
                    element
                        .attr(NAME_ATTRIBUTE)
                        .is_some_and(|value| value.eq_ignore_ascii_case(name))
                })
                .filter_map(|element| element.attr(CONTENT_ATTRIBUTE))
                .map(str::trim)
        };
        let first = |name: &'static str| {
            meta(name)
                .find(|value| !value.is_empty())
                .map(str::to_owned)
        };
        Self {
            title: elements
                .iter()
                .find(|element| element.value().name() == TITLE_ELEMENT)
                .map(|title| collapse(&title.text().collect::<String>()))
                .filter(|title| !title.is_empty()),
            authors: meta(AUTHOR)
                .filter(|author| !author.is_empty())
                .map(str::to_owned)
                .collect(),
            subject: first(SUBJECT),
            keywords: meta(KEYWORDS)
                .flat_map(|keywords| keywords.split(KEYWORD_SEPARATOR))
                .map(str::trim)
                .filter(|keyword| !keyword.is_empty())
                .map(str::to_owned)
                .collect(),
            generator: first(GENERATOR),
            language: html
                .root_element()
                .attr(LANG_ATTRIBUTE)
                .map(str::trim)
                .filter(|language| !language.is_empty())
                .map(str::to_owned),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_follows_html_name_rules() {
        let html = DocumentTree::parse(
            "<html lang=' en-AU '><head><title>  Spaced\n  title </title>\
             <meta name=AUTHOR content=' First '><meta name=author content=''><meta name=author content='Second'>\
             <meta name=keywords content=' a ,b,, c '><meta name=Keywords content='d'>\
             <meta name=subject content='Topic'><meta name=description content='Ignored'>\
             <meta name=generator content=''><meta name=generator content='Tool 2'></head>\
             <body><svg><title>Not the document title</title></svg></body></html>",
        );
        assert_eq!(
            DocumentMetadata::from_html(&html),
            DocumentMetadata {
                title: Some("Spaced title".into()),
                authors: vec!["First".into(), "Second".into()],
                subject: Some("Topic".into()),
                keywords: vec!["a".into(), "b".into(), "c".into(), "d".into()],
                generator: Some("Tool 2".into()),
                language: Some("en-AU".into()),
            }
        );
    }

    #[test]
    fn svg_titles_and_empty_values_give_no_metadata() {
        let html = DocumentTree::parse(
            "<html lang=''><title> </title><body><svg><title>Drawing</title></svg></body></html>",
        );
        assert_eq!(
            DocumentMetadata::from_html(&html),
            DocumentMetadata::default()
        );
    }
}
