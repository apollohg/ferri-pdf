//! HTML §4.8.4.3.7–§4.8.4.3.12: the image source an `img` selects, by its `picture`
//! siblings' media and types, `srcset` densities and `sizes`, for the print medium.
use crate::dom::{Element, HTML_NAMESPACE, TreeRelation};
use crate::resources::ResourceMetadata;
use crate::style::{POINTS_PER_PIXEL, media, print::PrintEnvironment};

const AUTO: &str = "auto";
const WIDTH_DESCRIPTOR: char = 'w';
const DENSITY_DESCRIPTOR: char = 'x';
const HEIGHT_DESCRIPTOR: char = 'h';
const DEFAULT_DENSITY: f32 = 1.0;
/// `100vw`, the source size when no `sizes` entry applies.
const FULL_VIEWPORT_WIDTH: f32 = 1.0;

/// The URL an `img` fetches and the pixel density that scales its natural dimensions.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ImageSource {
    pub url: String,
    pub density: f32,
}

#[derive(Debug, PartialEq)]
struct Candidate {
    url: String,
    width: Option<u32>,
    density: Option<f32>,
}

struct SourceSet {
    candidates: Vec<Candidate>,
    /// The source size, in CSS pixels.
    size: f32,
}

/// HTML §4.8.4.3.9 "update the source set": the element supplying the source set, and the set.
fn update<'a>(img: Element<'a>, environment: &PrintEnvironment) -> (Element<'a>, SourceSet) {
    let picture = img
        .parent(TreeRelation::Selector)
        .filter(|parent| parent.namespace() == HTML_NAMESPACE && parent.name() == "picture");
    let elements: Vec<Element<'a>> = picture.map_or_else(
        || vec![img],
        |picture| picture.children(TreeRelation::Selector).collect(),
    );
    for child in elements {
        if child.id() == img.id() {
            return (
                img,
                create(
                    img.attr("src").unwrap_or_default(),
                    img.attr("srcset").unwrap_or_default(),
                    img.attr("sizes").unwrap_or_default(),
                    environment,
                ),
            );
        }
        if child.namespace() != HTML_NAMESPACE || child.name() != "source" {
            continue;
        }
        let Some(srcset) = child.attr("srcset") else {
            continue;
        };
        let candidates = parse_srcset(srcset);
        if candidates.is_empty() {
            continue;
        }
        if child.attr("media").is_some_and(|query| {
            !media::MediaQuery::parse(query)
                .is_ok_and(|query| media::matches_media(&query, environment))
        }) {
            continue;
        }
        let size = parse_sizes(child.attr("sizes").unwrap_or_default(), environment);
        if child.attr("type").is_some_and(|kind| {
            ResourceMetadata::from_content_type(kind)
                .media_type
                .is_none_or(|essence| !crate::images::SUPPORTED_TYPES.contains(&essence.as_str()))
        }) {
            continue;
        }
        let source = if child.attr("width").is_some() || child.attr("height").is_some() {
            child
        } else {
            img
        };
        return (source, normalize(SourceSet { candidates, size }));
    }
    unreachable!("an img is among its picture's children")
}

/// HTML §4.8.4.3.8 "create a source set".
fn create(default: &str, srcset: &str, sizes: &str, environment: &PrintEnvironment) -> SourceSet {
    let mut candidates = if srcset.is_empty() {
        Vec::new()
    } else {
        parse_srcset(srcset)
    };
    if !default.is_empty()
        && !candidates.iter().any(|candidate| {
            candidate.density == Some(DEFAULT_DENSITY) || candidate.width.is_some()
        })
    {
        candidates.push(Candidate {
            url: default.into(),
            width: None,
            density: None,
        });
    }
    normalize(SourceSet {
        candidates,
        size: parse_sizes(sizes, environment),
    })
}

/// HTML §4.8.4.3.12 "normalize the source densities".
fn normalize(mut set: SourceSet) -> SourceSet {
    for candidate in &mut set.candidates {
        if candidate.density.is_none() {
            candidate.density = Some(
                candidate
                    .width
                    .map_or(DEFAULT_DENSITY, |width| width as f32 / set.size),
            );
        }
    }
    set
}

