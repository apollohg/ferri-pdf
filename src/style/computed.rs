use crate::dom::Element;
use crate::fonts::{
    DEFAULT_FAMILY, FontAdjustments, FontFamily, FontId, FontSelection, FontSlant, FontStyle,
    FontSynthesis, FontWeight, FontWidth,
};
use crate::paint::{BoxDecoration, Color};
use crate::resources::ResourceResolver;
use crate::style::{
    BoxSizing, BreakBetween, BreakInside, DEFAULT_FONT_SIZE, DEFAULT_TAB_SIZE, Display,
    LengthPercentage, ListStylePosition, ListStyleType, Size, StyleError, Stylesheet, TextAlign,
    WhiteSpace,
    declaration::Specified,
    grammar::Property,
    parser::{declarations, failure},
    properties,
    units::{Faces, FontBasis, Line, Relative, Tree, Viewport},
};
use cssparser::{Parser, ParserInput};
use std::sync::Arc;

/// Initial `opacity`.
const OPAQUE: f32 = 1.0;
const CONTENTS_HIDDEN_HTML: &[&str] = &[
    "br", "wbr", "meter", "progress", "canvas", "embed", "object", "audio", "iframe", "img",
    "video", "frame", "frameset", "input", "textarea", "select",
];
const CONTENTS_SVG_CONTAINERS: &[&str] = &["a", "g", "svg", "switch", "textPath", "tspan", "use"];

fn initial_decoration() -> BoxDecoration {
    let mut decoration = BoxDecoration::default();
    for border in &mut decoration.borders {
        border.width = border.used_width();
    }
    decoration
}

fn adjust_box_generation(style: &mut Style, element: Element<'_>) {
    if style.display != Display::Contents {
        return;
    }
    let parent = element.parent(crate::dom::TreeRelation::Selector);
    if (element.namespace() == crate::dom::HTML_NAMESPACE
        && CONTENTS_HIDDEN_HTML.contains(&element.name()))
        || element.namespace() == crate::dom::MATHML_NAMESPACE
        || (element.namespace() == crate::dom::SVG_NAMESPACE
            && (!CONTENTS_SVG_CONTAINERS.contains(&element.name())
                || (element.name() == "svg"
                    && parent
                        .is_none_or(|parent| parent.namespace() != crate::dom::SVG_NAMESPACE))))
    {
        style.display = Display::None;
    }
}

