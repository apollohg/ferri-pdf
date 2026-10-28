use crate::{
    MarginLocation, SourceLocation, StyleDiagnostic, StyleError, Stylesheet,
    cascade::{self, Candidate, Cascaded, DeclarationContext, choose, sort},
    component::Component,
    declaration::{self, PropertyDeclaration, Specified},
    grammar::{Descriptor, Keyword},
    page_parser::PageItem,
    parser::Declaration,
    stylesheet::{PAGE_AT_RULE, implemented_page_descriptor, parsed, unimplemented_descriptor},
    variables,
    wide_keywords::WideKeyword,
};
use ferri_pdf_fonts::FontNamespace;
use std::{cell::RefCell, collections::HashMap, sync::Arc};

const SPECIFICITY_COMPONENT_BITS: u32 = 10;
const MAX_SPECIFICITY_COMPONENT: u32 = (1 << SPECIFICITY_COMPONENT_BITS) - 1;
const NAME_SPECIFICITY: u32 = 1 << (2 * SPECIFICITY_COMPONENT_BITS);

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq, serde::Serialize)]
pub enum PageSide {
    Left,
    #[default]
    Right,
}

/// Semantic page selection; first and blank refer to printed physical pages.
#[derive(Clone, Debug, Eq, Hash, PartialEq, serde::Serialize)]
pub struct PageRequest {
    pub name: Option<Arc<str>>,
    pub side: PageSide,
    pub first: bool,
    pub blank: bool,
}

