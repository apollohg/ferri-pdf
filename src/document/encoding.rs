//! Byte decoding converges on the existing HTML tree builder.
use crate::dom::DocumentTree;
use encoding_rs::{Encoding, UTF_8, UTF_16BE, UTF_16LE, WINDOWS_1252, X_USER_DEFINED};
use std::ops::ControlFlow;

const CHARSET_ATTRIBUTE: &str = "charset";
const XML_DECLARATION: &[u8] = b"<?xml";
const XML_ENCODING: &[u8] = b"encoding";
const UTF16_LE_XML: &[u8] = b"<\0?\0x\0";
const UTF16_BE_XML: &[u8] = b"\0<\0?\0x";
const CSS_ENCODING_PREFIX: &[u8] = b"@charset \"";
const CSS_ENCODING_SCAN_BYTES: usize = 1024;

#[derive(Debug)]
pub(crate) struct DecodedInput {
    pub text: String,
    pub encoding: String,
    pub had_errors: bool,
}

pub(crate) type DecodedHtml = DecodedInput;
pub(crate) type DecodedCss = DecodedInput;

fn label(label: &str) -> Option<&'static Encoding> {
    Encoding::for_label(label.as_bytes())
}

fn decoded(bytes: &[u8], encoding: &'static Encoding) -> DecodedInput {
    let (text, encoding, had_errors) = encoding.decode(bytes);
    DecodedInput {
        text: text.into_owned(),
        encoding: encoding.name().into(),
        had_errors,
    }
}

fn html_declared_encoding(encoding: &'static Encoding) -> &'static Encoding {
    if encoding == UTF_16BE || encoding == UTF_16LE {
        UTF_8
    } else if encoding == X_USER_DEFINED {
        WINDOWS_1252
    } else {
        encoding
    }
}

fn meta_content_encoding(content: &str) -> Option<&'static Encoding> {
    let mut rest = content.as_bytes();
    let keyword = CHARSET_ATTRIBUTE.as_bytes();
    while let Some(index) = rest
        .windows(keyword.len())
        .position(|part| part.eq_ignore_ascii_case(keyword))
    {
        rest = &rest[index + keyword.len()..];
        while rest.first().is_some_and(u8::is_ascii_whitespace) {
            rest = &rest[1..];
        }
        if rest.first() != Some(&b'=') {
            continue;
        }
        rest = &rest[1..];
        while rest.first().is_some_and(u8::is_ascii_whitespace) {
            rest = &rest[1..];
        }
        let first = *rest.first()?;
        let value = if matches!(first, b'\'' | b'"') {
            rest = &rest[1..];
            &rest[..rest.iter().position(|byte| *byte == first)?]
        } else {
            let end = rest
                .iter()
                .position(|byte| byte.is_ascii_whitespace() || *byte == b';')
                .unwrap_or(rest.len());
            &rest[..end]
        };
        return Encoding::for_label(value);
    }
    None
}

fn xml_encoding(bytes: &[u8]) -> Option<&'static Encoding> {
    if bytes.starts_with(UTF16_LE_XML) {
        return Some(UTF_16LE);
    }
    if bytes.starts_with(UTF16_BE_XML) {
        return Some(UTF_16BE);
    }
    if !bytes.starts_with(XML_DECLARATION) {
        return None;
    }
    let end = bytes.iter().position(|byte| *byte == b'>')?;
    let start = bytes[..end]
        .windows(XML_ENCODING.len())
        .position(|part| part == XML_ENCODING)?
        + XML_ENCODING.len();
    let mut rest = &bytes[start..];
    while rest.first().is_some_and(|byte| *byte <= b' ') {
        rest = &rest[1..];
    }
    if rest.first() != Some(&b'=') {
        return None;
    }
    rest = &rest[1..];
    while rest.first().is_some_and(|byte| *byte <= b' ') {
        rest = &rest[1..];
    }
    let quote = *rest.first()?;
    if !matches!(quote, b'\'' | b'"') {
        return None;
    }
    rest = &rest[1..];
    let value = &rest[..rest.iter().position(|byte| *byte == quote)?];
    if value.iter().any(|byte| *byte <= b' ') {
        return None;
    }
    Encoding::for_label(value).map(|encoding| {
        if encoding == UTF_16BE || encoding == UTF_16LE {
            UTF_8
        } else {
            encoding
        }
    })
}

