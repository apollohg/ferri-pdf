use crate::{Element, HTML_NAMESPACE};

const HEADING_TAGS: &[&str] = &["h1", "h2", "h3", "h4", "h5", "h6"];
const FIRST_HEADING_LEVEL: u8 = 1;
const MAX_HEADING_LEVEL: u8 = 9;
const DECIMAL_RADIX: u8 = 10;
const HEADING_OFFSET: &str = "headingoffset";
const HEADING_RESET: &str = "headingreset";

/// The digit prefix of an HTML non-negative integer, with its sign validated.
pub fn non_negative_integer_digits(value: &str) -> Option<&str> {
    let (negative, digits) = integer_digits(value)?;
    (!negative || digits.bytes().all(|digit| digit == b'0')).then_some(digits)
}

/// The `(negative, digits)` prefix of an HTML integer, without a numeric size limit.
pub fn integer_digits(value: &str) -> Option<(bool, &str)> {
    let value = value.trim_start_matches(|ch: char| ch.is_ascii_whitespace());
    let (negative, value) = if let Some(value) = value.strip_prefix('-') {
        (true, value)
    } else {
        (false, value.strip_prefix('+').unwrap_or(value))
    };
    let count = value.bytes().take_while(u8::is_ascii_digit).count();
    let digits = &value[..count];
    (!digits.is_empty()).then_some((negative, digits))
}

impl Element<'_> {
    pub fn has_rel(self, token: &str) -> bool {
        self.attr("rel").is_some_and(|value| {
            value
                .split_ascii_whitespace()
                .any(|item| item.eq_ignore_ascii_case(token))
        })
    }

    /// HTML computed heading level, including inclusive offsets and reset boundaries.
    pub fn heading_level(self) -> Option<u8> {
        if self.namespace() != HTML_NAMESPACE {
            return None;
        }
        let mut level =
            HEADING_TAGS.iter().position(|tag| *tag == self.name())? as u8 + FIRST_HEADING_LEVEL;
        for ancestor in
            std::iter::successors(Some(self), |element| element.shadow_including_parent())
        {
            if ancestor.namespace() != HTML_NAMESPACE {
                continue;
            }
            if let Some(digits) = ancestor
                .attr(HEADING_OFFSET)
                .and_then(non_negative_integer_digits)
            {
                let offset = digits.bytes().fold(0u8, |offset, digit| {
                    offset
                        .saturating_mul(DECIMAL_RADIX)
                        .saturating_add(digit - b'0')
                        .min(MAX_HEADING_LEVEL)
                });
                level = level.saturating_add(offset).min(MAX_HEADING_LEVEL);
            }
            if ancestor.attr(HEADING_RESET).is_some() {
                break;
            }
        }
        Some(level)
    }
}

#[cfg(test)]
mod tests {
    use crate::{DocumentTree, Selector, XmlTree};

    #[test]
    fn css_attribute_lookup_respects_html_foreign_and_xml_name_rules() {
        let tree = DocumentTree::parse(
            "<p data-x='Value' data-empty='' data-Σ='Upper'></p><svg viewBox='0 0 20 10'></svg>",
        )
        .unwrap();
        let html = tree.select(&Selector::parse("p").unwrap()).next().unwrap();
        assert_eq!(html.attr_in_namespace(None, "DATA-X"), Some("Value"));
        assert_eq!(html.attr_in_namespace(None, "data-empty"), Some(""));
        assert_eq!(html.attr_in_namespace(None, "missing"), None);
        assert_eq!(
            html.attr_in_namespace(None, "data-σ"),
            None,
            "HTML folds ASCII attribute names only"
        );
        let svg = tree
            .select(&Selector::parse("svg").unwrap())
            .next()
            .unwrap();
        assert_eq!(svg.attr_in_namespace(None, "viewBox"), Some("0 0 20 10"));
        assert_eq!(
            svg.attr_in_namespace(None, "VIEWBOX"),
            None,
            "foreign attribute names are case-sensitive"
        );
        const ATTRIBUTE_NAMESPACE: &str = "urn:data";
        let source = format!(
            "<root xmlns='{}' xmlns:d='{ATTRIBUTE_NAMESPACE}' d:Size='17pt' Size='29pt' size='31pt'/>",
            crate::HTML_NAMESPACE
        );
        let document = roxmltree::Document::parse(&source).unwrap();
        let xml = XmlTree::new(&document);
        let element = xml.element(document.root_element()).unwrap();
        assert_eq!(
            element.attr_in_namespace(Some(ATTRIBUTE_NAMESPACE), "Size"),
            Some("17pt")
        );
        assert_eq!(
            element.attr_in_namespace(Some(ATTRIBUTE_NAMESPACE), "size"),
            None
        );
        assert_eq!(element.attr_in_namespace(None, "Size"), Some("29pt"));
        assert_eq!(
            element.attr_in_namespace(None, "size"),
            Some("31pt"),
            "XML remains case-sensitive even in the HTML namespace"
        );
    }
}
