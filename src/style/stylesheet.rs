use crate::fonts::{
    BasePalette, FaceSettings, FontBook, FontDescriptor, FontFaceStyle, FontNamespace, FontRange,
    FontSources, FontWeight, FontWidth,
};
use crate::resources::{ResourceBase, ResourceResolver};
use crate::style::{
    DEFAULT_PAGE_MARGIN, PrintSettings, StyleDiagnostic, StyleDiagnosticKind, StyleError,
    cascade::DeclarationContext,
    component::Component,
    conditions,
    declaration::{self, CUSTOM_PROPERTY_PREFIX, PropertyDeclaration, Specified},
    grammar::{Descriptor, Keyword, Property},
    layers::LayerTree,
    loader::{self, PendingKind, PreparedStylesheets, SourceDiagnostic},
    page_values::page_size,
    parser::{ContentRule, Declaration, absolute_urls},
    selector::RuleSelector,
    units::Relative,
    values::{face_style, face_weight, face_width, family_name, font_sources},
    variables,
    wide_keywords::WideKeyword,
};
use std::{cell::RefCell, collections::HashMap};

pub(super) const PAGE_AT_RULE: &str = "@page";
const FONT_FACE_AT_RULE: &str = "@font-face";
const PALETTE_AT_RULE: &str = "@font-palette-values";
pub(super) const PAGE_MARGINS: [Property; 4] = [
    Property::MarginTop,
    Property::MarginRight,
    Property::MarginBottom,
    Property::MarginLeft,
];

/// A stylesheet and the directory for resolving its relative resource references.
#[cfg(test)]
pub(crate) struct StylesheetSource {
    pub text: String,
    pub base: ResourceBase,
}

#[derive(Clone)]
pub(super) struct StyleRule {
    pub(super) selectors: Vec<RuleSelector>,
    pub(super) declarations: Vec<PropertyDeclaration>,
    pub(super) scope: Option<usize>,
    pub(super) tree_context: crate::dom::TreeContext,
    pub(super) encapsulation_order: usize,
}

pub(crate) struct Stylesheet {
    pub(super) environment: crate::style::print::PrintEnvironment,
    pub(super) page_rules: Vec<crate::style::page_rules::PageRule>,
    pub(super) first_page: crate::style::PageRequest,
    pub(super) page_progression_rtl: bool,
    pub(super) rules: Vec<StyleRule>,
    pub(super) layers: LayerTree,
    pub(super) scopes: Vec<crate::style::scope::Scope>,
    source_diagnostics: Vec<SourceDiagnostic>,
    pub(super) paper: [f32; 2],
    pub(super) document_base: ResourceBase,
    pub(super) diagnostics: RefCell<Vec<StyleDiagnostic>>,
    pub(super) rule_indices: [Vec<usize>; crate::style::selector::SELECTOR_TARGET_COUNT],
    pub page: [f32; 2],
    pub margin: [f32; 4],
    pub(super) page_extent: [f32; 2],
    pub(super) page_content_size: [f32; 2],
    pub(super) page_padding: [f32; 4],
    pub(super) page_border: [f32; 4],
    pub fonts: FontBook,
    font_namespaces: HashMap<crate::dom::TreeScopeId, FontNamespace>,
}

/// Declarations with relative `url()`s resolved against the declaring stylesheet.
pub(super) fn parsed(
    declarations: Vec<Declaration>,
    base: &ResourceBase,
    diagnostics: &RefCell<Vec<StyleDiagnostic>>,
    namespace: FontNamespace,
) -> Result<Vec<PropertyDeclaration>, StyleError> {
    let mut output = Vec::new();
    for mut declaration in declarations {
        let before = output.len();
        let result = if !declaration.name.starts_with(CUSTOM_PROPERTY_PREFIX)
            && Property::from_name(&declaration.name).is_some()
        {
            absolute_urls(&mut declaration, base)
        } else {
            Ok(())
        }
        .and_then(|()| declaration::parse(&declaration, &mut output));
        for parsed in &mut output[before..] {
            parsed.font_namespace = namespace;
        }
        match result {
            Ok(()) if output.len() > before => {}
            Ok(()) | Err(StyleError::Invalid(_)) => {
                diagnostics.borrow_mut().push(StyleDiagnostic {
                    location: declaration.location.clone(),
                    kind: StyleDiagnosticKind::InvalidDeclaration,
                    context: format!("{}: {}", declaration.name, declaration.value),
                });
            }
            Err(error) => return Err(error),
        }
    }
    Ok(output)
}