/// The element whose `width` and `height` attributes `img` maps to its dimension properties.
pub(super) fn dimension_source<'a>(
    img: Element<'a>,
    environment: &PrintEnvironment,
) -> Element<'a> {
    update(img, environment).0
}

/// HTML §4.8.4.3.7 "select an image source". Print output has infinite resolution
/// (`resolution: infinite`), so of the distinct densities the highest serves it best.
pub(super) fn select(img: Element<'_>, environment: &PrintEnvironment) -> Option<ImageSource> {
    let (_, set) = update(img, environment);
    let mut distinct: Vec<Candidate> = Vec::new();
    for candidate in set.candidates {
        if !distinct
            .iter()
            .any(|earlier| earlier.density == candidate.density)
        {
            distinct.push(candidate);
        }
    }
    distinct
        .into_iter()
        .reduce(|best, candidate| {
            if candidate.density > best.density {
                candidate
            } else {
                best
            }
        })
        .map(|candidate| ImageSource {
            url: candidate.url,
            density: candidate.density.unwrap_or(DEFAULT_DENSITY),
        })
}

/// HTML §4.8.4.3.10 "parse a srcset attribute".
fn parse_srcset(input: &str) -> Vec<Candidate> {
    let characters: Vec<char> = input.chars().collect();
    let mut position = 0;
    let mut candidates = Vec::new();
    loop {
        while characters
            .get(position)
            .is_some_and(|character| character.is_ascii_whitespace() || *character == ',')
        {
            position += 1;
        }
        if position >= characters.len() {
            return candidates;
        }
        let start = position;
        while characters
            .get(position)
            .is_some_and(|character| !character.is_ascii_whitespace())
        {
            position += 1;
        }
        let mut url: String = characters[start..position].iter().collect();
        let mut descriptors = Vec::new();
        if url.ends_with(',') {
            url = url.trim_end_matches(',').into();
        } else {
            position = tokenize_descriptors(&characters, position, &mut descriptors);
        }
        if let Some(candidate) = parse_descriptors(url, &descriptors) {
            candidates.push(candidate);
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum DescriptorState {
    InDescriptor,
    InParens,
    AfterDescriptor,
}

/// The srcset descriptor tokenizer; returns the position after the candidate's descriptors.
fn tokenize_descriptors(
    characters: &[char],
    mut position: usize,
    descriptors: &mut Vec<String>,
) -> usize {
    while characters
        .get(position)
        .is_some_and(char::is_ascii_whitespace)
    {
        position += 1;
    }
    let mut current = String::new();
    let mut state = DescriptorState::InDescriptor;
    loop {
        let character = characters.get(position).copied();
        match (state, character) {
            (DescriptorState::InDescriptor, Some(character)) if character.is_ascii_whitespace() => {
                if !current.is_empty() {
                    descriptors.push(std::mem::take(&mut current));
                    state = DescriptorState::AfterDescriptor;
                }
            }
            (DescriptorState::InDescriptor, Some(',')) => {
                if !current.is_empty() {
                    descriptors.push(current);
                }
                return position + 1;
            }
            (DescriptorState::InDescriptor, Some('(')) => {
                current.push('(');
                state = DescriptorState::InParens;
            }
            (DescriptorState::InDescriptor, None) => {
                if !current.is_empty() {
                    descriptors.push(current);
                }
                return position;
            }
            (DescriptorState::InDescriptor | DescriptorState::InParens, Some(character)) => {
                current.push(character);
                if state == DescriptorState::InParens && character == ')' {
                    state = DescriptorState::InDescriptor;
                }
            }
            (DescriptorState::InParens, None) => {
                descriptors.push(current);
                return position;
            }
            (DescriptorState::AfterDescriptor, Some(character))
                if character.is_ascii_whitespace() => {}
            (DescriptorState::AfterDescriptor, None) => return position,
            (DescriptorState::AfterDescriptor, Some(_)) => {
                state = DescriptorState::InDescriptor;
                continue;
            }
        }
        position += 1;
    }
}

/// The srcset descriptor parser: a candidate, or `None` on a parse error.
fn parse_descriptors(url: String, descriptors: &[String]) -> Option<Candidate> {
    let mut width = None;
    let mut density = None;
    let mut height = None;
    for descriptor in descriptors {
        let (value, kind) =
            descriptor.split_at(descriptor.len() - descriptor.chars().last()?.len_utf8());
        match kind.chars().next()? {
            WIDTH_DESCRIPTOR if is_non_negative_integer(value) => {
                if width.is_some() || density.is_some() {
                    return None;
                }
                width = Some(value.parse::<u32>().ok().filter(|width| *width != 0)?);
            }
            DENSITY_DESCRIPTOR if is_floating_point_number(value) => {
                if width.is_some() || density.is_some() || height.is_some() {
                    return None;
                }
                density = Some(
                    value
                        .parse::<f32>()
                        .ok()
                        .filter(|density| *density >= 0.0)?,
                );
            }
            HEIGHT_DESCRIPTOR if is_non_negative_integer(value) => {
                if height.is_some() || density.is_some() {
                    return None;
                }
                height = Some(value.parse::<u32>().ok().filter(|height| *height != 0)?);
            }
            _ => return None,
        }
    }
    if height.is_some() && width.is_none() {
        return None;
    }
    Some(Candidate {
        url,
        width,
        density,
    })
}

fn is_non_negative_integer(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit())
}

/// HTML §2.3.4.3 valid floating-point number: an optional `-`, digits with an optional
/// fraction or a bare fraction, and an optional exponent.
fn is_floating_point_number(value: &str) -> bool {
    let digits = |bytes: &[u8]| {
        bytes
            .iter()
            .take_while(|byte| byte.is_ascii_digit())
            .count()
    };
    let mut bytes = value.as_bytes();
    if let Some(rest) = bytes.strip_prefix(b"-") {
        bytes = rest;
    }
    let integer = digits(bytes);
    bytes = &bytes[integer..];
    let mut fraction = 0;
    if let Some(rest) = bytes.strip_prefix(b".") {
        fraction = digits(rest);
        if fraction == 0 {
            return false;
        }
        bytes = &rest[fraction..];
    }
    if integer == 0 && fraction == 0 {
        return false;
    }
    if let Some(rest) = bytes
        .strip_prefix(b"e")
        .or_else(|| bytes.strip_prefix(b"E"))
    {
        let rest = rest
            .strip_prefix(b"-")
            .or_else(|| rest.strip_prefix(b"+"))
            .unwrap_or(rest);
        let exponent = digits(rest);
        return exponent != 0 && exponent == rest.len();
    }
    bytes.is_empty()
}

/// HTML §4.8.4.3.11 "parse a sizes attribute", in CSS pixels. An `auto` size is skipped:
/// the `img` is not being rendered while its source is selected and has no last
/// auto-sizes width.
fn parse_sizes(value: &str, environment: &PrintEnvironment) -> f32 {
    for entry in media::comma_separated(value) {
        let entry = entry.trim_end_matches(|character: char| character.is_ascii_whitespace());
        let Some((condition, size)) = media::split_last_component(entry) else {
            continue;
        };
        let size = if size.eq_ignore_ascii_case(AUTO) {
            None
        } else {
            let Some(points) = media::source_size_length(size, environment) else {
                continue;
            };
            Some(points / POINTS_PER_PIXEL)
        };
        let condition =
            condition.trim_end_matches(|character: char| character.is_ascii_whitespace());
        if !condition.is_empty() && !media::condition_matches(condition, environment) {
            continue;
        }
        if let Some(size) = size {
            return size;
        }
    }
    FULL_VIEWPORT_WIDTH * environment.paper[0] / POINTS_PER_PIXEL
}
