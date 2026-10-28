use crate::document::DocumentError;
use crate::style::{StylesheetInput, StylesheetSources};
use ferri_pdf_dom::{DocumentTree, HTML_NAMESPACE, SVG_NAMESPACE};
use ferri_pdf_resources::{CSS_MEDIA_TYPE, ResourceBase, ResourceMetadata};

const STYLESHEET: &str = "stylesheet";

pub(super) fn discover(
    tree: &DocumentTree,
    base: &ResourceBase,
    encoding: &str,
    author: &[String],
    user: &[String],
) -> Result<StylesheetSources, DocumentError> {
    let declared = tree
        .elements()
        .find(|element| {
            element.namespace() == HTML_NAMESPACE
                && element.name() == "base"
                && element.attr("href").is_some()
        })
        .and_then(|element| element.attr("href"))
        .map(|reference| base.resolve(reference).map(ResourceBase::Url))
        .transpose()?;
    let effective = declared.as_ref().unwrap_or(base);
    let mut candidates = Vec::new();
    for element in tree.shadow_including_elements() {
        let style = element.name() == "style"
            && matches!(element.namespace(), HTML_NAMESPACE | SVG_NAMESPACE);
        let link = element.name() == "link" && element.namespace() == HTML_NAMESPACE;
        if !style && !link {
            continue;
        }
        if style && element.namespace() == HTML_NAMESPACE {
            if element.attr("type").is_some_and(|value| {
                !value.is_empty() && !value.eq_ignore_ascii_case(CSS_MEDIA_TYPE)
            }) {
                continue;
            }
        } else if element.attr("type").is_some_and(|value| {
            !value.is_empty()
                && ResourceMetadata::from_content_type(value)
                    .media_type
                    .as_deref()
                    != Some(CSS_MEDIA_TYPE)
        }) {
            continue;
        }
        if link {
            if element.attr("disabled").is_some() {
                continue;
            }
            if !element.has_rel(STYLESHEET) {
                continue;
            }
            if element
                .attr("href")
                .is_none_or(|reference| reference.trim_ascii().is_empty())
            {
                continue;
            }
        }
        let media = element.attr("media").unwrap_or_default().to_owned();
        let owner = Some(element.id());
        let input = if style {
            StylesheetInput::Inline {
                text: element.text().collect(),
                base: effective.clone(),
                media,
                owner,
                fallback_encoding: encoding.into(),
            }
        } else {
            StylesheetInput::External {
                reference: element
                    .attr("href")
                    .expect("nonempty href checked")
                    .trim_ascii()
                    .to_owned(),
                base: effective.clone(),
                media,
                owner,
                fallback_encoding: crate::document::encoding::css_environment(
                    element.attr("charset"),
                    encoding,
                )
                .into(),
            }
        };
        candidates.push(input);
    }
    let inline = |text: &String| StylesheetInput::Inline {
        text: text.clone(),
        base: effective.clone(),
        media: String::new(),
        owner: None,
        fallback_encoding: encoding.into(),
    };
    Ok(StylesheetSources {
        base: effective.clone(),
        document_encoding: encoding.into(),
        document_url: match base {
            ResourceBase::Url(url) => Some(url.clone()),
            ResourceBase::Directory(_) => None,
        },
        document: candidates,
        author: author.iter().map(inline).collect(),
        user: user.iter().map(inline).collect(),
    })
}