/// A valid descriptor the engine does not implement is an error unless it is initial.
pub(super) fn unimplemented_descriptor(
    descriptor: Descriptor,
    value: &Component,
    context: &str,
) -> Result<(), StyleError> {
    if declaration::descriptor(descriptor, descriptor.data().initial)?.as_ref() == Some(value) {
        return Ok(());
    }
    Err(StyleError::Unsupported(format!(
        "{context} descriptor: {}: {value}",
        descriptor.data().name
    )))
}

/// `override-colors` entries; `None` makes the descriptor invalid, as a colour that is not
/// absolute does (CSS Fonts 4 §9.2.3).
fn override_colors(
    value: &Component,
    relative: &Relative<'_>,
) -> Result<Option<Vec<(u32, crate::paint::Color)>>, StyleError> {
    let mut colors = Vec::new();
    for entry in value.items() {
        let [index, color] = entry.items() else {
            return Ok(None);
        };
        let Ok(index) = u32::try_from(crate::style::values::integer(index, relative)?) else {
            return Ok(None);
        };
        if matches!(
            color.unwrap(),
            Component::Color(cssparser_color::Color::CurrentColor)
        ) {
            return Ok(None);
        }
        colors.push((
            index,
            crate::style::colors::color(color, relative, crate::paint::Color::BLACK)?,
        ));
    }
    Ok(Some(colors))
}

enum FaceValue {
    Family(String),
    Weight(FontRange<FontWeight>),
    Width(FontRange<FontWidth>),
    Style(FontFaceStyle),
    Sources(Vec<crate::fonts::FontSource>),
    Features(std::sync::Arc<[crate::fonts::FontFeature]>),
    Variations(std::sync::Arc<[crate::fonts::FontVariation]>),
    Initial,
}
fn face_value(
    descriptor: Descriptor,
    value: &Component,
    relative: &Relative<'_>,
) -> Result<FaceValue, StyleError> {
    Ok(match descriptor {
        Descriptor::FontFaceFontFamily => FaceValue::Family(family_name(value)),
        Descriptor::FontFaceFontWeight => FaceValue::Weight(face_weight(value, relative)?),
        Descriptor::FontFaceFontStyle => FaceValue::Style(face_style(value, relative)?),
        Descriptor::FontFaceFontWidth | Descriptor::FontFaceFontStretch => {
            FaceValue::Width(face_width(value, relative)?)
        }
        Descriptor::FontFaceSrc => FaceValue::Sources(font_sources(value)?),
        Descriptor::FontFaceFontFeatureSettings => {
            FaceValue::Features(crate::style::font_values::feature_settings(value)?)
        }
        Descriptor::FontFaceFontVariationSettings => FaceValue::Variations(
            crate::style::font_values::variation_settings(value, relative)?,
        ),
        _ => {
            unimplemented_descriptor(descriptor, value, FONT_FACE_AT_RULE)?;
            FaceValue::Initial
        }
    })
}
pub(super) fn supports_descriptor(at_rule: &str, decl: &Declaration) -> bool {
    let Some(descriptor) = Descriptor::find(at_rule, &decl.name) else {
        return at_rule == PAGE_AT_RULE
            && declaration::supports_declaration(&decl.name, &decl.value);
    };
    if implemented_page_descriptor(descriptor) {
        if WideKeyword::parse(&decl.value).is_some() {
            return true;
        }
        match variables::Value::with_substitutions(&decl.value) {
            Ok(Some(_)) => return true,
            Err(_) => return false,
            Ok(None) => {}
        }
        return match declaration::page_value(descriptor, &decl.value) {
            Ok(Some(Specified::Wide(_))) => true,
            Ok(Some(Specified::Value(value))) => supports_page_value(descriptor, &value),
            _ => false,
        };
    }
    let Ok(Some(value)) = declaration::descriptor(descriptor, &decl.value) else {
        return false;
    };
    if at_rule == FONT_FACE_AT_RULE {
        return !decl.important
            && face_value(descriptor, &value, &Relative::initial()).is_ok_and(
                |value| match value {
                    FaceValue::Sources(sources) => !sources.is_empty(),
                    _ => true,
                },
            );
    }
    unimplemented_descriptor(descriptor, &value, at_rule).is_ok()
}

