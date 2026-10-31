//! The user-agent style sheets (HTML §15.3), parsed once per process and shared by every
//! render. A render binds again the only state their rules hold for it: the document base their
//! source locations name and their sheet's namespace bindings.
use crate::resources::ResourceBase;
use crate::style::{
    SourceLocation, StyleDiagnostic,
    declaration::{Declared, PropertyDeclaration, Specified},
    grammar::Property,
    page_parser::PageItem,
    page_rules::PageSelectors,
    parser::{ContentRule, Declaration, absolute_url_text},
    selector::{NamespaceMap, Namespaces, RuleSelector},
};
use std::{cell::RefCell, sync::Arc};

/// The rules of a user-agent sheet in cascade order, each with its order within the sheet.
pub(super) struct UserAgentSheet {
    pub(super) rules: Vec<(Vec<usize>, SharedRule)>,
    pub(super) namespaces: NamespaceMap,
}

pub(super) enum SharedRule {
    Style(Vec<RuleSelector>, Vec<SharedDeclaration>),
    Page {
        selectors: PageSelectors,
        items: Vec<PageItem<SharedSource>>,
    },
}
impl SharedRule {
    /// `rule` as the sheet holds it; `None` for a kind of rule the user-agent sheets lack.
    pub(super) fn of(rule: ContentRule) -> Option<Self> {
        Some(match rule {
            ContentRule::Style(selectors, declarations) => Self::Style(
                selectors,
                declarations.iter().map(SharedDeclaration::of).collect(),
            ),
            ContentRule::Page { selectors, items } => Self::Page {
                selectors,
                items: items
                    .iter()
                    .map(|item| item.map(SharedSource::of))
                    .collect(),
            },
            _ => return None,
        })
    }
    /// The rule for a render whose document base is `base`, binding `namespaces`.
    pub(super) fn bind(
        &self,
        base: &ResourceBase,
        namespaces: &Namespaces,
        diagnostics: &RefCell<Vec<StyleDiagnostic>>,
    ) -> ContentRule {
        match self {
            Self::Style(selectors, declarations) => ContentRule::ParsedStyle(
                selectors.clone(),
                declarations
                    .iter()
                    .flat_map(|declaration| declaration.bind(base, namespaces, diagnostics))
                    .collect(),
            ),
            Self::Page { selectors, items } => ContentRule::Page {
                selectors: selectors.clone(),
                items: items
                    .iter()
                    .map(|item| item.map(|source| source.bind(base, namespaces)))
                    .collect(),
            },
        }
    }
}

/// A declaration as written, apart from its base and namespace bindings.
pub(super) struct SharedSource {
    name: String,
    value: String,
    important: bool,
    line: u32,
    column: u32,
    namespaced: bool,
}
impl SharedSource {
    fn of(declaration: &Declaration) -> Self {
        Self {
            name: declaration.name.clone(),
            value: declaration.value.clone(),
            important: declaration.important,
            line: declaration.location.line,
            column: declaration.location.column,
            namespaced: declaration.namespaces.is_some(),
        }
    }
    fn location(&self, base: &ResourceBase) -> SourceLocation {
        SourceLocation {
            base: base.clone(),
            line: self.line,
            column: self.column,
        }
    }
    fn namespaces(&self, namespaces: &Namespaces) -> Option<Namespaces> {
        self.namespaced.then(|| namespaces.clone())
    }
    fn bind(&self, base: &ResourceBase, namespaces: &Namespaces) -> Declaration {
        Declaration {
            name: self.name.clone(),
            value: self.value.clone(),
            important: self.important,
            location: self.location(base),
            namespaces: self.namespaces(namespaces),
        }
    }
}

/// A style rule's declaration and the longhand and custom-property declarations it parses to,
/// unless it parses again each render: where it substitutes values at computed-value time,
/// whose parsed form holds per-render state, where its URLs resolve against the document base,
/// or where it is invalid, which each render reports.
pub(super) struct SharedDeclaration {
    source: SharedSource,
    parsed: Option<Vec<SharedDeclared>>,
}
impl SharedDeclaration {
    fn of(declaration: &Declaration) -> Self {
        let source = SharedSource::of(declaration);
        let mut output = Vec::new();
        let parsed = absolute_url_text(
            &declaration.value,
            &ResourceBase::default(),
            &declaration.name,
        )
        .is_ok_and(|resolved| resolved.is_none())
            && crate::style::declaration::parse(declaration, &mut output).is_ok()
            && !output.is_empty();
        Self {
            parsed: parsed
                .then(|| {
                    output
                        .into_iter()
                        .map(|parsed| SharedDeclared::of(parsed.declared))
                        .collect()
                })
                .flatten(),
            source,
        }
    }
    fn bind(
        &self,
        base: &ResourceBase,
        namespaces: &Namespaces,
        diagnostics: &RefCell<Vec<StyleDiagnostic>>,
    ) -> Vec<PropertyDeclaration> {
        let Some(parsed) = &self.parsed else {
            return crate::style::stylesheet::parsed(
                vec![self.source.bind(base, namespaces)],
                base,
                diagnostics,
                crate::fonts::FontNamespace::ROOT,
            );
        };
        let location = Some(Arc::new(self.source.location(base)));
        parsed
            .iter()
            .map(|declared| PropertyDeclaration {
                declared: declared.bind(),
                location: location.clone(),
                important: self.source.important,
                origin: crate::style::cascade::Origin::Author,
                layer: crate::style::layers::LayerId::default(),
                font_namespace: crate::fonts::FontNamespace::ROOT,
                namespaces: self.source.namespaces(namespaces),
            })
            .collect()
    }
}

enum SharedDeclared {
    Longhand(Property, Specified),
    Custom(Arc<str>, Arc<str>, bool),
}
impl SharedDeclared {
    /// `None` for a declaration pending substitution.
    fn of(declared: Declared) -> Option<Self> {
        match declared {
            Declared::Longhand(property, specified) => Some(Self::Longhand(property, specified)),
            Declared::Custom(name, value, contextual) => {
                Some(Self::Custom(name, value, contextual))
            }
            Declared::Pending(..) => None,
        }
    }
    fn bind(&self) -> Declared {
        match self {
            Self::Longhand(property, specified) => Declared::Longhand(*property, specified.clone()),
            Self::Custom(name, value, contextual) => {
                Declared::Custom(name.clone(), value.clone(), *contextual)
            }
        }
    }
}
