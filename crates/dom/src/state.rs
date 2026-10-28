use crate::controls::InputType;
use crate::{Element, HTML_NAMESPACE, SVG_NAMESPACE, TreeRelation, XLINK_NAMESPACE, XML_NAMESPACE};
use icu_properties::{CodePointMapData, props::BidiClass};
use percent_encoding::percent_decode_str;
use std::{collections::HashMap, sync::Arc};
use url::Url;

const RESERVED_CUSTOM_NAMES: &[&str] = &[
    "annotation-xml",
    "color-profile",
    "font-face",
    "font-face-src",
    "font-face-uri",
    "font-face-format",
    "font-face-name",
    "missing-glyph",
];

pub(crate) fn is_html_whitespace(ch: char) -> bool {
    matches!(ch, '\t' | '\n' | '\u{000c}' | '\r' | ' ')
}

pub(crate) fn is_custom_element_name(name: &str) -> bool {
    name.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
        && name.contains('-')
        && !RESERVED_CUSTOM_NAMES.contains(&name)
        && name.chars().all(|ch| {
            matches!(ch,
                '-' | '.' | '_' | '0'..='9' | 'a'..='z' | '\u{b7}'
                | '\u{c0}'..='\u{d6}' | '\u{d8}'..='\u{f6}' | '\u{f8}'..='\u{37d}'
                | '\u{37f}'..='\u{1fff}' | '\u{200c}'..='\u{200d}' | '\u{203f}'..='\u{2040}'
                | '\u{2070}'..='\u{218f}' | '\u{2c00}'..='\u{2fef}' | '\u{3001}'..='\u{d7ff}'
                | '\u{f900}'..='\u{fdcf}' | '\u{fdf0}'..='\u{fffd}' | '\u{10000}'..='\u{effff}'
            )
        })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Directionality {
    Ltr,
    Rtl,
}

#[derive(Debug)]
pub struct InitialState {
    pub language: Arc<str>,
    pub direction: Directionality,
    pub input_type: Option<InputType>,
    /// None for elements to which HTML enabled/disabled states do not apply.
    pub disabled: Option<bool>,
    pub(crate) control_ancestry: crate::controls::ControlAncestry,
}

pub(crate) fn prepare_initial_states<'a>(
    elements: impl Iterator<Item = Element<'a>>,
    states: &mut HashMap<ego_tree::NodeId, InitialState>,
    default_language: &str,
) {
    let default_language: Arc<str> = default_language.into();
    let mut pending = Vec::new();
    for element in elements {
        let mut current = Some(element);
        while let Some(element) = current {
            if states.contains_key(&element.raw.id()) {
                break;
            }
            pending.push(element);
            current = element.shadow_including_parent();
        }
        while let Some(element) = pending.pop() {
            let parent = element.shadow_including_parent().map(|parent| {
                states
                    .get(&parent.raw.id())
                    .expect("parent state precedes its child")
            });
            let language = element.declared_language().map_or_else(
                || Arc::clone(parent.map_or(&default_language, |state| &state.language)),
                Arc::from,
            );
            let input_type = (element.namespace() == HTML_NAMESPACE && element.name() == "input")
                .then(|| InputType::of(element));
            let direction = element.initial_directionality(
                parent.map_or(Directionality::Ltr, |state| state.direction),
                input_type,
                &|host| states[&host.raw.id()].direction,
            );
            let control_parent = element.parent(TreeRelation::Selector).map(|parent| {
                states
                    .get(&parent.raw.id())
                    .expect("DOM parent state precedes its child")
            });
            let (disabled, control_ancestry) =
                crate::controls::initial_disabled(element, control_parent);
            states.insert(
                element.raw.id(),
                InitialState {
                    language,
                    direction,
                    input_type,
                    disabled,
                    control_ancestry,
                },
            );
        }
    }
}