pub(super) fn supports_page_value(descriptor: Descriptor, value: &Component) -> bool {
    match descriptor {
        Descriptor::PageSize => {
            page_size(value, &Relative::validation(), crate::style::DEFAULT_PAGE).is_ok()
        }
        Descriptor::PageBleed => {
            value.keyword() == Some(Keyword::Auto)
                || crate::style::values::length(value, &Relative::validation()).is_ok()
        }
        Descriptor::PageMarks => crate::style::PageMarks::from_component(value).is_ok(),
        _ => false,
    }
}

pub(super) fn implemented_page_descriptor(descriptor: Descriptor) -> bool {
    matches!(
        descriptor,
        Descriptor::PageSize | Descriptor::PageBleed | Descriptor::PageMarks
    )
}

impl Stylesheet {
    /// Copies prepared rules and page context; source diagnostics remain with the original owner.
    #[must_use]
    #[cfg(test)]
    pub(crate) fn fork_for_layout(&self) -> Self {
        Self {
            environment: self.environment.clone(),
            page_rules: self.page_rules.clone(),
            first_page: self.first_page.clone(),
            page_progression_rtl: self.page_progression_rtl,
            rules: self.rules.clone(),
            layers: self.layers.clone(),
            scopes: self.scopes.clone(),
            source_diagnostics: Vec::new(),
            paper: self.paper,
            document_base: self.document_base.clone(),
            diagnostics: RefCell::default(),
            rule_indices: self.rule_indices.clone(),
            page: self.page,
            margin: self.margin,
            page_extent: self.page_extent,
            page_content_size: self.page_content_size,
            page_padding: self.page_padding,
            page_border: self.page_border,
            fonts: self.fonts.clone(),
            font_namespaces: self.font_namespaces.clone(),
        }
    }

    pub(crate) fn first_page_request(&self) -> &crate::style::PageRequest {
        &self.first_page
    }
    pub(crate) fn page_progression_rtl(&self) -> bool {
        self.page_progression_rtl
    }

    #[cfg(test)]
    pub(crate) fn resolve_page_geometry(
        &mut self,
        root: &crate::style::Style,
    ) -> Result<(), StyleError> {
        self.resolve_page_geometry_with_resolver(root, &crate::resources::FileResolver::default())
    }

    #[cfg(test)]
    pub(crate) fn parse(sources: &[StylesheetSource]) -> Result<Self, StyleError> {
        let base = sources
            .first()
            .map(|source| source.base.clone())
            .unwrap_or_default();
        Self::parse_with_base(sources, &base)
    }

    #[cfg(test)]
    pub(crate) fn parse_with_base(
        sources: &[StylesheetSource],
        document_base: &ResourceBase,
    ) -> Result<Self, StyleError> {
        Self::parse_with_origins(sources, &[], document_base)
    }

    #[cfg(test)]
    pub(crate) fn parse_with_origins(
        author: &[StylesheetSource],
        user: &[StylesheetSource],
        document_base: &ResourceBase,
    ) -> Result<Self, StyleError> {
        Self::parse_with_settings(author, user, document_base, &PrintSettings::default())
    }

    #[cfg(test)]
    pub(crate) fn parse_with_settings(
        author: &[StylesheetSource],
        user: &[StylesheetSource],
        document_base: &ResourceBase,
        settings: &PrintSettings,
    ) -> Result<Self, StyleError> {
        Self::from_prepared(
            loader::inline(author, user, document_base, settings)?,
            settings,
            None,
        )
    }