#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub(crate) struct Style {
    #[serde(skip)]
    pub(super) custom_properties: crate::style::variables::Environment,
    #[serde(skip)]
    pub(super) inherited_custom_properties: crate::style::variables::Environment,
    #[serde(skip)]
    pub(super) computed_content: Arc<crate::style::generated::ComputedContent>,
    #[serde(skip)]
    pub(super) content_context: crate::style::generated::ContentContext,
    #[serde(skip)]
    pub(super) page_parent: Option<Arc<Style>>,
    #[serde(skip)]
    document_root: bool,
    #[serde(skip)]
    item_parent: bool,
    pub size: f32,
    pub line_height: crate::style::LineHeight,
    pub letter_spacing: f32,
    pub word_spacing: LengthPercentage,
    pub white_space: WhiteSpace,
    pub line_breaking: crate::style::LineBreaking,
    pub hyphens: crate::style::Hyphens,
    pub overflow_wrap: crate::style::OverflowWrap,
    pub text_transform: crate::style::TextTransform,
    pub language: String,
    pub quotes: crate::style::Quotes,
    pub counters: crate::style::CounterProperties,
    pub string_set: Arc<[crate::style::StringAssignment]>,
    pub tab_size: usize,
    pub list_style_image: crate::paint::BackgroundImage,
    pub list_style_type: ListStyleType,
    pub list_style_position: ListStylePosition,
    pub families: Vec<FontFamily>,
    #[serde(skip)]
    pub(super) font_namespace: crate::fonts::FontNamespace,
    #[serde(skip)]
    element_font_namespace: crate::fonts::FontNamespace,
    pub font_weight: FontWeight,
    #[expect(
        clippy::struct_field_names,
        reason = "the field names the CSS font-style property"
    )]
    pub font_style: FontStyle,
    pub font_variant_caps: crate::style::FontVariantCaps,
    pub font_width: FontWidth,
    pub font_slant: FontSlant,
    pub font_synthesis: FontSynthesis,
    pub font_features: crate::style::FontFeatures,
    pub font_adjustments: FontAdjustments,
    #[serde(skip)]
    font_candidates: Arc<[FontId]>,
    /// The root element's font, which root-relative units measure; `None` before the root.
    #[serde(skip)]
    pub(super) root_font: Option<RootFont>,
    pub margin: [Size; 4],
    pub padding: [LengthPercentage; 4],
    pub position: crate::style::Position,
    pub float: crate::style::Float,
    pub clear: crate::style::Clear,
    /// `None` is `auto`.
    pub z_index: Option<i32>,
    /// Inline-level before blockification; selects the inline static position.
    pub inline_level_origin: bool,
    pub transform: Vec<crate::style::TransformFunction>,
    /// Group opacity; below one the box is a stacking context composited as one layer.
    pub opacity: f32,
    pub mix_blend_mode: crate::paint::BlendMode,
    /// `isolation: isolate`.
    pub isolate: bool,
    pub visibility: crate::style::Visibility,
    pub transform_origin: [LengthPercentage; 3],
    /// `translate`, `rotate` and `scale`; `None` is `none`.
    pub translate: Option<[LengthPercentage; 3]>,
    pub rotate: Option<crate::style::Rotation>,
    pub scale: Option<[f32; 3]>,
    pub transform_box: crate::style::TransformBox,
    #[expect(
        clippy::struct_field_names,
        reason = "the field names the CSS transform-style property"
    )]
    pub transform_style: crate::style::TransformStyle,
    /// `None` is `none`.
    pub perspective: Option<f32>,
    pub perspective_origin: crate::paint::Position,
    /// `backface-visibility: hidden`.
    pub backface_hidden: bool,
    pub inset: [Size; 4],
    pub width: Size,
    pub min_width: Size,
    pub max_width: Size,
    pub height: Size,
    pub min_height: Size,
    pub max_height: Size,
    pub display: Display,
    pub box_sizing: BoxSizing,
    pub object_fit: crate::style::ObjectFit,
    pub object_position: crate::paint::Position,
    pub page: crate::style::Page,
    /// Horizontal and vertical overflow after axis pairing.
    pub overflow: [crate::style::Overflow; 2],
    pub rtl: bool,
    pub unicode_bidi: crate::style::UnicodeBidi,
    pub text_indent: crate::style::TextIndent,
    pub align: TextAlign,
    pub align_last: Option<TextAlign>,
    pub break_before: BreakBetween,
    pub break_after: BreakBetween,
    /// `break-inside`.
    pub keep: BreakInside,
    pub widows: usize,
    pub orphans: usize,
    /// The used `color`; `computed_color` keeps its colour space for `currentcolor`.
    pub color: Color,
    pub computed_color: crate::paint::AbsoluteColor,
    pub color_scheme: crate::style::color_scheme::ColorSchemes,
    pub decoration: BoxDecoration,
    pub clone_decoration: bool,
    pub text_decorations: crate::style::TextDecorations,
    /// Line-left and line-right `text-overflow`.
    pub text_overflow: [crate::style::TextOverflow; 2],
    pub clamp: crate::style::LineClamp,
    pub table: crate::style::TableStyle,
    pub flex: crate::style::FlexStyle,
    pub bookmark: crate::style::BookmarkStyle,
    pub grid: crate::style::GridStyle,
    pub multicol: crate::style::MulticolStyle,
    #[serde(skip)]
    pub(super) inert: crate::style::inert::InertValues,
}
impl Default for Style {
    fn default() -> Self {
        let custom_properties = crate::style::variables::Environment::default();
        Self {
            custom_properties: custom_properties.clone(),
            inherited_custom_properties: custom_properties,
            computed_content: crate::style::generated::INITIAL_CONTENT.clone(),
            content_context: Default::default(),
            page_parent: None,
            document_root: false,
            item_parent: false,
            size: DEFAULT_FONT_SIZE,
            line_height: crate::style::LineHeight::Normal,
            letter_spacing: 0.0,
            word_spacing: LengthPercentage::ZERO,
            white_space: WhiteSpace::NORMAL,
            line_breaking: crate::style::LineBreaking::default(),
            hyphens: crate::style::Hyphens::default(),
            overflow_wrap: crate::style::OverflowWrap::Normal,
            text_transform: crate::style::TextTransform::default(),
            language: String::new(),
            quotes: crate::style::Quotes::default(),
            counters: Default::default(),
            string_set: crate::style::string_values::empty(),
            tab_size: DEFAULT_TAB_SIZE,
            list_style_image: crate::paint::BackgroundImage::None,
            list_style_type: ListStyleType::default(),
            list_style_position: ListStylePosition::default(),
            families: vec![FontFamily::Generic(DEFAULT_FAMILY)],
            font_namespace: crate::fonts::FontNamespace::ROOT,
            element_font_namespace: crate::fonts::FontNamespace::ROOT,
            font_weight: FontWeight::NORMAL,
            font_style: FontStyle::Normal,
            font_variant_caps: crate::style::FontVariantCaps::Normal,
            font_width: FontWidth::NORMAL,
            font_slant: FontSlant::NORMAL,
            font_synthesis: FontSynthesis::ALL,
            font_features: crate::style::FontFeatures::default(),
            font_adjustments: FontAdjustments::default(),
            font_candidates: Arc::from([]),
            root_font: None,
            margin: [Size::ZERO; 4],
            padding: [LengthPercentage::ZERO; 4],
            position: crate::style::Position::Static,
            float: crate::style::Float::None,
            clear: crate::style::Clear::None,
            z_index: None,
            inline_level_origin: false,
            transform: Vec::new(),
            opacity: OPAQUE,
            mix_blend_mode: crate::paint::BlendMode::Normal,
            isolate: false,
            visibility: crate::style::Visibility::Visible,
            transform_origin: crate::style::transform::INITIAL_ORIGIN,
            translate: None,
            rotate: None,
            scale: None,
            transform_box: crate::style::TransformBox::default(),
            transform_style: crate::style::TransformStyle::default(),
            perspective: None,
            perspective_origin: crate::style::transform::INITIAL_PERSPECTIVE_ORIGIN,
            backface_hidden: false,
            inset: [const { Size::Auto }; 4],
            width: Size::Auto,
            min_width: Size::Auto,
            max_width: Size::Auto,
            height: Size::Auto,
            min_height: Size::Auto,
            max_height: Size::Auto,
            display: Display::Inline,
            box_sizing: BoxSizing::ContentBox,
            object_fit: crate::style::ObjectFit::default(),
            object_position: [const { crate::paint::PositionComponent::CENTER }; 2],
            page: crate::style::Page::default(),
            overflow: [crate::style::Overflow::Visible; 2],
            rtl: false,
            unicode_bidi: crate::style::UnicodeBidi::Normal,
            text_indent: crate::style::TextIndent::default(),
            align: TextAlign::Start,
            align_last: None,
            break_before: BreakBetween::Auto,
            break_after: BreakBetween::Auto,
            keep: BreakInside::Auto,
            widows: crate::style::DEFAULT_MIN_FRAGMENT_LINES,
            orphans: crate::style::DEFAULT_MIN_FRAGMENT_LINES,
            color: Color::BLACK,
            computed_color: crate::paint::AbsoluteColor::BLACK,
            color_scheme: Default::default(),
            decoration: initial_decoration(),
            clone_decoration: false,
            text_decorations: crate::style::TextDecorations::default(),
            text_overflow: [crate::style::TextOverflow::Clip; 2],
            clamp: crate::style::LineClamp::default(),
            table: crate::style::TableStyle::default(),
            flex: crate::style::FlexStyle::default(),
            bookmark: crate::style::BookmarkStyle::default(),
            grid: crate::style::GridStyle::default(),
            multicol: crate::style::MulticolStyle::default(),
            inert: crate::style::inert::InertValues::default(),
        }
    }
}
impl Style {
    /// Font-name environment owned by the element's tree scope.
    pub(crate) fn element_font_namespace(&self) -> crate::fonts::FontNamespace {
        self.element_font_namespace
    }
    /// Font-name environment of the declared `font-family`, where font-specific names resolve.
    pub(crate) fn font_namespace(&self) -> crate::fonts::FontNamespace {
        self.font_namespace
    }

