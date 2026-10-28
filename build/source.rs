//! Loads machine-readable CSS data into a source-neutral model. Only this module knows the
//! mdn-data file layout, so another source with the same Value Definition Syntax can replace it.
use crate::BuildError;
use serde_json::{Map, Value};
use std::{collections::BTreeMap, fs, path::Path};

const MDN_DIR: &str = "mdn-data";
const PROPERTIES_FILE: &str = "properties.json";
const SYNTAXES_FILE: &str = "syntaxes.json";
const AT_RULES_FILE: &str = "at-rules.json";
const UNITS_FILE: &str = "units.json";
const ERRATA_FILE: &str = "errata.json";
const EXTENSIONS_FILE: &str = "extensions.json";
/// Vendor and proprietary entries are not part of any CSS specification.
const EXCLUDED_STATUS: &str = "nonstandard";
const CUSTOM_PROPERTY_ENTRY: &str = "--*";
/// At-rules whose descriptors the engine parses.
pub(crate) const AT_RULES: [&str; 4] = [
    "@page",
    "@font-face",
    "@font-palette-values",
    "@counter-style",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Origin {
    Specification,
    Errata,
    Extension,
}

pub(crate) enum Initial {
    Value(String),
    /// A description in place of a value, such as `dependsOnUserAgent`.
    Prose(String),
    Longhands(Vec<String>),
}

pub(crate) struct Property {
    pub(crate) syntax: String,
    pub(crate) initial: Initial,
    pub(crate) inherited: bool,
    pub(crate) groups: Vec<String>,
    pub(crate) origin: Origin,
}

pub(crate) struct Descriptor {
    pub(crate) at_rule: String,
    pub(crate) name: String,
    pub(crate) syntax: String,
    pub(crate) initial: String,
}

pub(crate) struct Spec {
    pub(crate) properties: BTreeMap<String, Property>,
    pub(crate) syntaxes: BTreeMap<String, String>,
    pub(crate) descriptors: Vec<Descriptor>,
    pub(crate) units: Vec<String>,
    /// Every file read, for build-script change tracking.
    pub(crate) files: Vec<String>,
}

impl Spec {
    pub(crate) fn load(directory: &Path) -> Result<Self, BuildError> {
        let mut spec = Self {
            properties: BTreeMap::new(),
            syntaxes: BTreeMap::new(),
            descriptors: Vec::new(),
            units: Vec::new(),
            files: Vec::new(),
        };
        let mdn = directory.join(MDN_DIR);
        for (name, entry) in object(&spec.read(&mdn.join(PROPERTIES_FILE))?, PROPERTIES_FILE)? {
            if name == CUSTOM_PROPERTY_ENTRY || text(entry, name, "status")? == EXCLUDED_STATUS {
                continue;
            }
            spec.properties
                .insert(name.clone(), property(entry, name, Origin::Specification)?);
        }
        for (name, entry) in object(&spec.read(&mdn.join(SYNTAXES_FILE))?, SYNTAXES_FILE)? {
            spec.syntaxes
                .insert(name.clone(), text(entry, name, "syntax")?.to_owned());
        }
        let at_rules = spec.read(&mdn.join(AT_RULES_FILE))?;
        for at_rule in AT_RULES {
            let context = format!("{AT_RULES_FILE} {at_rule}");
            let rule = at_rules.get(at_rule).ok_or_else(|| BuildError::Field {
                context: context.clone(),
                field: "at-rule",
            })?;
            let descriptors = rule.get("descriptors").ok_or_else(|| BuildError::Field {
                context: context.clone(),
                field: "descriptors",
            })?;
            for (name, entry) in object(descriptors, &context)? {
                let context = format!("{at_rule} {name}");
                spec.descriptors.push(Descriptor {
                    at_rule: at_rule.to_owned(),
                    name: name.clone(),
                    syntax: text(entry, &context, "syntax")?.to_owned(),
                    initial: text(entry, &context, "initial")?.to_owned(),
                });
            }
        }
        spec.units = object(&spec.read(&mdn.join(UNITS_FILE))?, UNITS_FILE)?
            .keys()
            .cloned()
            .collect();
        spec.apply_errata(&directory.join(ERRATA_FILE))?;
        spec.apply_extensions(&directory.join(EXTENSIONS_FILE))?;
        Ok(spec)
    }

    fn read(&mut self, path: &Path) -> Result<Value, BuildError> {
        self.files.push(path.display().to_string());
        let source = fs::read_to_string(path).map_err(|source| BuildError::Io {
            path: path.display().to_string(),
            source,
        })?;
        serde_json::from_str(&source).map_err(|source| BuildError::Json {
            path: path.display().to_string(),
            source,
        })
    }

    /// Corrections from the specifications where the data is missing or outdated; each entry
    /// replaces only the fields it names, and additions define omitted properties in full.
    fn apply_errata(&mut self, path: &Path) -> Result<(), BuildError> {
        let errata = self.read(path)?;
        let file = path.display().to_string();
        for (name, entry) in section(&errata, &file, "syntaxes")? {
            citation(entry, name)?;
            self.syntaxes
                .insert(name.clone(), text(entry, name, "syntax")?.to_owned());
        }
        for (name, entry) in section(&errata, &file, "properties")? {
            citation(entry, name)?;
            let property = self
                .properties
                .get_mut(name)
                .ok_or_else(|| BuildError::UnknownErratum { name: name.clone() })?;
            property.origin = Origin::Errata;
            if let Some(syntax) = entry.get("syntax") {
                string(syntax, name, "syntax")?.clone_into(&mut property.syntax);
            }
            if let Some(value) = entry.get("initial") {
                property.initial = initial(value, name)?;
            }
            if let Some(value) = entry.get("inherited") {
                property.inherited = value.as_bool().ok_or_else(|| BuildError::Field {
                    context: name.to_owned(),
                    field: "inherited",
                })?;
            }
        }
        for (name, entry) in section(&errata, &file, "additions")? {
            citation(entry, name)?;
            if self.properties.contains_key(name) {
                return Err(BuildError::DuplicateAddition { name: name.clone() });
            }
            self.properties
                .insert(name.clone(), property(entry, name, Origin::Errata)?);
        }
        if let Some(rules) = errata.get("descriptors") {
            for (at_rule, descriptors) in object(rules, &file)? {
                if !AT_RULES.contains(&at_rule.as_str()) {
                    return Err(BuildError::UnknownErratum {
                        name: at_rule.clone(),
                    });
                }
                for (name, entry) in object(descriptors, at_rule)? {
                    let context = format!("{at_rule} {name}");
                    citation(entry, &context)?;
                    if let Some(descriptor) = self.descriptors.iter_mut().find(|descriptor| {
                        descriptor.at_rule == *at_rule && descriptor.name == *name
                    }) {
                        if let Some(syntax) = entry.get("syntax") {
                            string(syntax, &context, "syntax")?.clone_into(&mut descriptor.syntax);
                        }
                        if let Some(initial) = entry.get("initial") {
                            string(initial, &context, "initial")?
                                .clone_into(&mut descriptor.initial);
                        }
                    } else {
                        self.descriptors.push(Descriptor {
                            at_rule: at_rule.clone(),
                            name: name.clone(),
                            syntax: text(entry, &context, "syntax")?.to_owned(),
                            initial: text(entry, &context, "initial")?.to_owned(),
                        });
                    }
                }
            }
        }
        Ok(())
    }

    /// Non-standard properties used by real documents, kept apart from specification data.
    fn apply_extensions(&mut self, path: &Path) -> Result<(), BuildError> {
        let extensions = self.read(path)?;
        let file = path.display().to_string();
        for (name, entry) in section(&extensions, &file, "properties")? {
            citation(entry, name)?;
            if self.properties.contains_key(name) {
                return Err(BuildError::DuplicateExtension { name: name.clone() });
            }
            self.properties
                .insert(name.clone(), property(entry, name, Origin::Extension)?);
        }
        Ok(())
    }
}

fn property(entry: &Value, name: &str, origin: Origin) -> Result<Property, BuildError> {
    Ok(Property {
        syntax: text(entry, name, "syntax")?.to_owned(),
        initial: initial(
            entry.get("initial").ok_or_else(|| BuildError::Field {
                context: name.to_owned(),
                field: "initial",
            })?,
            name,
        )?,
        inherited: entry
            .get("inherited")
            .and_then(Value::as_bool)
            .ok_or_else(|| BuildError::Field {
                context: name.to_owned(),
                field: "inherited",
            })?,
        groups: entry
            .get("groups")
            .and_then(Value::as_array)
            .ok_or_else(|| BuildError::Field {
                context: name.to_owned(),
                field: "groups",
            })?
            .iter()
            .map(|group| string(group, name, "groups").map(str::to_owned))
            .collect::<Result<_, _>>()?,
        origin,
    })
}

fn initial(value: &Value, name: &str) -> Result<Initial, BuildError> {
    match value {
        Value::Array(longhands) => Ok(Initial::Longhands(
            longhands
                .iter()
                .map(|longhand| string(longhand, name, "initial").map(str::to_owned))
                .collect::<Result<_, _>>()?,
        )),
        value => Ok(Initial::Value(
            string(value, name, "initial")?.trim().to_owned(),
        )),
    }
}

fn citation(entry: &Value, name: &str) -> Result<(), BuildError> {
    text(entry, name, "source").map(|_| ())
}

fn section<'a>(
    document: &'a Value,
    file: &str,
    name: &'static str,
) -> Result<&'a Map<String, Value>, BuildError> {
    object(
        document.get(name).ok_or_else(|| BuildError::Field {
            context: file.to_owned(),
            field: name,
        })?,
        file,
    )
}

fn object<'a>(value: &'a Value, context: &str) -> Result<&'a Map<String, Value>, BuildError> {
    value.as_object().ok_or_else(|| BuildError::Field {
        context: context.to_owned(),
        field: "object",
    })
}

fn text<'a>(entry: &'a Value, context: &str, field: &'static str) -> Result<&'a str, BuildError> {
    string(
        entry.get(field).ok_or_else(|| BuildError::Field {
            context: context.to_owned(),
            field,
        })?,
        context,
        field,
    )
}

fn string<'a>(value: &'a Value, context: &str, field: &'static str) -> Result<&'a str, BuildError> {
    value.as_str().ok_or_else(|| BuildError::Field {
        context: context.to_owned(),
        field,
    })
}