    pub(crate) fn prepare(
        tree: &crate::dom::DocumentTree,
        sources: &crate::style::StylesheetSources,
        resolver: &dyn ResourceResolver,
        settings: &PrintSettings,
        decode: crate::style::CssDecoder,
    ) -> Result<Self, StyleError> {
        Self::from_prepared(
            loader::prepare_with_limits(
                tree,
                sources,
                resolver,
                settings,
                decode,
                loader::Limits::default(),
            )?,
            settings,
            Some(tree),
        )
    }

    pub(crate) fn take_source_diagnostics(&mut self) -> Vec<SourceDiagnostic> {
        std::mem::take(&mut self.source_diagnostics)
    }

    fn from_prepared(
        prepared: PreparedStylesheets,
        settings: &PrintSettings,
        tree: Option<&crate::dom::DocumentTree>,
    ) -> Result<Self, StyleError> {
        let PreparedStylesheets {
            all,
            mut layers,
            mut scopes,
            style_scopes,
            conditions,
            environment,
            diagnostics,
            source_diagnostics,
            document_base,
        } = prepared;
        let applicable = conditions::applicability(&conditions, &environment);
        let mut content = Vec::new();
        for rule in all {
            if rule
                .condition
                .is_some_and(|index| !applicable[index].matching)
            {
                continue;
            }
            let parent = scopes[rule.scope].expect("an active rule has an active enclosing layer");
            match rule.kind {
                PendingKind::Content(content_rule) => content.push((
                    content_rule,
                    rule.base,
                    rule.origin,
                    parent,
                    rule.condition
                        .is_some_and(|index| applicable[index].paper_dependent),
                    rule.style_scope,
                    rule.tree_scope,
                    rule.encapsulation_order,
                )),
                PendingKind::LayerStatement(names) => {
                    for name in names {
                        layers.register(parent, Some(&name));
                    }
                }
                PendingKind::LayerBlock(name, scope) => {
                    scopes[scope] = Some(layers.register(parent, name.as_deref()));
                }
            }
        }
        layers.freeze();
        let mut font_book = FontBook::with_preferences(settings.fonts.clone());
        let mut font_namespaces = HashMap::new();
        if let Some(tree) = tree {
            font_namespaces.insert(tree.root_element().tree_scope(), FontNamespace::ROOT);
            for element in tree.shadow_including_elements() {
                if let Some(host) = element.shadow_host() {
                    let parent = font_namespaces[&host.tree_scope()];
                    font_namespaces
                        .entry(element.tree_scope())
                        .or_insert_with(|| font_book.child_namespace(parent));
                }
            }
        }
        let mut sheet = Self {
            environment,
            page_rules: Vec::new(),
            first_page: crate::style::PageRequest::default(),
            page_progression_rtl: false,
            rules: Vec::new(),
            layers,
            scopes: style_scopes,
            source_diagnostics,
            paper: settings.paper,
            document_base: document_base.clone(),
            diagnostics,
            rule_indices: std::array::from_fn(|_| Vec::new()),
            page: settings.paper,
            margin: [DEFAULT_PAGE_MARGIN; 4],
            page_extent: settings.paper,
            page_content_size: settings
                .paper
                .map(|dimension| dimension - 2.0 * DEFAULT_PAGE_MARGIN),
            page_padding: [0.0; 4],
            page_border: [0.0; 4],
            fonts: font_book,
            font_namespaces,
        };
        let mut fonts = Vec::new();
        let mut feature_values = Vec::new();
        let mut palettes = Vec::new();
        let mut page_order = 0;
        for (
            order,
            (
                rule,
                source_base,
                origin,
                node,
                paper_qualified,
                scope,
                tree_scope,
                encapsulation_order,
            ),
        ) in content.into_iter().enumerate()
        {
            let layer = sheet.layers.id(node);
            let namespace = sheet.font_namespace(tree_scope);
            match rule {
                ContentRule::Style(selectors, declarations) => {
                    let index = sheet.rules.len();
                    for selector in &selectors {
                        let indices = &mut sheet.rule_indices[selector.target_index()];
                        if indices.last() != Some(&index) {
                            indices.push(index);
                        }
                    }
                    sheet.rules.push(StyleRule {
                        selectors,
                        scope,
                        tree_context: crate::style::selector::tree_context(origin, tree_scope),
                        encapsulation_order,
                        declarations: parsed(
                            declarations,
                            &source_base,
                            &sheet.diagnostics,
                            namespace,
                        )?
                        .into_iter()
                        .map(|mut declaration| {
                            declaration.origin = origin;
                            declaration.layer = layer;
                            declaration
                        })
                        .collect(),
                    });
                }
                ContentRule::Page { selectors, items } => {
                    sheet
                        .page_rules
                        .push(crate::style::page_rules::PageRule::parse(
                            selectors,
                            items,
                            DeclarationContext {
                                encapsulation_order,
                                origin,
                                attached: false,
                                layer,
                                specificity: 0,
                                scope_proximity: None,
                                order: 0,
                            },
                            namespace,
                            paper_qualified,
                            &sheet.diagnostics,
                            &mut page_order,
                        )?);
                }
                ContentRule::Font(declarations, location) => {
                    fonts.push((
                        (origin, layer, order),
                        declarations,
                        location,
                        source_base,
                        namespace,
                    ));
                }
                ContentRule::FeatureValues {
                    families,
                    definitions,
                } => {
                    feature_values.push(((origin, layer, order), families, definitions, namespace));
                }
                ContentRule::Palette(name, declarations, location) => {
                    palettes.push((
                        (origin, layer, order),
                        name,
                        declarations,
                        location,
                        namespace,
                    ));
                }
            }
        }
        fonts.sort_by_key(|(priority, ..)| *priority);
        for (_, declarations, location, source_base, namespace) in fonts {
            match sheet.font_face(&declarations, source_base, namespace) {
                Ok(()) => {}
                Err(StyleError::Invalid(context)) => {
                    sheet.diagnostics.borrow_mut().push(StyleDiagnostic {
                        location,
                        kind: StyleDiagnosticKind::InvalidRule,
                        context,
                    });
                }
                Err(error) => return Err(error),
            }
        }
        feature_values.sort_by_key(|(priority, ..)| *priority);
        for (_, families, definitions, namespace) in feature_values {
            for family in &families {
                for definition in &definitions {
                    sheet.fonts.define_feature_value(
                        namespace,
                        family,
                        definition.kind,
                        &definition.name,
                        definition.values.clone(),
                    );
                }
            }
        }
        palettes.sort_by_key(|(priority, ..)| *priority);
        for (_, name, declarations, location, namespace) in palettes {
            match sheet.palette_values(&name, &declarations, namespace) {
                Ok(()) => {}
                Err(StyleError::Invalid(context)) => {
                    sheet.diagnostics.borrow_mut().push(StyleDiagnostic {
                        location,
                        kind: StyleDiagnosticKind::InvalidRule,
                        context,
                    });
                }
                Err(error) => return Err(error),
            }
        }
        Ok(sheet)
    }