    pub(crate) fn has_inside_marker(&self) -> bool {
        self.list_style_position == ListStylePosition::Inside
            && (self.list_style_type != ListStyleType::None
                || !matches!(self.list_style_image, crate::paint::BackgroundImage::None))
    }

    /// Inline padding, borders and unsplit margins retain an otherwise empty line.
    pub(crate) fn has_inline_edges(&self, continued: [bool; 2]) -> bool {
        const TOP: usize = 0;
        const RIGHT: usize = 1;
        const BOTTOM: usize = 2;
        const LEFT: usize = 3;
        let edge = |side: usize| {
            !self.padding[side].is_zero() || self.decoration.borders[side].used_width() > 0.0
        };
        let inline_sides = if self.rtl {
            [RIGHT, LEFT]
        } else {
            [LEFT, RIGHT]
        };
        inline_sides
            .into_iter()
            .zip(continued)
            .any(|(side, continued)| !continued && (edge(side) || !self.margin[side].is_zero()))
            || [TOP, BOTTOM].into_iter().any(edge)
    }

    #[must_use]
    pub(crate) fn anonymous(&self, display: Display) -> Self {
        let mut style = Self::default();
        properties::inherit(&mut style, self);
        style.display = display;
        style.custom_properties = self.custom_properties.clone();
        style.inherited_custom_properties = self.custom_properties.clone();
        style.language.clone_from(&self.language);
        style.font_candidates = self.font_candidates.clone();
        style.element_font_namespace = self.element_font_namespace;
        style.root_font.clone_from(&self.root_font);
        style.grid.initial_justify_items(&self.grid);
        style.text_decorations.propagated = if self.display == Display::Contents {
            self.text_decorations.propagated.clone()
        } else {
            self.text_decorations.propagated()
        };
        style.reset_current_colors();
        style
    }
    pub(super) fn normalize_display(&mut self, relative: &Relative<'_>) {
        if self.document_root && self.display == Display::Contents {
            self.display = Display::Block;
        }
        if self.content_context == crate::style::generated::ContentContext::Element
            && let Tree::Element(element) = relative.tree
        {
            adjust_box_generation(self, element);
        }
        if self.position.is_out_of_flow() {
            self.inline_level_origin = self.display.is_inline_level();
        }
        if self.document_root
            || self.item_parent
            || self.position.is_out_of_flow()
            || self.floating().is_some()
        {
            self.display = self.display.blockified();
        }
    }

