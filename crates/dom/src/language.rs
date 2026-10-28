use ferri_pdf_spec_data::languages as data;

const SCRIPT_LENGTH: usize = 4;
const REGION_ALPHA_LENGTH: usize = 2;
const REGION_NUMERIC_LENGTH: usize = 3;
const SINGLETON_LENGTH: usize = 1;
const MAX_SUBTAG_LENGTH: usize = 8;
const MIN_VARIANT_LENGTH: usize = 5;

fn replacement<'a>(values: &'a [(&str, &str)], key: &str) -> Option<&'a str> {
    values
        .binary_search_by_key(&key, |(key, _)| key)
        .ok()
        .map(|index| values[index].1)
}

fn alpha(value: &str) -> bool {
    value.bytes().all(|ch| ch.is_ascii_alphabetic())
}
fn numeric(value: &str) -> bool {
    value.bytes().all(|ch| ch.is_ascii_digit())
}

pub(crate) fn extlang_form(value: &str) -> String {
    let lower = value.to_ascii_lowercase();
    let value = replacement(data::TAG, &lower).unwrap_or(&lower);
    let mut parts: Vec<String> = value.split('-').map(str::to_owned).collect();
    if parts[0] == "x" {
        return value.to_owned();
    }
    if let Some(alias) = replacement(data::LANGUAGE, &parts[0]) {
        parts[0] = alias.to_owned();
    }
    if parts.len() > 1 && replacement(data::EXTLANG, &parts[1]) == Some(parts[0].as_str()) {
        parts.remove(0);
        if let Some(alias) = replacement(data::LANGUAGE, &parts[0]) {
            parts[0] = alias.to_owned();
        }
    }
    let mut index = 1;
    if parts
        .get(index)
        .is_some_and(|part| part.len() == SCRIPT_LENGTH && alpha(part))
    {
        if let Some(alias) = replacement(data::SCRIPT, &parts[index]) {
            parts[index] = alias.to_owned();
        }
        index += 1;
    }
    if parts.get(index).is_some_and(|part| {
        (part.len() == REGION_ALPHA_LENGTH && alpha(part))
            || (part.len() == REGION_NUMERIC_LENGTH && numeric(part))
    }) {
        if let Some(alias) = replacement(data::REGION, &parts[index]) {
            parts[index] = alias.to_owned();
        }
        index += 1;
    }
    while parts.get(index).is_some_and(|part| {
        part.len() >= MIN_VARIANT_LENGTH
            || (part.len() == SCRIPT_LENGTH && part.as_bytes()[0].is_ascii_digit())
    }) {
        if let Some(alias) = replacement(data::VARIANT, &parts[index]) {
            parts[index] = alias.to_owned();
        }
        index += 1;
    }
    let extension_start = index;
    let mut extensions = Vec::new();
    while parts
        .get(index)
        .is_some_and(|part| part.len() == SINGLETON_LENGTH && part != "x")
    {
        let start = index;
        index += 1;
        while parts
            .get(index)
            .is_some_and(|part| part.len() > SINGLETON_LENGTH)
        {
            index += 1;
        }
        extensions.push(parts[start..index].to_vec());
    }
    if !extensions.is_empty() {
        extensions.sort_by(|a, b| a[0].cmp(&b[0]));
        let tail = parts[index..].to_vec();
        parts.truncate(extension_start);
        parts.extend(extensions.into_iter().flatten());
        parts.extend(tail);
    }
    if let Some(prefix) = replacement(data::EXTLANG, &parts[0]) {
        parts.insert(0, prefix.to_owned());
    }
    parts.join("-")
}

pub(crate) fn matches(language: &str, range: &str) -> bool {
    if range.is_empty() {
        return language.is_empty();
    }
    if language.is_empty() {
        return false;
    }
    let valid = range.split('-').enumerate().all(|(index, part)| {
        part == "*"
            || (!part.is_empty()
                && part.len() <= MAX_SUBTAG_LENGTH
                && part.bytes().all(|ch| {
                    if index == 0 {
                        ch.is_ascii_alphabetic()
                    } else {
                        ch.is_ascii_alphanumeric()
                    }
                }))
    });
    if !valid {
        return false;
    }
    let language = extlang_form(language);
    let range = extlang_form(range);
    let mut tags = language.split('-');
    let mut ranges = range.split('-');
    let first = ranges.next().expect("nonempty language range");
    if !tags.next().is_some_and(|tag| first == "*" || first == tag) {
        return false;
    }
    for part in ranges {
        if part == "*" {
            continue;
        }
        loop {
            match tags.next() {
                Some(tag) if tag == part => break,
                Some(tag) if tag.len() != SINGLETON_LENGTH => {}
                _ => return false,
            }
        }
    }
    true
}