fn strong_direction(text: &str) -> Option<Directionality> {
    let data = CodePointMapData::<BidiClass>::new();
    text.chars().find_map(|ch| match data.get(ch) {
        BidiClass::LeftToRight => Some(Directionality::Ltr),
        BidiClass::RightToLeft | BidiClass::ArabicLetter => Some(Directionality::Rtl),
        _ => None,
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DirectionAttribute {
    Undefined,
    Ltr,
    Rtl,
    Auto,
}

impl<'a> Element<'a> {
    pub fn initial_state(self) -> &'a InitialState {
        &self.tree.initial_states[&self.raw.id()]
    }
    fn direction_attribute(self) -> DirectionAttribute {
        if self.namespace() == HTML_NAMESPACE
            && let Some(value) = self.attr("dir")
        {
            for (keyword, state) in [
                ("ltr", DirectionAttribute::Ltr),
                ("rtl", DirectionAttribute::Rtl),
                ("auto", DirectionAttribute::Auto),
            ] {
                if value.eq_ignore_ascii_case(keyword) {
                    return state;
                }
            }
        }
        DirectionAttribute::Undefined
    }
    fn excludes_auto_direction(self) -> bool {
        self.namespace() == HTML_NAMESPACE
            && (matches!(self.name(), "bdi" | "script" | "style" | "textarea")
                || self.direction_attribute() != DirectionAttribute::Undefined)
    }
    pub fn language(self) -> &'a str {
        &self.initial_state().language
    }
    fn declared_language(self) -> Option<&'a str> {
        if let Some((_, language)) =
            self.value().attrs.iter().find(|(name, _)| {
                name.ns.as_ref() == XML_NAMESPACE && name.local.as_ref() == "lang"
            })
        {
            return Some(language);
        }
        matches!(self.namespace(), HTML_NAMESPACE | SVG_NAMESPACE)
            .then(|| self.attr("lang"))
            .flatten()
    }

    pub fn directionality(self) -> Directionality {
        self.initial_state().direction
    }
    fn initial_directionality(
        self,
        parent: Directionality,
        input_type: Option<InputType>,
        host_direction: &impl Fn(Self) -> Directionality,
    ) -> Directionality {
        if self.namespace() != HTML_NAMESPACE {
            return parent;
        }
        match self.direction_attribute() {
            DirectionAttribute::Ltr => Directionality::Ltr,
            DirectionAttribute::Rtl => Directionality::Rtl,
            value if value == DirectionAttribute::Auto || self.name() == "bdi" => self
                .auto_directionality(input_type, host_direction)
                .unwrap_or(Directionality::Ltr),
            _ if input_type == Some(InputType::Tel) => Directionality::Ltr,
            _ => parent,
        }
    }

    fn auto_directionality(
        self,
        input_type: Option<InputType>,
        host_direction: &impl Fn(Self) -> Directionality,
    ) -> Option<Directionality> {
        let value = if self.name() == "textarea" {
            Some(self.text().collect::<String>())
        } else {
            input_type.and_then(|kind| kind.auto_direction_value(self))
        };
        if let Some(value) = value {
            return (!value.is_empty())
                .then(|| strong_direction(&value).unwrap_or(Directionality::Ltr));
        }
        if self.namespace() == HTML_NAMESPACE
            && self.name() == "slot"
            && self.shadow_host().is_some()
            && self
                .tree
                .shadow
                .slots
                .get(&self.raw.id())
                .is_some_and(|nodes| !nodes.is_empty())
        {
            return self
                .child_nodes(TreeRelation::Inheritance)
                .find_map(|node| {
                    if let Some(text) = node.value().as_text() {
                        strong_direction(text)
                    } else {
                        node.element().and_then(|element| {
                            element.contained_auto_directionality(true, host_direction)
                        })
                    }
                });
        }
        self.contained_auto_directionality(false, host_direction)
    }
    fn contained_auto_directionality(
        self,
        exclude_root: bool,
        host_direction: &impl Fn(Self) -> Directionality,
    ) -> Option<Directionality> {
        if exclude_root && self.excludes_auto_direction() {
            return None;
        }
        let mut pending: Vec<_> = self.child_nodes(TreeRelation::Selector).rev().collect();
        while let Some(node) = pending.pop() {
            if let Some(text) = node.value().as_text()
                && let Some(direction) = strong_direction(text)
            {
                return Some(direction);
            }
            if let Some(element) = node.element() {
                if element.excludes_auto_direction() {
                    continue;
                }
                if element.namespace() == HTML_NAMESPACE
                    && element.name() == "slot"
                    && let Some(host) = element.shadow_host()
                {
                    return Some(host_direction(host));
                }
                pending.extend(element.child_nodes(TreeRelation::Selector).rev());
            }
        }
        None
    }

    pub fn is_defined(self) -> bool {
        if self.namespace() != HTML_NAMESPACE {
            return true;
        }
        let name = self.name();
        let autonomous = is_custom_element_name(name);
        !autonomous && self.attr("is").is_none()
    }

    pub fn hyperlink(self) -> Option<&'a str> {
        match (self.namespace(), self.name()) {
            (HTML_NAMESPACE, "a" | "area") => self.attr("href"),
            (SVG_NAMESPACE, "a") => self.attr("href").or_else(|| {
                self.value()
                    .attrs
                    .iter()
                    .find(|(name, _)| {
                        name.ns.as_ref() == XLINK_NAMESPACE && name.local.as_ref() == "href"
                    })
                    .map(|(_, value)| value.as_ref())
            }),
            _ => None,
        }
    }

    pub fn is_target(self) -> bool {
        let Some(fragment) = self.tree.document_url.as_ref().and_then(Url::fragment) else {
            return false;
        };
        if fragment.is_empty() {
            return false;
        }
        let indicated = |fragment: &str| {
            self.tree
                .elements()
                .find(|element| element.attr("id") == Some(fragment))
                .or_else(|| {
                    self.tree.elements().find(|element| {
                        element.namespace() == HTML_NAMESPACE
                            && element.name() == "a"
                            && element.attr("name") == Some(fragment)
                    })
                })
        };
        indicated(fragment)
            .or_else(|| indicated(&percent_decode_str(fragment).decode_utf8_lossy()))
            .is_some_and(|target| target == self)
    }

    pub fn is_local_link(self, segments: Option<u32>) -> bool {
        let Some(document) = &self.tree.document_url else {
            return false;
        };
        let Some(link) = self.hyperlink().and_then(|href| {
            self.tree
                .base_url
                .as_ref()
                .and_then(|base| base.join(href).ok())
        }) else {
            return false;
        };
        let Some(count) = segments else {
            let mut document = document.clone();
            let mut link = link;
            document.set_fragment(None);
            link.set_fragment(None);
            return document == link;
        };
        let (Some(mut document_path), Some(mut link_path)) =
            (document.path_segments(), link.path_segments())
        else {
            return false;
        };
        if document.origin() != link.origin() {
            return false;
        }
        for _ in 0..count {
            match (document_path.next(), link_path.next()) {
                (Some(a), Some(b)) if a == b => {}
                _ => return false,
            }
        }
        true
    }
}