    pub(super) fn independent_box(&self) -> bool {
        self.document_root || self.item_parent
    }

    pub(crate) fn floating(&self) -> Option<crate::style::FloatSide> {
        if self.document_root || self.item_parent || self.position.is_out_of_flow() {
            None
        } else {
            self.float.side(self.rtl)
        }
    }

    pub(crate) fn same_text_style(&self, other: &Self) -> bool {
        self.size == other.size
            && self.line_height == other.line_height
            && self.letter_spacing == other.letter_spacing
            && self.word_spacing == other.word_spacing
            && self.text_transform == other.text_transform
            && self.language == other.language
            && self.overflow_wrap == other.overflow_wrap
            && self.white_space == other.white_space
            && self.line_breaking == other.line_breaking
            && self.hyphens == other.hyphens
            && self.tab_size == other.tab_size
            && self.families == other.families
            && self.font_weight == other.font_weight
            && self.font_style == other.font_style
            && self.font_variant_caps == other.font_variant_caps
            && self.font_width == other.font_width
            && self.font_slant == other.font_slant
            && self.font_synthesis == other.font_synthesis
            && self.font_features == other.font_features
            && self.font_adjustments == other.font_adjustments
            && self.rtl == other.rtl
            && self.color == other.color
            && self.visibility == other.visibility
    }
    /// Initial `currentcolor` border and column rule colours take the element's colour.
    fn reset_current_colors(&mut self) {
        for border in &mut self.decoration.borders {
            border.color = self.color;
        }
        self.multicol.rule.color = self.color;
    }
    pub(super) fn backgrounds_mut(&mut self) -> &mut crate::paint::Backgrounds {
        Arc::make_mut(&mut self.decoration.backgrounds)
    }
    /// Opacity below one makes the box a stacking context painted as one group (CSS Color 4 §15).
    pub(crate) fn is_translucent(&self) -> bool {
        self.opacity < OPAQUE
    }
    /// Opacity, blending and `isolation: isolate` make the box a stacking context composited
    /// as one group (Compositing 1 §3.4).
    pub(crate) fn is_grouped(&self) -> bool {
        self.is_translucent()
            || self.mix_blend_mode != crate::paint::BlendMode::Normal
            || self.isolate
    }
    pub(crate) fn font_selection(&self) -> FontSelection {
        FontSelection {
            weight: self.font_weight,
            style: self.font_style,
            width: self.font_width,
            slant: self.font_slant,
            synthesis: self.font_synthesis,
        }
    }
    pub(crate) fn font_candidates(&self) -> &[FontId] {
        &self.font_candidates
    }
    pub(crate) fn named_font_candidates(&self, fonts: &crate::fonts::FontBook) -> Arc<[FontId]> {
        fonts.named_candidates_in_namespace(
            self.font_namespace,
            &self.families,
            self.font_selection(),
        )
    }
}

/// The computed font properties of the root element.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct RootFont {
    size: f32,
    line_height: crate::style::LineHeight,
    candidates: Arc<[FontId]>,
    selection: FontSelection,
    adjustments: FontAdjustments,
}

/// Properties applied before the rest: `font-size` resolves relative units against the parent's
/// font, the others select the element's own font, and `line-height` resolves against it.
pub(super) const FONT_SELECTION: [Property; 8] = [
    Property::FontFamily,
    Property::FontStyle,
    Property::FontWidth,
    Property::FontWeight,
    Property::FontSynthesisWeight,
    Property::FontSynthesisStyle,
    Property::FontSynthesisSmallCaps,
    Property::FontSynthesisPosition,
];

/// Properties applied after font matching that change the selected faces' metrics, which
/// font-relative units measure.
pub(super) const FONT_ADJUSTMENT: [Property; 3] = [
    Property::FontOpticalSizing,
    Property::FontVariationSettings,
    Property::FontSizeAdjust,
];

pub(super) use crate::style::cascade::cascade;
use crate::style::cascade::{Cascaded, DeclarationContext, Origin, Winners, insert};

impl Stylesheet {
    #[cfg(test)]
    pub(crate) fn computed(
        &self,
        element: Element<'_>,
        parent: &Style,
    ) -> Result<Style, StyleError> {
        self.computed_with_resolver(element, parent, &crate::resources::FileResolver::default())
    }

