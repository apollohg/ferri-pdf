use crate::resources::{ResourceError, ResourceMetadata, mime::parse_content_type};
use base64::{
    Engine, alphabet,
    engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig},
};
use std::path::PathBuf;

pub(super) const SCHEME: &str = "data:";
const RESOURCE_LABEL: &str = "[data URL]";
const DEFAULT_MEDIA_TYPE: &str = "text/plain";
const DEFAULT_CHARSET: &str = "US-ASCII";
const BASE64_PARAMETER: &str = "base64";
const BASE64_INPUT_BYTES: usize = 3;
const BASE64_OUTPUT_CHARS: usize = 4;
const HEX_RADIX: u32 = 16;
const BYTE_HIGH_NIBBLE: u8 = 16;
const DECODER: GeneralPurpose = GeneralPurpose::new(
    &alphabet::STANDARD,
    GeneralPurposeConfig::new()
        .with_decode_padding_mode(DecodePaddingMode::RequireNone)
        .with_decode_allow_trailing_bits(true),
);

#[derive(Debug, thiserror::Error)]
pub enum DataUrlError {
    #[error("missing metadata/payload separator")]
    MissingSeparator,
    #[error("invalid base64 payload: {0}")]
    Base64(#[from] base64::DecodeError),
}

fn too_large(limit: usize) -> ResourceError {
    ResourceError::TooLarge {
        path: PathBuf::from(RESOURCE_LABEL),
        limit,
    }
}

fn parts(data: &str) -> Result<(&str, &str, bool), DataUrlError> {
    let (metadata, payload) = data.split_once(',').ok_or(DataUrlError::MissingSeparator)?;
    if metadata.contains('#') {
        return Err(DataUrlError::MissingSeparator);
    }
    let metadata =
        metadata.trim_matches(|ch: char| matches!(ch, '\t' | '\n' | '\x0c' | '\r' | ' '));
    let base64 = metadata.rsplit(';').next().is_some_and(|part| {
        part.trim_start_matches(' ')
            .eq_ignore_ascii_case(BASE64_PARAMETER)
    }) && metadata.contains(';');
    let metadata = if base64 {
        metadata
            .rsplit_once(';')
            .expect("base64 parameter delimiter")
            .0
            .trim_end_matches(' ')
    } else {
        metadata
    };
    Ok((metadata, payload, base64))
}

pub(super) fn metadata(data: &str) -> Result<ResourceMetadata, DataUrlError> {
    let (metadata, _, _) = parts(data)?;
    let mime_type = if metadata.starts_with(';') {
        format!("{DEFAULT_MEDIA_TYPE}{metadata}")
    } else {
        metadata.to_owned()
    };
    let parsed = parse_content_type(mime_type.as_bytes());
    Ok(if parsed.media_type.is_none() {
        ResourceMetadata {
            media_type: Some(DEFAULT_MEDIA_TYPE.into()),
            charset: Some(DEFAULT_CHARSET.into()),
        }
    } else {
        parsed
    })
}

pub(super) fn decode(data: &str, limit: usize) -> Result<Vec<u8>, ResourceError> {
    let (_, payload, base64) = parts(data)?;
    let payload = payload.split('#').next().unwrap_or(payload);
    let encoded_limit = if base64 {
        limit
            .div_ceil(BASE64_INPUT_BYTES)
            .saturating_mul(BASE64_OUTPUT_CHARS)
    } else {
        limit
    };
    let mut bytes = Vec::with_capacity(payload.len().min(encoded_limit));
    let mut source = payload.as_bytes();
    while let Some((&byte, rest)) = source.split_first() {
        source = rest;
        let byte = if byte == b'%' && source.len() >= 2 {
            match (
                (source[0] as char).to_digit(HEX_RADIX),
                (source[1] as char).to_digit(HEX_RADIX),
            ) {
                (Some(high), Some(low)) => {
                    source = &source[2..];
                    high as u8 * BYTE_HIGH_NIBBLE + low as u8
                }
                _ => byte,
            }
        } else {
            byte
        };
        if base64 && matches!(byte, b'\t' | b'\n' | b'\x0c' | b'\r' | b' ') {
            continue;
        }
        if bytes.len() == encoded_limit {
            return Err(too_large(limit));
        }
        bytes.push(byte);
    }
    let bytes = if base64 {
        let mut payload = bytes.as_slice();
        if payload.len().is_multiple_of(BASE64_OUTPUT_CHARS) {
            payload = payload
                .strip_suffix(b"==")
                .or_else(|| payload.strip_suffix(b"="))
                .unwrap_or(payload);
        }
        DECODER.decode(payload).map_err(DataUrlError::Base64)?
    } else {
        bytes
    };
    if bytes.len() > limit {
        return Err(too_large(limit));
    }
    Ok(bytes)
}