    pub(crate) fn take_diagnostics(&mut self) -> Vec<StyleDiagnostic> {
        std::mem::take(self.diagnostics.get_mut())
    }

    pub(super) fn font_namespace(&self, scope: Option<crate::dom::TreeScopeId>) -> FontNamespace {
        scope
            .and_then(|scope| self.font_namespaces.get(&scope))
            .copied()
            .unwrap_or(FontNamespace::ROOT)
    }

    /// An `@font-palette-values` rule (CSS Fonts 4 §9.2). Invalid descriptors are ignored; a
    /// rule without `font-family` is invalid, and generic families match no face.
    fn palette_values(
        &mut self,
        name: &str,
        declarations: &[Declaration],
        namespace: FontNamespace,
    ) -> Result<(), StyleError> {
        let mut families = None;
        let mut base = BasePalette::Index(0);
        let mut overrides = Vec::new();
        let relative = Relative {
            paper: self.paper,
            ..self.initial_relative()
        };
        for d in declarations {
            let Some(descriptor) = Descriptor::find(PALETTE_AT_RULE, &d.name) else {
                continue;
            };
            if variables::Value::with_substitutions(&d.value)?.is_some() {
                return Err(StyleError::Unsupported(format!(
                    "{PALETTE_AT_RULE} descriptor: {}: {} (root-element substitution)",
                    d.name, d.value
                )));
            }
            let Some(value) = declaration::descriptor(descriptor, &d.value)? else {
                continue;
            };
            match descriptor {
                Descriptor::FontPaletteValuesFontFamily => {
                    families = Some(
                        declaration::value(Property::FontFamily, &d.value, &mut 0)?
                            .map(|list| crate::style::values::families(&list))
                            .unwrap_or_default(),
                    );
                }
                Descriptor::FontPaletteValuesBasePalette => {
                    base = match value.unwrap() {
                        Component::Keyword(Keyword::Light) => BasePalette::Light,
                        Component::Keyword(Keyword::Dark) => BasePalette::Dark,
                        _ => BasePalette::Index(
                            u32::try_from(crate::style::values::integer(&value, &relative)?)
                                .map_err(|_| StyleError::Invalid(d.value.clone()))?,
                        ),
                    };
                }
                Descriptor::FontPaletteValuesOverrideColors => {
                    if let Some(colors) = override_colors(&value, &relative)? {
                        overrides = colors;
                    }
                }
                _ => {}
            }
        }
        let families = families.ok_or("font-palette-values missing font-family")?;
        let named = families
            .into_iter()
            .map(|family| match family {
                crate::fonts::FontFamily::Named(name) => Some(name),
                crate::fonts::FontFamily::Generic(_) => None,
            })
            .collect::<Option<Vec<_>>>()
            .unwrap_or_default();
        self.fonts
            .define_palette(namespace, name, named, base, overrides);
        Ok(())
    }