    #[cfg(test)]
    pub(crate) fn pseudo(
        &self,
        element: Element<'_>,
        parent: &Style,
        pseudo: crate::style::PseudoElement,
    ) -> Result<Option<(Style, crate::style::GeneratedContent)>, StyleError> {
        self.pseudo_with_resolver(
            element,
            parent,
            pseudo,
            &crate::resources::FileResolver::default(),
        )
    }

    /// `resolver` loads the fonts that font-relative units measure.
    pub(crate) fn computed_with_resolver(
        &self,
        element: Element<'_>,
        parent: &Style,
        resolver: &dyn ResourceResolver,
    ) -> Result<Style, StyleError> {
        let (style, cascaded) = self.prepare_element(element, parent)?;
        let (style, winners) = crate::style::resolution::resolve_values(
            self,
            style,
            parent,
            cascaded,
            resolver,
            &crate::style::resolution::ValueContext {
                tree: Tree::Element(element),
                viewport: self.page_area(),
                page_font: false,
            },
        )?;
        self.computed_values(element, parent, style, winners, resolver)
    }

    pub(super) fn computed_values(
        &self,
        element: Element<'_>,
        parent: &Style,
        mut style: Style,
        mut winners: Winners,
        resolver: &dyn ResourceResolver,
    ) -> Result<Style, StyleError> {
        let mut inherited = self.inherited(parent, self.page_area(), resolver);
        inherited.tree = Tree::Element(element);
        Self::display(&mut style, parent, &mut winners, &inherited)?;
        // The page context inherits the root's computed values even without a root box.
        if style.display == Display::None
            && element
                .parent(crate::dom::TreeRelation::Inheritance)
                .is_some()
        {
            return Ok(style);
        }
        let relative = self.text_basis(&mut style, parent, &mut winners, inherited, resolver)?;
        Self::apply_properties(style, parent, winners, &relative)
    }

    pub(super) fn prepare_element(
        &self,
        element: Element<'_>,
        parent: &Style,
    ) -> Result<(Style, Cascaded), StyleError> {
        let mut style = parent.anonymous(Display::Inline);
        style.element_font_namespace = self.font_namespace(Some(element.tree_scope()));
        style.document_root = element.is_in_document_tree()
            && element
                .parent(crate::dom::TreeRelation::Inheritance)
                .is_none();
        style.item_parent = parent.display.is_item_container()
            || (parent.display == Display::Contents && parent.item_parent);
        crate::style::table_values::structure(&mut style, element);
        element.language().clone_into(&mut style.language);
        if parent.root_font.is_none() {
            style.quotes = style.quotes.inherited(&style.language);
        }
        let (mut cascaded, mut order) = self.rule_winners(element, None);
        for default in crate::style::user_agent::declarations(element, &self.document_base)? {
            let mut declaration = default.declaration;
            declaration.font_namespace = self.font_namespace(Some(element.tree_scope()));
            insert(
                &mut cascaded,
                &declaration,
                DeclarationContext {
                    encapsulation_order: if declaration.origin == Origin::AuthorHint {
                        element.encapsulation_order()
                    } else {
                        crate::dom::DOCUMENT_CONTEXT_ORDER
                    },
                    origin: declaration.origin,
                    attached: false,
                    layer: declaration.layer,
                    specificity: if declaration.origin == Origin::UserAgent {
                        default.specificity
                    } else {
                        0
                    },
                    scope_proximity: None,
                    order: 0,
                },
            );
        }
        if element.namespace() == crate::dom::HTML_NAMESPACE && element.name() == "th" {
            cascaded.header_alignment(self.font_namespace(Some(element.tree_scope())));
        }
        if let Some(inline) = element.attr("style") {
            let mut i = ParserInput::new(inline);
            let raw = declarations(
                &mut Parser::new(&mut i),
                &self.document_base,
                &self.diagnostics,
            )
            .map_err(failure)?;
            let parsed = crate::style::stylesheet::parsed(
                raw,
                &self.document_base,
                &self.diagnostics,
                self.font_namespace(Some(element.tree_scope())),
            )?;
            for d in &parsed {
                order += 1;
                insert(
                    &mut cascaded,
                    d,
                    DeclarationContext {
                        encapsulation_order: element.encapsulation_order(),
                        origin: Origin::Author,
                        attached: true,
                        layer: crate::style::layers::LayerId::Unlayered,
                        specificity: 0,
                        scope_proximity: None,
                        order,
                    },
                );
            }
        }
        Ok((style, cascaded))
    }

    /// The page area, which is the viewport in paged media (CSS Page 3 §3).
    pub(super) fn page_area(&self) -> [f32; 2] {
        let area = self.page_content_box();
        [area.width, area.height]
    }

    pub(super) fn page_geometry(&self) -> crate::style::PageGeometry {
        crate::style::PageGeometry {
            paper: self.page,
            extent: self.page_extent,
            area: self.page_content_size,
            margin: self.margin,
            padding: self.page_padding,
            border: self.page_border,
        }
    }

