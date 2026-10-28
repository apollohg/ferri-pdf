use crate::ResourceMetadata;

const CHARSET_PARAMETER: &[u8] = b"charset";

fn is_http_whitespace(byte: u8) -> bool {
    matches!(byte, b'\t' | b'\n' | b'\r' | b' ')
}

fn trim_end(mut bytes: &[u8]) -> &[u8] {
    while bytes.last().is_some_and(|byte| is_http_whitespace(*byte)) {
        bytes = &bytes[..bytes.len() - 1];
    }
    bytes
}

fn trim_start(mut bytes: &[u8]) -> &[u8] {
    while bytes.first().is_some_and(|byte| is_http_whitespace(*byte)) {
        bytes = &bytes[1..];
    }
    bytes
}

fn is_http_token(byte: u8) -> bool {
    byte.is_ascii_alphanumeric()
        || matches!(
            byte,
            b'!' | b'#'
                | b'$'
                | b'%'
                | b'&'
                | b'\''
                | b'*'
                | b'+'
                | b'-'
                | b'.'
                | b'^'
                | b'_'
                | b'`'
                | b'|'
                | b'~'
        )
}

// MIME Sniffing's byte parser uses isomorphic decoding, including quoted obs-text.
pub(crate) fn parse_content_type(bytes: &[u8]) -> ResourceMetadata {
    let input = trim_start(trim_end(bytes));
    let Some(slash) = input.iter().position(|byte| *byte == b'/') else {
        return ResourceMetadata::default();
    };
    let type_name = &input[..slash];
    let rest = &input[slash + 1..];
    let subtype_end = rest
        .iter()
        .position(|byte| *byte == b';')
        .unwrap_or(rest.len());
    let subtype = trim_end(&rest[..subtype_end]);
    if type_name.is_empty()
        || subtype.is_empty()
        || !type_name.iter().chain(subtype).copied().all(is_http_token)
    {
        return ResourceMetadata::default();
    }
    let essence: String = type_name
        .iter()
        .chain(b"/")
        .chain(subtype)
        .map(|byte| char::from(byte.to_ascii_lowercase()))
        .collect();
    let mut metadata = ResourceMetadata {
        media_type: Some(essence),
        charset: None,
    };
    let mut rest = &rest[subtype_end..];
    while !rest.is_empty() {
        rest = trim_start(&rest[1..]);
        let name_end = rest
            .iter()
            .position(|byte| matches!(byte, b';' | b'='))
            .unwrap_or(rest.len());
        let name = &rest[..name_end];
        rest = &rest[name_end..];
        match rest.first() {
            None => break,
            Some(b';') => continue,
            _ => rest = &rest[1..],
        }
        if rest.is_empty() {
            break;
        }
        let value;
        if rest[0] == b'"' {
            let mut quoted = Vec::new();
            rest = &rest[1..];
            while let Some((&byte, next)) = rest.split_first() {
                rest = next;
                match byte {
                    b'"' => break,
                    b'\\' => {
                        if let Some((&escaped, next)) = rest.split_first() {
                            quoted.push(escaped);
                            rest = next;
                        } else {
                            quoted.push(byte);
                        }
                    }
                    _ => quoted.push(byte),
                }
            }
            let end = rest
                .iter()
                .position(|byte| *byte == b';')
                .unwrap_or(rest.len());
            rest = &rest[end..];
            value = quoted;
        } else {
            let end = rest
                .iter()
                .position(|byte| *byte == b';')
                .unwrap_or(rest.len());
            value = trim_end(&rest[..end]).to_vec();
            rest = &rest[end..];
            if value.is_empty() {
                continue;
            }
        }
        if metadata.charset.is_none()
            && name.eq_ignore_ascii_case(CHARSET_PARAMETER)
            && value
                .iter()
                .all(|byte| matches!(byte, b'\t' | b' '..=b'~' | 0x80..=0xff))
        {
            metadata.charset = Some(value.into_iter().map(char::from).collect());
        }
    }
    metadata
}