fn parse_html(
    text: &str,
    current_encoding: Option<&'static Encoding>,
) -> Result<ControlFlow<&'static Encoding, DocumentTree>, crate::dom::DomError> {
    let mut tentative_encoding = current_encoding;
    DocumentTree::parse_with_encoding(text, |declaration| {
        if tentative_encoding.is_some() {
            let encoding = declaration
                .charset
                .and_then(label)
                .or_else(|| declaration.content_type.and_then(meta_content_encoding));
            if let Some(encoding) = encoding {
                let encoding = html_declared_encoding(encoding);
                if Some(encoding) != tentative_encoding {
                    return ControlFlow::Break(encoding);
                }
                tentative_encoding = None;
            }
        }
        ControlFlow::Continue(())
    })
}

pub(crate) fn decode_document(
    bytes: &[u8],
    transport_charset: Option<&str>,
) -> Result<(DecodedHtml, DocumentTree), crate::dom::DomError> {
    let bom = Encoding::for_bom(bytes).map(|(encoding, _)| encoding);
    let transport = transport_charset.and_then(label);
    let encoding = bom
        .or(transport)
        .or_else(|| xml_encoding(bytes))
        .unwrap_or(UTF_8);
    let certain =
        bom.is_some() || transport.is_some() || encoding == UTF_16BE || encoding == UTF_16LE;
    let mut result = decoded(bytes, encoding);
    let html = match parse_html(&result.text, (!certain).then_some(encoding))? {
        ControlFlow::Continue(html) => html,
        ControlFlow::Break(declared) => {
            if declared != encoding {
                result = decoded(bytes, declared);
            }
            DocumentTree::parse(&result.text)?
        }
    };
    Ok((result, html))
}

/// Decode a plain text resource by its BOM, else its transport charset, else as UTF-8.
pub(crate) fn decode_text(bytes: &[u8], transport_charset: Option<&str>) -> DecodedInput {
    let encoding = Encoding::for_bom(bytes)
        .map(|(encoding, _)| encoding)
        .or_else(|| transport_charset.and_then(label))
        .unwrap_or(UTF_8);
    decoded(bytes, encoding)
}

/// Decode buffered HTML, honoring BOM/transport and actual parser encoding declarations.
#[cfg(test)]
pub(crate) fn decode_html(
    bytes: &[u8],
    transport_charset: Option<&str>,
) -> Result<DecodedHtml, crate::dom::DomError> {
    decode_document(bytes, transport_charset).map(|(decoded, _)| decoded)
}

fn css_declared_encoding(bytes: &[u8]) -> Option<&'static Encoding> {
    let scan = &bytes[..bytes.len().min(CSS_ENCODING_SCAN_BYTES)];
    let rest = scan.strip_prefix(CSS_ENCODING_PREFIX)?;
    let end = rest
        .iter()
        .position(|byte| *byte == b'"' || !byte.is_ascii())?;
    if rest.get(end..end + 2) != Some(b"\";") {
        return None;
    }
    let encoding = Encoding::for_label(&rest[..end])?;
    Some(if encoding == UTF_16BE || encoding == UTF_16LE {
        UTF_8
    } else {
        encoding
    })
}

/// Decode CSS using transport, exact @charset syntax and referring environment fallback.
pub(crate) fn decode_css(
    bytes: &[u8],
    transport_charset: Option<&str>,
    environment_encoding: Option<&str>,
) -> DecodedCss {
    let fallback = transport_charset
        .and_then(label)
        .or_else(|| css_declared_encoding(bytes))
        .or_else(|| environment_encoding.and_then(label))
        .unwrap_or(UTF_8);
    decoded(bytes, fallback)
}

pub(crate) fn css_environment<'a>(charset: Option<&str>, document: &'a str) -> &'a str {
    charset
        .and_then(label)
        .map_or(document, |encoding| encoding.name())
}
pub(crate) fn stylesheet(
    bytes: &[u8],
    transport: Option<&str>,
    environment: Option<&str>,
) -> crate::style::DecodedStylesheet {
    let decoded = decode_css(bytes, transport, environment);
    crate::style::DecodedStylesheet {
        text: decoded.text,
        encoding: decoded.encoding,
        had_errors: decoded.had_errors,
    }
}