    /// The first page area's rectangle, including the page box's padding and border insets.
    pub(crate) fn page_content_box(&self) -> crate::paint::Rect {
        self.page_geometry().content_box()
    }

    fn rule_winners(
        &self,
        element: Element<'_>,
        pseudo: Option<crate::style::PseudoElement>,
    ) -> (Cascaded, usize) {
        let mut winners = Cascaded::default();
        let mut order = 0;
        let mut scopes = crate::style::scope::ScopeCache::new(element);
        for &index in &self.rule_indices[crate::style::selector::target_index(pseudo)] {
            let rule = &self.rules[index];
            if let Some((specificity, scope_proximity)) = rule
                .selectors
                .iter()
                .filter_map(|selector| {
                    scopes
                        .roots(rule.scope, &self.scopes, selector)
                        .filter_map(|(root, proximity)| {
                            selector
                                .matches(&element, pseudo, root, rule.tree_context)
                                .map(|specificity| (specificity, proximity))
                        })
                        .max_by_key(|(specificity, proximity)| {
                            (
                                *specificity,
                                std::cmp::Reverse(proximity.unwrap_or(usize::MAX)),
                            )
                        })
                })
                .max_by_key(|(specificity, proximity)| {
                    (
                        *specificity,
                        std::cmp::Reverse(proximity.unwrap_or(usize::MAX)),
                    )
                })
            {
                for declaration in &rule.declarations {
                    order += 1;
                    insert(
                        &mut winners,
                        declaration,
                        DeclarationContext {
                            encapsulation_order: rule.encapsulation_order,
                            origin: declaration.origin,
                            attached: false,
                            layer: declaration.layer,
                            specificity,
                            scope_proximity,
                            order,
                        },
                    );
                }
            }
        }
        (winners, order)
    }

    pub(crate) fn pseudo_with_resolver(
        &self,
        element: Element<'_>,
        parent: &Style,
        pseudo: crate::style::PseudoElement,
        resolver: &dyn ResourceResolver,
    ) -> Result<Option<(Style, crate::style::GeneratedContent)>, StyleError> {
        let (style, winners) = self.prepare_pseudo(element, parent, pseudo)?;
        let (mut style, mut winners) = crate::style::resolution::resolve_values(
            self,
            style,
            parent,
            winners,
            resolver,
            &crate::style::resolution::ValueContext {
                tree: Tree::Element(element),
                viewport: self.page_area(),
                page_font: false,
            },
        )?;
        let mut inherited = self.inherited(parent, self.page_area(), resolver);
        inherited.tree = Tree::Element(element);
        let Some(content) = Self::generated(&mut winners, &mut style, parent, &inherited)? else {
            return Ok(None);
        };
        Self::display(&mut style, parent, &mut winners, &inherited)?;
        if style.display == Display::None {
            return Ok(None);
        }
        let relative = self.text_basis(&mut style, parent, &mut winners, inherited, resolver)?;
        Ok(Some((
            Self::apply_properties(style, parent, winners, &relative)?,
            content,
        )))
    }

    pub(super) fn prepare_pseudo(
        &self,
        element: Element<'_>,
        parent: &Style,
        pseudo: crate::style::PseudoElement,
    ) -> Result<(Style, Cascaded), StyleError> {
        let (mut winners, _) = self.rule_winners(element, Some(pseudo));
        for declaration in crate::style::user_agent::pseudo_declarations(element, pseudo)? {
            insert(
                &mut winners,
                &declaration,
                DeclarationContext {
                    encapsulation_order: crate::dom::DOCUMENT_CONTEXT_ORDER,
                    origin: declaration.origin,
                    attached: false,
                    layer: declaration.layer,
                    specificity: 0,
                    scope_proximity: None,
                    order: 0,
                },
            );
        }
        let mut style = parent.anonymous(Display::Inline);
        style.element_font_namespace = self.font_namespace(Some(element.tree_scope()));
        style.content_context = crate::style::generated::ContentContext::BeforeAfter;
        style.item_parent = parent.display.is_item_container()
            || (parent.display == Display::Contents && parent.item_parent);
        Ok((style, winners))
    }

    pub(super) fn generated(
        winners: &mut Winners,
        style: &mut Style,
        parent: &Style,
        relative: &Relative<'_>,
    ) -> Result<Option<crate::style::GeneratedContent>, StyleError> {
        let value = winners
            .remove(&Property::Content)
            .unwrap_or(Specified::Wide(
                crate::style::wide_keywords::WideKeyword::Initial,
            ));
        properties::apply(Property::Content, &value, style, parent, relative)?;
        Ok(match style.computed_content.as_ref() {
            crate::style::generated::ComputedContent::Items { content, .. } => {
                Some(content.clone())
            }
            _ => None,
        })
    }

