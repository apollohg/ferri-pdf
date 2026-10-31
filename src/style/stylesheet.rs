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
    declaration::{self, CUSTOM_PROPERTY_PREFIX, Declared, PropertyDeclaration, Specified},
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
use std::{cell::RefCell, collections::HashMap, sync::Arc};

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
    /// The innermost enclosing `@container` rule.
    pub(super) container: Option<usize>,
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
    pub(super) container_rules: Vec<crate::style::container_queries::ContainerRule>,
    pub(super) query_sizes: crate::style::QuerySizes,
    source_diagnostics: Vec<SourceDiagnostic>,
    pub(super) paper: [f32; 2],
    pub(super) document_base: ResourceBase,
    pub(super) diagnostics: RefCell<Vec<StyleDiagnostic>>,
    pub(super) degradations: std::rc::Rc<crate::style::Degradations>,
    pub(super) rule_indices: [Vec<usize>; crate::style::selector::SELECTOR_TARGET_COUNT],
    pub page: [f32; 2],
    pub margin: [f32; 4],
    pub(super) page_extent: [f32; 2],
    pub(super) page_content_size: [f32; 2],
    pub(super) page_padding: [f32; 4],
    pub(super) page_border: [f32; 4],
    pub fonts: FontBook,
    font_namespaces: HashMap<crate::dom::TreeScopeId, FontNamespace>,
    pub(super) counter_styles: Arc<crate::style::counter_style::CounterStyles>,
    /// `@font-palette-values` rules in cascade order, which resolve once the root element's
    /// style is known.
    palette_rules: Vec<PaletteRule>,
    /// Each tree scope's enclosing one, for loosely matched tree-scoped names.
    scope_parents: HashMap<FontNamespace, FontNamespace>,
    /// `@position-try` rules by name: the declarations of the last in cascade order
    /// (CSS Anchor Positioning 1 §6.4).
    position_try: HashMap<crate::style::anchor_values::AnchorName, Arc<[PropertyDeclaration]>>,
    /// The anchor frames the previous layout measured.
    pub(crate) anchor_nodes:
        RefCell<HashMap<crate::style::PageStyleTarget, crate::style::anchor_values::AnchorNode>>,
    anchor_order: HashMap<crate::dom::ElementId, [usize; 3]>,
    pub(super) anchor_frames: crate::style::anchor_values::AnchorFrames,
    /// Custom property registrations from `@property` rules.
    pub(super) registrations: Arc<crate::style::registration::Registrations>,
}

fn anchor_order(tree: &crate::dom::DocumentTree) -> HashMap<crate::dom::ElementId, [usize; 3]> {
    let mut order: HashMap<_, [usize; 3]> = HashMap::new();
    let mut next = 0;
    let mut pending = vec![(tree.root_element(), false)];
    while let Some((element, closing)) = pending.pop() {
        if closing {
            order
                .get_mut(&element.id())
                .expect("the element's start has an order")[2] = next;
            next += 1;
        } else {
            order.insert(element.id(), [next, next + 1, next + 1]);
            next += 2;
            pending.push((element, true));
            pending.extend(
                element
                    .children(crate::dom::TreeRelation::Formatting)
                    .rev()
                    .map(|child| (child, false)),
            );
        }
    }
    order
}

/// An `@font-palette-values` rule awaiting its root-element context (CSS Fonts 4 §9.2).
#[derive(Clone)]
struct PaletteRule {
    name: String,
    declarations: Vec<Declaration>,
    location: crate::style::SourceLocation,
    namespace: FontNamespace,
}

/// A resolved `@font-palette-values` rule: its namespace, name, families, base palette and
/// override colours.
type PaletteDefinition = (
    FontNamespace,
    String,
    Vec<String>,
    BasePalette,
    Vec<(u32, crate::paint::AbsoluteColor)>,
);