    fn font_face(
        &mut self,
        declarations: &[Declaration],
        base: ResourceBase,
        namespace: FontNamespace,
    ) -> Result<(), StyleError> {
        let mut family = None;
        let mut src = None;
        let mut weight = FontRange::Auto;
        let mut width = FontRange::Auto;
        let mut style = FontFaceStyle::Auto;
        let mut settings = FaceSettings::default();
        let relative = Relative {
            paper: self.paper,
            ..self.initial_relative()
        };
        for d in declarations {
            let Some(descriptor) = Descriptor::find(FONT_FACE_AT_RULE, &d.name) else {
                continue;
            };
            let Some(value) = declaration::descriptor(descriptor, &d.value)? else {
                continue;
            };
            match face_value(descriptor, &value, &relative)? {
                FaceValue::Family(value) => family = Some(value),
                FaceValue::Weight(value) => weight = value,
                FaceValue::Width(value) => width = value,
                FaceValue::Style(value) => style = value,
                FaceValue::Sources(value) if !value.is_empty() => src = Some(value),
                FaceValue::Sources(_) | FaceValue::Initial => {}
                FaceValue::Features(value) => settings.features = value,
                FaceValue::Variations(value) => settings.variations = value,
            }
        }
        let family = family.ok_or("font-face missing family")?;
        let src = src.ok_or("font-face missing src")?;
        self.fonts.declare_in_namespace(
            namespace,
            FontDescriptor {
                family,
                weight,
                width,
                style,
            },
            settings,
            FontSources { sources: src, base },
        );
        Ok(())
    }

    /// Resolve first-page geometry without provisional viewport values.
    #[cfg(test)]
    pub(crate) fn resolve_page_geometry_with_resolver(
        &mut self,
        root: &crate::style::Style,
        resolver: &dyn ResourceResolver,
    ) -> Result<(), StyleError> {
        let geometry = crate::style::resolution::provided(self, root, resolver)?;
        self.set_page_geometry(&geometry);
        Ok(())
    }
}