pub struct ResolvedPage {
    pub request: PageRequest,
    pub geometry: crate::PageGeometry,
    pub style: crate::Style,
    pub production: crate::PageProduction,
    pub margins: Vec<crate::PageMargin>,
}
impl Default for PageRequest {
    fn default() -> Self {
        Self {
            name: None,
            side: PageSide::Right,
            first: true,
            blank: false,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub(super) struct PageSelectors(pub Vec<PageSelector>);
impl PageSelectors {
    fn matching_specificity(&self, request: &PageRequest) -> Option<u32> {
        if self.0.is_empty() {
            Some(0)
        } else {
            self.0
                .iter()
                .filter_map(|selector| selector.matching_specificity(request))
                .max()
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct PageSelector {
    name: Option<Arc<str>>,
    left: bool,
    right: bool,
    first: bool,
    blank: bool,
    first_blank: u32,
    left_right: u32,
}
impl PageSelector {
    pub(crate) fn new(name: Option<Arc<str>>) -> Self {
        Self {
            name,
            left: false,
            right: false,
            first: false,
            blank: false,
            first_blank: 0,
            left_right: 0,
        }
    }
    pub(crate) fn add_pseudo(&mut self, name: &str) -> bool {
        let component = cssparser::match_ignore_ascii_case! {name,
            "left" => {self.left=true; &mut self.left_right},
            "right" => {self.right=true; &mut self.left_right},
            "first" => {self.first=true; &mut self.first_blank},
            "blank" => {self.blank=true; &mut self.first_blank},
            _ => return false,
        };
        *component = component.saturating_add(1).min(MAX_SPECIFICITY_COMPONENT);
        true
    }
    pub(crate) fn is_empty(&self) -> bool {
        self.name.is_none() && !(self.left || self.right || self.first || self.blank)
    }
    fn matching_specificity(&self, request: &PageRequest) -> Option<u32> {
        if self.name.as_ref().is_some_and(|name| {
            name.eq_ignore_ascii_case(Keyword::Auto.name()) || request.name.as_ref() != Some(name)
        }) || (self.left && request.side != PageSide::Left)
            || (self.right && request.side != PageSide::Right)
            || (self.first && !request.first)
            || (self.blank && !request.blank)
        {
            return None;
        }
        Some(
            if self.name.is_some() {
                NAME_SPECIFICITY
            } else {
                0
            } + (self.first_blank << SPECIFICITY_COMPONENT_BITS)
                + self.left_right,
        )
    }
    fn could_match(&self, request: &PageRequest, name_known: bool, side_known: bool) -> bool {
        [PageSide::Left, PageSide::Right].into_iter().any(|side| {
            (!side_known || side == request.side)
                && self
                    .matching_specificity(&PageRequest {
                        name: if name_known {
                            request.name.clone()
                        } else {
                            self.name.clone()
                        },
                        side,
                        ..request.clone()
                    })
                    .is_some()
        })
    }
}

#[derive(Clone)]
pub(super) enum PageValue {
    Value(Arc<Component>),
    Pending {
        value: variables::Value,
        namespace: FontNamespace,
        namespaces: Option<crate::selector::Namespaces>,
    },
    Wide(WideKeyword),
}

#[derive(Clone)]
struct OrderedDeclaration {
    value: PropertyDeclaration,
    context: DeclarationContext,
}
fn ordered(
    values: Vec<PropertyDeclaration>,
    context: DeclarationContext,
    order: &mut usize,
) -> Vec<OrderedDeclaration> {
    values
        .into_iter()
        .map(|mut value| {
            value.origin = context.origin;
            value.layer = context.layer;
            let context = DeclarationContext {
                order: *order,
                ..context
            };
            *order += 1;
            OrderedDeclaration { value, context }
        })
        .collect()
}

#[derive(Clone)]
pub(super) struct PageRule {
    selectors: PageSelectors,
    declarations: Vec<OrderedDeclaration>,
    descriptors: Vec<(Descriptor, Candidate<PageValue>)>,
    margins: [Vec<OrderedDeclaration>; MarginLocation::ALL.len()],
    contextual_descriptors: bool,
}
impl PageRule {
    pub(super) fn depends_on_name(&self, request: &PageRequest, side_known: bool) -> bool {
        self.selectors.0.iter().any(|selector| {
            selector.name.is_some() && selector.could_match(request, false, side_known)
        })
    }

    pub(super) fn depends_on_side(&self, request: &PageRequest, name_known: bool) -> bool {
        self.selectors.0.iter().any(|selector| {
            (selector.left || selector.right) && selector.could_match(request, name_known, false)
        })
    }

    pub(crate) fn parse(
        selectors: PageSelectors,
        items: Vec<PageItem>,
        context: DeclarationContext,
        namespace: FontNamespace,
        paper_qualified: bool,
        diagnostics: &RefCell<Vec<StyleDiagnostic>>,
        order: &mut usize,
    ) -> Result<Self, StyleError> {
        let mut rule = Self {
            selectors,
            declarations: Vec::new(),
            descriptors: Vec::new(),
            margins: std::array::from_fn(|_| Vec::new()),
            contextual_descriptors: false,
        };
        for item in items {
            match item {
                PageItem::Declaration(declaration) => {
                    if let Some(descriptor) = Descriptor::find(PAGE_AT_RULE, &declaration.name) {
                        if paper_qualified && descriptor == Descriptor::PageSize {
                            continue;
                        }
                        let Some(value) = page_value(descriptor, &declaration, namespace)? else {
                            continue;
                        };
                        rule.contextual_descriptors |= matches!(&value,PageValue::Pending{value,..} if value.requires_context());
                        let context = DeclarationContext {
                            order: *order,
                            ..context
                        };
                        *order += 1;
                        rule.descriptors.push((
                            descriptor,
                            Candidate::new(value, context, declaration.important)
                                .with_location(Some(Arc::new(declaration.location))),
                        ));
                    } else {
                        let base = declaration.location.base.clone();
                        rule.declarations.extend(ordered(
                            parsed(vec![declaration], &base, diagnostics, namespace)?,
                            context,
                            order,
                        ));
                    }
                }
                PageItem::Margin(location, declarations) => {
                    for declaration in declarations {
                        let base = declaration.location.base.clone();
                        rule.margins[location as usize].extend(ordered(
                            parsed(vec![declaration], &base, diagnostics, namespace)?,
                            context,
                            order,
                        ));
                    }
                }
            }
        }
        Ok(rule)
    }
}

fn page_value(
    descriptor: Descriptor,
    declaration: &Declaration,
    namespace: FontNamespace,
) -> Result<Option<PageValue>, StyleError> {
    let pending = match variables::Value::with_property_substitutions(&declaration.value) {
        Ok(value) => value,
        Err(StyleError::Invalid(_)) => return Ok(None),
        Err(error) => return Err(error),
    };
    Ok(
        if let Some(keyword) = WideKeyword::parse(&declaration.value) {
            Some(PageValue::Wide(keyword))
        } else if let Some(value) = pending {
            Some(PageValue::Pending {
                value,
                namespace,
                namespaces: declaration.namespaces.clone(),
            })
        } else if implemented_page_descriptor(descriptor) {
            match declaration::page_value(descriptor, &declaration.value)? {
                Some(Specified::Wide(keyword)) => Some(PageValue::Wide(keyword)),
                Some(Specified::Value(value)) => Some(PageValue::Value(value)),
                _ => None,
            }
        } else {
            declaration::descriptor(descriptor, &declaration.value)?
                .map(|value| PageValue::Value(Arc::new(value)))
        },
    )
}

#[derive(Clone, Default)]
pub(super) struct MatchedPageRules {
    pub declarations: Cascaded,
    pub margins: [Cascaded; MarginLocation::ALL.len()],
    descriptors: HashMap<Descriptor, Vec<Candidate<PageValue>>>,
    pub contextual_descriptors: bool,
}
impl Stylesheet {
    pub fn resolve_page(
        &self,
        root: &crate::Style,
        request: PageRequest,
        first_viewport: Option<[f32; 2]>,
        resolver: &dyn ferri_pdf_resources::ResourceResolver,
    ) -> Result<ResolvedPage, StyleError> {
        let rules = self.match_page(&request);
        let (geometry, style) =
            crate::resolution::page(self, root, rules.clone(), first_viewport, resolver)?;
        let viewport = first_viewport.unwrap_or(geometry.area);
        let production = self.page_production_for(&style, &rules, viewport, resolver)?;
        let margins = self.page_margins_for(&style, &rules, viewport, resolver)?;
        Ok(ResolvedPage {
            request,
            geometry,
            style,
            production,
            margins,
        })
    }
    pub(super) fn match_page(&self, request: &PageRequest) -> MatchedPageRules {
        let mut matched = MatchedPageRules::default();
        for rule in &self.page_rules {
            let Some(specificity) = rule.selectors.matching_specificity(request) else {
                continue;
            };
            for declaration in &rule.declarations {
                cascade::insert(
                    &mut matched.declarations,
                    &declaration.value,
                    DeclarationContext {
                        specificity,
                        ..declaration.context
                    },
                );
            }
            for (location, declarations) in rule.margins.iter().enumerate() {
                for declaration in declarations {
                    cascade::insert(
                        &mut matched.margins[location],
                        &declaration.value,
                        DeclarationContext {
                            specificity,
                            ..declaration.context
                        },
                    );
                }
            }
            for (descriptor, value) in &rule.descriptors {
                let mut value = value.clone();
                value.context.specificity = specificity;
                matched
                    .descriptors
                    .entry(*descriptor)
                    .or_default()
                    .push(value);
            }
            matched.contextual_descriptors |= rule.contextual_descriptors;
        }
        for values in matched.descriptors.values_mut() {
            sort(values);
        }
        matched
    }
}
impl MatchedPageRules {
    pub(crate) fn validate_descriptors(
        &self,
        sheet: &Stylesheet,
        page: &crate::Style,
        viewport: [f32; 2],
        resolver: &dyn ferri_pdf_resources::ResourceResolver,
    ) -> Result<(), StyleError> {
        let mut descriptors = self
            .descriptors
            .keys()
            .copied()
            .filter(|descriptor| !implemented_page_descriptor(*descriptor))
            .collect::<Vec<_>>();
        descriptors.sort_unstable_by_key(|descriptor| descriptor.data().name);
        for descriptor in descriptors {
            if let Some(value) = self
                .descriptor_value(sheet, descriptor, page, viewport, resolver)?
                .0
            {
                unimplemented_descriptor(descriptor, &value, PAGE_AT_RULE)?;
            }
        }
        Ok(())
    }
    pub(crate) fn descriptor_value(
        &self,
        sheet: &Stylesheet,
        descriptor: Descriptor,
        context: &crate::Style,
        viewport: [f32; 2],
        resolver: &dyn ferri_pdf_resources::ResourceResolver,
    ) -> Result<(Option<Arc<Component>>, Option<SourceLocation>), StyleError> {
        self.descriptor_with(descriptor, |pending, namespace, namespaces, _| {
            crate::resolution::substitute_known(
                sheet, context, pending, namespace, namespaces, viewport, resolver,
            )
        })
    }
    pub(crate) fn descriptor_source(&self, descriptor: Descriptor) -> Option<SourceLocation> {
        self.descriptors
            .get(&descriptor)?
            .last()?
            .location
            .as_deref()
            .cloned()
    }
    pub(crate) fn descriptor_with(
        &self,
        descriptor: Descriptor,
        mut substitute: impl FnMut(
            &variables::Value,
            FontNamespace,
            Option<&crate::selector::Namespaces>,
            Option<&SourceLocation>,
        ) -> Result<Option<variables::Expanded>, StyleError>,
    ) -> Result<(Option<Arc<Component>>, Option<SourceLocation>), StyleError> {
        let values = self
            .descriptors
            .get(&descriptor)
            .map_or(&[][..], Vec::as_slice);
        let winner = choose(
            values,
            |index, value, _| {
                Ok(match value {
                    PageValue::Value(value) => Specified::Value(value.clone()),
                    PageValue::Wide(keyword) => Specified::Wide(*keyword),
                    PageValue::Pending {
                        value,
                        namespace,
                        namespaces,
                    } => match substitute(
                        value,
                        *namespace,
                        namespaces.as_ref(),
                        values[index].location.as_deref(),
                    )? {
                        Some(source) => declaration::page_value(descriptor, source.as_str())?
                            .unwrap_or(Specified::Wide(WideKeyword::Unset)),
                        None => Specified::Wide(WideKeyword::Unset),
                    },
                })
            },
            |value| match value {
                Specified::Wide(keyword) => Some(*keyword),
                _ => None,
            },
        )?;
        Ok(match winner {
            Some((index, Specified::Value(value))) => {
                (Some(value), values[index].location.as_deref().cloned())
            }
            _ => (None, None),
        })
    }
}