/// Declarations with relative `url()`s resolved against the declaring stylesheet.
pub(super) fn parsed(
    declarations: Vec<Declaration>,
    base: &ResourceBase,
    diagnostics: &RefCell<Vec<StyleDiagnostic>>,
    namespace: FontNamespace,
) -> Vec<PropertyDeclaration> {
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
        let declared = format!("{}: {}", declaration.name, declaration.value);
        let (kind, context) = match result {
            Ok(()) if output.len() > before => continue,
            Ok(()) => (StyleDiagnosticKind::InvalidDeclaration, declared),
            Err(error) => (
                StyleDiagnosticKind::of(&error, StyleDiagnosticKind::InvalidDeclaration),
                format!("{declared} ({error})"),
            ),
        };
        diagnostics.borrow_mut().push(StyleDiagnostic {
            location: declaration.location.clone(),
            kind,
            context,
        });
    }
    output
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
) -> Result<Option<Vec<(u32, crate::paint::AbsoluteColor)>>, StyleError> {
    let mut colors = Vec::new();
    for entry in value.items() {
        let [index, color] = entry.items() else {
            return Ok(None);
        };
        let Ok(index) = u32::try_from(crate::style::values::integer(index, relative)?) else {
            return Ok(None);
        };
        if crate::style::resolution::contains_current_color(color) {
            return Ok(None);
        }
        colors.push((
            index,
            crate::style::colors::absolute(color, relative, &crate::paint::AbsoluteColor::BLACK)?,
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
    Features(Arc<[crate::fonts::FontFeature]>),
    Variations(Arc<[crate::fonts::FontVariation]>),
    UnicodeRange(crate::fonts::UnicodeRanges),
    SizeAdjust(f32),
    AscentOverride(Option<f32>),
    DescentOverride(Option<f32>),
    LineGapOverride(Option<f32>),
    Initial,
}
/// `normal | <percentage [0,∞]>` of the metric override descriptors (CSS Fonts 4 §4.11).
fn metric_override(value: &Component, relative: &Relative<'_>) -> Result<Option<f32>, StyleError> {
    if value.keyword() == Some(Keyword::Normal) {
        return Ok(None);
    }
    crate::style::values::percentage(value, relative).map(Some)
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
        Descriptor::FontFaceUnicodeRange => {
            FaceValue::UnicodeRange(crate::fonts::UnicodeRanges::new(
                value
                    .items()
                    .iter()
                    .map(|range| match range.unwrap() {
                        Component::UnicodeRange(start, end) => Ok((*start, *end)),
                        other => Err(crate::style::box_values::unsupported(other)),
                    })
                    .collect::<Result<Vec<_>, StyleError>>()?,
            ))
        }
        Descriptor::FontFaceSizeAdjust => {
            FaceValue::SizeAdjust(crate::style::values::percentage(value, relative)?)
        }
        Descriptor::FontFaceAscentOverride => {
            FaceValue::AscentOverride(metric_override(value, relative)?)
        }
        Descriptor::FontFaceDescentOverride => {
            FaceValue::DescentOverride(metric_override(value, relative)?)
        }
        Descriptor::FontFaceLineGapOverride => {
            FaceValue::LineGapOverride(metric_override(value, relative)?)
        }
        // Fonts load before layout, so every face is ready at the one paint and no display
        // timeline period applies (CSS Fonts 4 §4.9).
        Descriptor::FontFaceFontDisplay => FaceValue::Initial,
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
    if at_rule == crate::style::counter_style::COUNTER_STYLE_AT_RULE {
        return !decl.important && crate::style::counter_style::supported(descriptor, &value);
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
        Descriptor::PagePageOrientation => {
            crate::style::PageOrientation::from_component(value).is_ok()
        }
        _ => false,
    }
}

pub(super) fn implemented_page_descriptor(descriptor: Descriptor) -> bool {
    matches!(
        descriptor,
        Descriptor::PageSize
            | Descriptor::PageBleed
            | Descriptor::PageMarks
            | Descriptor::PagePageOrientation
    )
}

impl Stylesheet {
    /// Whether any rule selects `pseudo`, which otherwise inherits every value it computes.
    pub(crate) fn has_pseudo_rules(&self, pseudo: crate::style::PseudoElement) -> bool {
        !self.rule_indices[crate::style::selector::target_index(Some(pseudo))].is_empty()
    }
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
            container_rules: self.container_rules.clone(),
            query_sizes: self.query_sizes.clone(),
            source_diagnostics: Vec::new(),
            paper: self.paper,
            document_base: self.document_base.clone(),
            diagnostics: RefCell::default(),
            degradations: std::rc::Rc::default(),
            rule_indices: self.rule_indices.clone(),
            page: self.page,
            margin: self.margin,
            page_extent: self.page_extent,
            page_content_size: self.page_content_size,
            page_padding: self.page_padding,
            page_border: self.page_border,
            fonts: self.fonts.clone(),
            font_namespaces: self.font_namespaces.clone(),
            counter_styles: self.counter_styles.clone(),
            palette_rules: self.palette_rules.clone(),
            scope_parents: self.scope_parents.clone(),
            position_try: self.position_try.clone(),
            anchor_frames: self.anchor_frames.clone(),
            anchor_nodes: self.anchor_nodes.clone(),
            anchor_order: self.anchor_order.clone(),
            registrations: self.registrations.clone(),
        }
    }

    /// Registers the custom property an `@property` rule names; rules apply in document
    /// order, so the last valid one wins (CSS Properties and Values API 1 §2.1).
    fn register(
        &self,
        registrations: &mut crate::style::registration::Registrations,
        name: String,
        declarations: &[Declaration],
        location: &crate::style::SourceLocation,
        base: &ResourceBase,
    ) {
        let (registration, invalid) =
            match crate::style::registration::parse(declarations, base, &name) {
                Ok(parsed) => parsed,
                Err(error) => {
                    self.ignore_rule(location.clone(), &format!("@property {name}"), &error);
                    return;
                }
            };
        let mut diagnostics = self.diagnostics.borrow_mut();
        for declaration in invalid {
            diagnostics.push(StyleDiagnostic {
                location: declaration.location.clone(),
                kind: StyleDiagnosticKind::InvalidDeclaration,
                context: format!(
                    "@property {name}: {}: {}",
                    declaration.name, declaration.value
                ),
            });
        }
        match registration {
            Some(registration) => registrations.insert(name, registration),
            None => diagnostics.push(StyleDiagnostic {
                location: location.clone(),
                kind: StyleDiagnosticKind::InvalidRule,
                context: format!("@property {name}"),
            }),
        }
    }

    /// Reports a rule or descriptor the engine ignores because computing it failed with
    /// `error`: an invalid, unsupported or over-limit construct (CSS Syntax 3 §2.2).
    fn ignore_rule(
        &self,
        location: crate::style::SourceLocation,
        context: &str,
        error: &StyleError,
    ) {
        self.diagnostics.borrow_mut().push(StyleDiagnostic {
            location,
            kind: StyleDiagnosticKind::of(error, StyleDiagnosticKind::InvalidRule),
            context: format!("{context}: {error}"),
        });
    }

    pub(crate) fn first_page_request(&self) -> &crate::style::PageRequest {
        &self.first_page
    }
    /// Whether a page rule selects pages by position, so that page requests carry it.
    pub(crate) fn selects_page_positions(&self) -> bool {
        self.page_rules
            .iter()
            .any(crate::style::page_rules::PageRule::selects_positions)
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
        presentation: crate::style::Presentation,
        decode: crate::style::CssDecoder,
    ) -> Result<Self, StyleError> {
        Self::from_prepared(
            loader::prepare_with_limits(
                tree,
                sources,
                resolver,
                settings,
                presentation,
                decode,
                loader::Limits::default(),
            )?,
            settings,
            Some(tree),
        )
    }

    pub(crate) fn presentation(&self) -> crate::style::Presentation {
        self.environment.presentation
    }

    /// The source an HTML `img` element selects for the print medium (HTML §4.8.4.3.7).
    pub(crate) fn image_source(
        &self,
        img: crate::dom::Element<'_>,
    ) -> Option<crate::style::ImageSource> {
        crate::style::source_set::select(img, &self.environment)
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
            container_rules,
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
                // CSS Conditional 5 §5.4: page contexts have no query container, so their rules
                // inside `@container` never apply; name-defining rules are unconditional.
                PendingKind::Content(ContentRule::Page { .. }) if rule.container.is_some() => {}
                PendingKind::Content(content_rule) => content.push((
                    content_rule,
                    rule.base,
                    rule.origin,
                    parent,
                    rule.condition
                        .is_some_and(|index| applicable[index].paper_dependent),
                    rule.style_scope,
                    rule.container,
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
        let mut scope_parents = HashMap::new();
        if let Some(tree) = tree {
            font_namespaces.insert(tree.root_element().tree_scope(), FontNamespace::ROOT);
            for element in tree.shadow_including_elements() {
                if let Some(host) = element.shadow_host() {
                    let parent = font_namespaces[&host.tree_scope()];
                    font_namespaces
                        .entry(element.tree_scope())
                        .or_insert_with(|| {
                            let child = font_book.child_namespace(parent);
                            scope_parents.insert(child, parent);
                            child
                        });
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
            container_rules,
            query_sizes: crate::style::QuerySizes::default(),
            source_diagnostics,
            paper: settings.paper,
            document_base: document_base.clone(),
            diagnostics,
            degradations: std::rc::Rc::default(),
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
            counter_styles: crate::style::counter_style::CounterStyles::predefined(),
            palette_rules: Vec::new(),
            scope_parents: scope_parents.clone(),
            position_try: HashMap::new(),
            anchor_frames: crate::style::anchor_values::AnchorFrames::default(),
            anchor_nodes: RefCell::default(),
            anchor_order: tree.map(anchor_order).unwrap_or_default(),
            registrations: Arc::default(),
        };
        let mut fonts = Vec::new();
        let mut feature_values = Vec::new();
        let mut palettes = Vec::new();
        let mut counter_styles = Vec::new();
        let mut position_try = Vec::new();
        let mut registrations = crate::style::registration::Registrations::default();
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
                container,
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
                        container,
                        tree_context: crate::style::selector::tree_context(origin, tree_scope),
                        encapsulation_order,
                        declarations: parsed(
                            declarations,
                            &source_base,
                            &sheet.diagnostics,
                            namespace,
                        )
                        .into_iter()
                        .map(|mut declaration| {
                            declaration.origin = origin;
                            declaration.layer = layer;
                            declaration
                        })
                        .collect(),
                    });
                }
                // CSS Paged Media 3 §3: page rules style page boxes, which continuous media lacks.
                ContentRule::Page { .. }
                    if sheet.environment.presentation != crate::style::Presentation::Paged => {}
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
                        ));
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
                ContentRule::CounterStyle(name, declarations, location) => {
                    counter_styles.push((
                        (origin, layer, order),
                        name,
                        declarations,
                        location,
                        namespace,
                    ));
                }
                ContentRule::Property(name, declarations, location) => {
                    sheet.register(
                        &mut registrations,
                        name,
                        &declarations,
                        &location,
                        &source_base,
                    );
                }
                ContentRule::PositionTry(name, declarations, location) => {
                    position_try.push((
                        (origin, layer, order),
                        name,
                        declarations,
                        location,
                        source_base,
                        namespace,
                    ));
                }
            }
        }
        sheet.registrations = Arc::new(registrations);
        if sheet.selects_page_positions() {
            sheet.first_page.sequence = Some(crate::style::PageSequence::first());
        }
        // CSS Counter Styles 3 §3: rules of one name cascade atomically; the last in cascade
        // order wins.
        counter_styles.sort_by_key(|(priority, ..)| *priority);
        let mut rules = Vec::new();
        for (_, name, declarations, location, namespace) in counter_styles {
            match crate::style::counter_style::CounterStyleRule::parse(
                &name,
                &declarations,
                namespace,
            ) {
                Ok(Some(rule)) => rules.push(rule),
                Ok(None) => sheet.diagnostics.borrow_mut().push(StyleDiagnostic {
                    location,
                    kind: StyleDiagnosticKind::InvalidRule,
                    context: format!("@counter-style {name}"),
                }),
                Err(error) => {
                    sheet.ignore_rule(location, &format!("@counter-style {name}"), &error);
                }
            }
        }
        sheet.counter_styles = Arc::new(crate::style::counter_style::CounterStyles::new(
            rules,
            scope_parents,
        ));
        // CSS Anchor Positioning 1 §6.4: rules of one name cascade as `@keyframes` rules
        // do, the last in cascade order winning whole.
        position_try.sort_by_key(|(priority, ..)| *priority);
        for (_, name, declarations, location, source_base, namespace) in position_try {
            let declarations =
                sheet.position_try_declarations(declarations, &source_base, &location, namespace);
            sheet.position_try.insert(
                crate::style::anchor_values::AnchorName {
                    name: Arc::from(name),
                    scope: namespace,
                },
                declarations,
            );
        }
        fonts.sort_by_key(|(priority, ..)| *priority);
        for (_, declarations, location, source_base, namespace) in fonts {
            if let Err(error) = sheet.font_face(&declarations, source_base, namespace) {
                sheet.ignore_rule(location, FONT_FACE_AT_RULE, &error);
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
        sheet.palette_rules = palettes
            .into_iter()
            .map(|(_, name, declarations, location, namespace)| PaletteRule {
                name,
                declarations,
                location,
                namespace,
            })
            .collect();
        Ok(sheet)
    }

    /// The declarations of a `@position-try` rule (CSS Anchor Positioning 1 §6.4): only
    /// accepted `@position-try` properties apply, and `!important` makes a declaration
    /// invalid.
    fn position_try_declarations(
        &self,
        declarations: Vec<Declaration>,
        base: &ResourceBase,
        location: &crate::style::SourceLocation,
        namespace: FontNamespace,
    ) -> Arc<[PropertyDeclaration]> {
        let mut accepted = Vec::new();
        for declaration in parsed(declarations, base, &self.diagnostics, namespace) {
            let property = match &declaration.declared {
                Declared::Longhand(property, _) | Declared::Pending(property, _) => Some(*property),
                Declared::Custom(..) => None,
            };
            if !declaration.important && property.is_some_and(crate::style::anchor_values::accepted)
            {
                accepted.push(declaration);
            } else {
                self.diagnostics.borrow_mut().push(StyleDiagnostic {
                    location: declaration.location.as_deref().unwrap_or(location).clone(),
                    kind: StyleDiagnosticKind::InvalidDeclaration,
                    context: format!(
                        "{} in @position-try",
                        property.map_or("custom property", Property::name)
                    ),
                });
            }
        }
        Arc::from(accepted)
    }

    /// The `@position-try` rule a tree-scoped reference names: the rule of that name in its
    /// own tree scope or the nearest enclosing one.
    pub(super) fn position_try(
        &self,
        name: &crate::style::anchor_values::AnchorName,
    ) -> Option<&Arc<[PropertyDeclaration]>> {
        let mut scope = name.scope;
        loop {
            let candidate = crate::style::anchor_values::AnchorName {
                name: name.name.clone(),
                scope,
            };
            if let Some(declarations) = self.position_try.get(&candidate) {
                return Some(declarations);
            }
            scope = *self.scope_parents.get(&scope)?;
        }
    }

    /// Whether a name in tree scope `defining` is visible to references from `referring`:
    /// the same scope or one enclosing it (CSS Scoping 1 §3.5).
    pub(crate) fn scope_includes(&self, defining: FontNamespace, referring: FontNamespace) -> bool {
        std::iter::successors(Some(referring), |scope| {
            self.scope_parents.get(scope).copied()
        })
        .any(|scope| scope == defining)
    }

    pub(super) fn record_anchor(
        &self,
        element: crate::dom::Element<'_>,
        pseudo: Option<crate::style::PseudoElement>,
        style: &crate::style::Style,
    ) {
        let parent = if pseudo.is_some() {
            Some(element)
        } else {
            element.parent(crate::dom::TreeRelation::Formatting)
        };
        self.anchor_nodes.borrow_mut().insert(
            crate::style::anchor_values::key(element.id(), pseudo),
            crate::style::anchor_values::AnchorNode {
                multicol: style.multicol.is_multicol(),
                has_box: !matches!(
                    style.display,
                    crate::style::Display::None | crate::style::Display::Contents
                ),
                visible: style.visibility.is_visible(),
                parent: parent.map(|parent| crate::style::anchor_values::key(parent.id(), None)),
                order: self
                    .anchor_order
                    .get(&element.id())
                    .copied()
                    .unwrap_or_default()[match pseudo {
                    Some(crate::style::PseudoElement::Before) => 1,
                    Some(crate::style::PseudoElement::After) => 2,
                    _ => 0,
                }],
                position: style.position,
                contains_absolute: style.contains_absolute(),
                contains_fixed: style.contains_fixed(),
                names: style.anchor.names.clone(),
                scope: style.anchor.scope.clone(),
                default: style.anchor.default_reference(),
                page_defaults: style
                    .anchor
                    .pages
                    .iter()
                    .map(|(page, style)| (*page, style.anchor.default_reference()))
                    .collect(),
            },
        );
    }

    /// The anchor frames this styling consulted, which a layout must reproduce.
    pub(crate) fn anchor_frames(&self) -> &crate::style::anchor_values::AnchorFrames {
        &self.anchor_frames
    }
    pub(crate) fn anchor_frames_mut(&mut self) -> &mut crate::style::anchor_values::AnchorFrames {
        &mut self.anchor_frames
    }

    /// Defines the `@font-palette-values` palettes in the context of the root element's
    /// computed style `root`, which `var()` and relative units refer to (CSS Fonts 4 §9.2).
    pub(super) fn define_palettes(
        &mut self,
        root: &crate::style::Style,
        resolver: &dyn ResourceResolver,
    ) -> Result<(), StyleError> {
        let rules = std::mem::take(&mut self.palette_rules);
        let mut definitions = Vec::with_capacity(rules.len());
        {
            let relative = self.inherited(root, self.page_area(), resolver);
            for rule in &rules {
                match Self::palette_values(rule, root, &relative) {
                    Ok(definition) => definitions.push(definition),
                    Err(error) => self.ignore_rule(
                        rule.location.clone(),
                        &format!("@font-palette-values {}", rule.name),
                        &error,
                    ),
                }
            }
        }
        for (namespace, name, families, base, overrides) in definitions {
            self.fonts
                .define_palette(namespace, &name, families, base, overrides);
        }
        Ok(())
    }

    pub(crate) fn take_diagnostics(&mut self) -> Vec<StyleDiagnostic> {
        std::mem::take(self.diagnostics.get_mut())
    }

    /// Reports input rendered without, instead of failing.
    pub(crate) fn degrade(&self, degradation: crate::style::Degradation) {
        self.degradations.record(degradation);
    }

    /// Where computing styles and content reports what it renders without.
    pub(crate) fn degradations(&self) -> &std::rc::Rc<crate::style::Degradations> {
        &self.degradations
    }

    /// The degradations recorded since the last call.
    pub(crate) fn take_degradations(&self) -> Vec<crate::style::Degradation> {
        self.degradations.take()
    }

    pub(super) fn font_namespace(&self, scope: Option<crate::dom::TreeScopeId>) -> FontNamespace {
        scope
            .and_then(|scope| self.font_namespaces.get(&scope))
            .copied()
            .unwrap_or(FontNamespace::ROOT)
    }

    /// An `@font-palette-values` rule (CSS Fonts 4 §9.2). Invalid descriptors are ignored; a
    /// rule without `font-family` is invalid, and generic families match no face. `var()`
    /// substitutes the root element's custom properties; a reference that fails makes its
    /// descriptor invalid.
    fn palette_values(
        rule: &PaletteRule,
        root: &crate::style::Style,
        relative: &Relative<'_>,
    ) -> Result<PaletteDefinition, StyleError> {
        let mut families = None;
        let mut base = BasePalette::Index(0);
        let mut overrides = Vec::new();
        for d in &rule.declarations {
            let Some(descriptor) = Descriptor::find(PALETTE_AT_RULE, &d.name) else {
                continue;
            };
            let source = match variables::Value::with_substitutions(&d.value)? {
                None => d.value.clone(),
                Some(value) => match value
                    .substitute(&root.custom_properties, &root.inherited_custom_properties)?
                {
                    Some(expanded) => expanded.as_str().to_owned(),
                    None => continue,
                },
            };
            let Some(value) = declaration::descriptor(descriptor, &source)? else {
                continue;
            };
            match descriptor {
                Descriptor::FontPaletteValuesFontFamily => {
                    families = Some(
                        declaration::value(Property::FontFamily, &source, &mut 0)?
                            .map(|list| crate::style::values::families(&list))
                            .unwrap_or_default(),
                    );
                }
                Descriptor::FontPaletteValuesBasePalette => {
                    base = match value.unwrap() {
                        Component::Keyword(Keyword::Light) => BasePalette::Light,
                        Component::Keyword(Keyword::Dark) => BasePalette::Dark,
                        _ => BasePalette::Index(
                            u32::try_from(crate::style::values::integer(&value, relative)?)
                                .map_err(|_| StyleError::Invalid(source.clone()))?,
                        ),
                    };
                }
                Descriptor::FontPaletteValuesOverrideColors => {
                    if let Some(colors) = override_colors(&value, relative)? {
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
        Ok((rule.namespace, rule.name.clone(), named, base, overrides))
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
            let value = match declaration::descriptor(descriptor, &d.value).and_then(|value| {
                value
                    .map(|value| face_value(descriptor, &value, &relative))
                    .transpose()
            }) {
                Ok(Some(value)) => value,
                Ok(None) => continue,
                Err(error) => {
                    self.diagnostics.borrow_mut().push(StyleDiagnostic {
                        location: d.location.clone(),
                        kind: StyleDiagnosticKind::of(
                            &error,
                            StyleDiagnosticKind::InvalidDeclaration,
                        ),
                        context: format!("{FONT_FACE_AT_RULE} {}: {} ({error})", d.name, d.value),
                    });
                    continue;
                }
            };
            match value {
                FaceValue::Family(value) => family = Some(value),
                FaceValue::Weight(value) => weight = value,
                FaceValue::Width(value) => width = value,
                FaceValue::Style(value) => style = value,
                FaceValue::Sources(value) if !value.is_empty() => src = Some(value),
                FaceValue::Sources(_) | FaceValue::Initial => {}
                FaceValue::Features(value) => settings.features = value,
                FaceValue::Variations(value) => settings.variations = value,
                FaceValue::UnicodeRange(value) => settings.unicode_range = Some(value),
                FaceValue::SizeAdjust(value) => settings.size_adjust = value,
                FaceValue::AscentOverride(value) => settings.metric_overrides.ascent = value,
                FaceValue::DescentOverride(value) => settings.metric_overrides.descent = value,
                FaceValue::LineGapOverride(value) => settings.metric_overrides.line_gap = value,
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