    fn display(
        style: &mut Style,
        parent: &Style,
        winners: &mut Winners,
        relative: &Relative<'_>,
    ) -> Result<(), StyleError> {
        if let Some(value) = winners.remove(&Property::Position) {
            properties::apply(Property::Position, &value, style, parent, relative)?;
        }
        if let Some(value) = winners.remove(&Property::Float) {
            properties::apply(Property::Float, &value, style, parent, relative)?;
        }
        if let Some(value) = winners.remove(&Property::Display) {
            properties::apply(Property::Display, &value, style, parent, relative)?;
        } else {
            style.normalize_display(relative);
        }
        Ok(())
    }

    /// The font measured with the sheet's faces.
    pub(super) fn basis<'a>(
        &'a self,
        size: f32,
        line_height: crate::style::LineHeight,
        candidates: &Arc<[FontId]>,
        selection: FontSelection,
        adjustments: &FontAdjustments,
        resolver: &'a dyn ResourceResolver,
    ) -> FontBasis<'a> {
        let faces = || {
            Faces::book(
                &self.fonts,
                resolver,
                candidates.clone(),
                selection,
                adjustments.clone(),
                size,
            )
        };
        FontBasis::Known {
            size,
            faces: faces(),
            line: Line::Known {
                height: line_height,
                size,
                faces: faces(),
            },
        }
    }

    pub(super) fn initial_relative(&self) -> Relative<'static> {
        Relative::with_initial_font(self.environment.relative.initial_font.clone())
    }

    /// `Canvas` in the root's used colour scheme, the canvas surface colour (CSS Color
    /// Adjustment 1 §2.2).
    pub(crate) fn canvas_surface(&self, root: &Style) -> Color {
        crate::style::colors::canvas(self.used_color_scheme(&root.color_scheme))
    }

    /// The used colour scheme of computed `color-scheme` `schemes` (CSS Color Adjustment 1
    /// §2.1); print preferences never override it.
    pub(crate) fn used_color_scheme(
        &self,
        schemes: &crate::style::color_scheme::ColorSchemes,
    ) -> crate::style::ColorScheme {
        schemes.used(
            self.environment.preferences.color_scheme,
            &self.environment.page_color_schemes,
        )
    }

    /// What a child of `parent` inherits: the parent's font and the root's, or initial values
    /// for the root element, which has no parent style.
    pub(super) fn inherited<'a>(
        &'a self,
        parent: &Style,
        viewport: [f32; 2],
        resolver: &'a dyn ResourceResolver,
    ) -> Relative<'a> {
        match &parent.root_font {
            None => Relative {
                viewport: Viewport::Known(viewport),
                paper: self.paper,
                tree: Tree::Root,
                ..self.initial_relative()
            },
            Some(root) => Relative {
                font: self.basis(
                    parent.size,
                    parent.line_height,
                    &parent.font_candidates,
                    parent.font_selection(),
                    &parent.font_adjustments,
                    resolver,
                ),
                root: self.basis(
                    root.size,
                    root.line_height,
                    &root.candidates,
                    root.selection,
                    &root.adjustments,
                    resolver,
                ),
                viewport: Viewport::Known(viewport),
                paper: self.paper,
                tree: Tree::Root,
                grammar_only: false,
                work: Default::default(),
                initial_page_font_size: false,
                initial_font: self.environment.relative.initial_font.clone(),
                color_scheme: self.used_color_scheme(&parent.color_scheme),
            },
        }
    }

    /// Computes the font properties and returns what the element's other relative lengths
    /// resolve against: its own font, with `lh` measuring the parent's line height until
    /// `line-height` computes (CSS Values 4 §6.1.1), and `color-scheme` then colour first for
    /// `light-dark()`, system colours and `currentcolor`.
    pub(super) fn text_basis<'a>(
        &'a self,
        style: &mut Style,
        parent: &Style,
        winners: &mut Winners,
        mut inherited: Relative<'a>,
        resolver: &'a dyn ResourceResolver,
    ) -> Result<Relative<'a>, StyleError> {
        let root = parent.root_font.is_none();
        if let Some(value) = winners.remove(&Property::ColorScheme) {
            properties::apply(Property::ColorScheme, &value, style, parent, &inherited)?;
        }
        let color_scheme = self.used_color_scheme(&style.color_scheme);
        for property in [Property::FontSize].iter().chain(&FONT_SELECTION) {
            if let Some(value) = winners.remove(property) {
                properties::apply(*property, &value, style, parent, &inherited)?;
            }
            inherited.initial_page_font_size = false;
        }
        style.font_candidates = self.fonts.candidates_in_namespace(
            style.font_namespace,
            &style.families,
            style.font_selection(),
        );
        for property in &FONT_ADJUSTMENT {
            let Some(value) = winners.remove(property) else {
                continue;
            };
            // `from-font` measures the first available face as the other adjustments select it.
            let measured = Relative {
                font: self.basis(
                    style.size,
                    crate::style::LineHeight::Normal,
                    &style.font_candidates,
                    style.font_selection(),
                    &FontAdjustments {
                        size_adjust: None,
                        ..style.font_adjustments.clone()
                    },
                    resolver,
                ),
                ..inherited.clone()
            };
            properties::apply(*property, &value, style, parent, &measured)?;
        }
        let Relative {
            font: inherited_font,
            root: inherited_root,
            viewport,
            paper,
            tree,
            work,
            ..
        } = inherited;
        let own = |line| FontBasis::Known {
            size: style.size,
            faces: Faces::book(
                &self.fonts,
                resolver,
                style.font_candidates.clone(),
                style.font_selection(),
                style.font_adjustments.clone(),
                style.size,
            ),
            line,
        };
        // On the root, root-relative units measure the root's font except that `rlh` keeps
        // the initial line height until `line-height` computes.
        let line_basis = Relative {
            font: own(inherited_font.into_line()),
            root: if root {
                own(inherited_root.into_line())
            } else {
                inherited_root
            },
            viewport,
            paper,
            tree,
            grammar_only: false,
            work: work.clone(),
            initial_page_font_size: false,
            initial_font: self.environment.relative.initial_font.clone(),
            color_scheme,
        };
        if let Some(value) = winners.remove(&Property::LineHeight) {
            properties::apply(Property::LineHeight, &value, style, parent, &line_basis)?;
        }
        if root {
            self.capture_root_font(style);
        }
        let relative = Relative {
            font: self.basis(
                style.size,
                style.line_height,
                &style.font_candidates,
                style.font_selection(),
                &style.font_adjustments,
                resolver,
            ),
            root: if root {
                self.basis(
                    style.size,
                    style.line_height,
                    &style.font_candidates,
                    style.font_selection(),
                    &style.font_adjustments,
                    resolver,
                )
            } else {
                line_basis.root
            },
            viewport,
            paper,
            tree,
            grammar_only: false,
            work: work.clone(),
            initial_page_font_size: false,
            initial_font: self.environment.relative.initial_font.clone(),
            color_scheme,
        };
        // The root's initial `canvastext` follows its own colour scheme.
        let initial = Specified::Wide(crate::style::wide_keywords::WideKeyword::Initial);
        if let Some(value) = winners
            .remove(&Property::Color)
            .or_else(|| root.then_some(initial))
        {
            properties::apply(Property::Color, &value, style, parent, &relative)?;
        }
        style.decoration = initial_decoration();
        style.reset_current_colors();
        style.text_decorations.declared.color = style.color;
        Ok(relative)
    }

    pub(super) fn capture_root_font(&self, style: &mut Style) {
        if style.font_candidates.is_empty() {
            style.font_candidates = self.fonts.candidates_in_namespace(
                style.font_namespace,
                &style.families,
                style.font_selection(),
            );
        }
        style.root_font = Some(RootFont {
            size: style.size,
            line_height: style.line_height,
            candidates: style.font_candidates.clone(),
            selection: style.font_selection(),
            adjustments: style.font_adjustments.clone(),
        });
    }

    pub(super) fn apply_properties(
        mut style: Style,
        parent: &Style,
        mut winners: Winners,
        relative: &Relative<'_>,
    ) -> Result<Style, StyleError> {
        for property in Property::BorderStyle.longhands() {
            if let Some(value) = winners.remove(property) {
                properties::apply(*property, &value, &mut style, parent, relative)?;
            }
        }
        let initial = Specified::Wide(crate::style::wide_keywords::WideKeyword::Initial);
        for property in Property::BorderWidth.longhands() {
            let value = winners.remove(property);
            properties::apply(
                *property,
                value.as_ref().unwrap_or(&initial),
                &mut style,
                parent,
                relative,
            )?;
        }
        crate::style::overflow::compute(&mut style, parent, &mut winners, relative)?;
        for (property, value) in &winners {
            properties::apply(*property, value, &mut style, parent, relative)?;
        }
        if style.content_context == crate::style::generated::ContentContext::Element
            && matches!(
                style.computed_content.as_ref(),
                crate::style::generated::ComputedContent::Items { .. }
            )
        {
            return Err(StyleError::Unsupported(
                "generated content replacement on elements".into(),
            ));
        }
        // Out-of-flow boxes and the contents of atomic inlines receive no decorations.
        if style.position.is_out_of_flow()
            || style.floating().is_some()
            || (style.display.is_atomic_inline() && !parent.display.is_item_container())
        {
            style.text_decorations.propagated.clear();
        }
        if !style.size.is_finite()
            || style.size < 0.0
            || style
                .line_height
                .resolve(style.size)
                .is_some_and(|height| !height.is_finite() || height < 0.0)
        {
            return Err("negative or non-finite text dimensions".into());
        }
        Ok(style)
    }
}
