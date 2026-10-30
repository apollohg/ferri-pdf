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
    decoration.outline.width = decoration.outline.computed_width();
    decoration
}

fn adjust_box_generation(style: &mut Style, element: Element<'_>) {
    let element_box = style.content_context == crate::style::generated::ContentContext::Element;
    // MathML Core §4.1 resolves math/table display before CSS Display's box fixups.
    if matches!(style.display, Display::InlineMath | Display::BlockMath) {
        let inline = style.display == Display::InlineMath;
        if element_box && element.namespace() == crate::dom::MATHML_NAMESPACE {
            match element.name() {
                "mtable" => {
                    style.display = if inline {
                        Display::InlineTable
                    } else {
                        Display::Table
                    }
                }
                "mtr" => style.display = Display::TableRow,
                "mtd" => style.display = Display::TableCell,
                _ => {}
            }
        } else {
            style.display = if inline {
                Display::Inline
            } else {
                Display::Block
            };
        }
    }
    if !element_box || style.display != Display::Contents {
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

/// The element or pseudo-element a style was computed for, and the origin of the style it
/// inherited from, which `::first-line` recomputes inheritance through (CSS Pseudo-Elements 4
/// §2.1.3). Provenance is no computed value: it never makes two styles differ.
#[derive(Clone, Default)]
pub(crate) struct StyleOrigin(Option<Arc<OriginNode>>);
#[derive(Debug)]
pub(crate) struct OriginNode {
    pub element: crate::dom::ElementId,
    pub pseudo: Option<crate::style::PseudoElement>,
    pub parent: StyleOrigin,
}
impl StyleOrigin {
    fn of(
        element: Element<'_>,
        pseudo: Option<crate::style::PseudoElement>,
        parent: &Style,
    ) -> Self {
        Self(Some(Arc::new(OriginNode {
            element: element.id(),
            pseudo,
            parent: parent.origin.clone(),
        })))
    }
    pub(crate) fn node(&self) -> Option<&Arc<OriginNode>> {
        self.0.as_ref()
    }
    /// Whether both styles were computed for the same box, or both for none.
    pub(crate) fn same(&self, other: &Self) -> bool {
        match (&self.0, &other.0) {
            (Some(node), Some(other)) => Arc::ptr_eq(node, other),
            (None, None) => true,
            _ => false,
        }
    }
}
impl PartialEq for StyleOrigin {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}
impl std::fmt::Debug for StyleOrigin {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("StyleOrigin")
    }
}

#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub(crate) struct Style {
    pub zoom: f32,
    pub effective_zoom: f32,
    #[serde(skip)]
    pub(super) canonical: Option<Arc<Style>>,
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
    pub(crate) origin: StyleOrigin,
    #[serde(skip)]
    pub(super) document_root: bool,
    #[serde(skip)]
    item_parent: bool,
    #[serde(skip)]
    math_parent: bool,
    /// The parent inlinifies its in-flow children (CSS Display 3 §2.7, §5).
    #[serde(skip)]
    inlinifying_parent: bool,
    pub math: crate::style::math_values::Math,
    pub size: f32,
    pub line_height: crate::style::LineHeight,
    /// `line-height-step`; zero leaves line box heights unrounded.
    pub line_height_step: f32,
    pub letter_spacing: f32,
    pub word_spacing: LengthPercentage,
    pub text_spacing: crate::style::TextSpacing,
    pub white_space: WhiteSpace,
    pub line_breaking: crate::style::LineBreaking,
    pub hyphens: crate::style::Hyphens,
    pub overflow_wrap: crate::style::OverflowWrap,
    pub text_transform: crate::style::TextTransform,
    pub language: String,
    pub quotes: crate::style::Quotes,
    pub counters: crate::style::CounterProperties,
    pub string_set: Arc<[crate::style::StringAssignment]>,
    pub tab_size: crate::style::TabSize,
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
    /// The running element name of `position: running()` (CSS GCPM 3 §1.2.1).
    pub running: Option<Arc<str>>,
    pub float: crate::style::Float,
    pub footnote_display: crate::style::FootnoteDisplay,
    pub footnote_policy: crate::style::FootnotePolicy,
    pub clear: crate::style::Clear,
    /// CSS Shapes 1 §6: the float area's shape, the margin that expands it, and the alpha
    /// threshold that extracts it from an image.
    pub shape_outside: Arc<crate::paint::ShapeOutside>,
    pub shape_margin: LengthPercentage,
    pub shape_image_threshold: f32,
    /// `None` is `auto`.
    pub z_index: Option<i32>,
    /// `clip`; `None` is `auto`.
    pub clip: Option<crate::style::position::ClipRect>,
    /// Inline-level before blockification; selects the inline static position.
    pub inline_level_origin: bool,
    pub transform: Vec<crate::style::TransformFunction>,
    pub motion: Arc<crate::style::motion::Motion>,
    /// Group opacity; below one the box is a stacking context composited as one layer.
    pub opacity: f32,
    pub mix_blend_mode: crate::paint::BlendMode,
    /// `filter`, applied in order; empty is `none`.
    pub filter: Arc<Vec<crate::paint::FilterFunction>>,
    /// `backdrop-filter`, applied in order to the backdrop; empty is `none`.
    pub backdrop_filter: Arc<Vec<crate::paint::FilterFunction>>,
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
    pub image_orientation: crate::style::ImageOrientation,
    pub image_resolution: crate::style::ImageResolution,
    pub(crate) object_view_box: Option<crate::paint::BasicShape>,
    pub page: crate::style::Page,
    /// Horizontal and vertical overflow after axis pairing.
    pub overflow: [crate::style::Overflow; 2],
    pub overflow_clip_margin: [crate::style::OverflowClipMargin; 4],
    pub scroll_markers: crate::style::ScrollMarkers,
    /// `scroll-margin` (CSS Scroll Snap 1 §6.1): how far the box's scroll snap area, and the
    /// area a scroll into view aligns, extends past its border box.
    pub scroll_margin: [f32; 4],
    /// `scroll-padding` (CSS Scroll Snap 1 §6.2): the insets of a scroll container's
    /// optimal viewing region from its scrollport; `None` is `auto`.
    pub scroll_padding: [Option<LengthPercentage>; 4],
    pub rtl: bool,
    pub writing_mode: crate::style::logical::WritingMode,
    pub(crate) principal_flow: Option<crate::style::logical::Flow>,
    pub(crate) layout_writing_mode: crate::style::logical::WritingMode,
    pub(super) parent_writing_mode: crate::style::logical::WritingMode,
    pub text_orientation: crate::style::logical::TextOrientation,
    pub text_combine: crate::style::logical::TextCombine,
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
    /// `accent-color`; `None` is `auto`.
    pub accent_color: Option<crate::paint::AbsoluteColor>,
    pub widget: crate::style::WidgetStyle,
    pub color_scheme: crate::style::color_scheme::ColorSchemes,
    pub decoration: BoxDecoration,
    #[serde(skip)]
    pub(super) shadows: crate::style::shadow_values::Shadows,
    pub clone_decoration: bool,
    pub margin_trim: crate::style::MarginTrim,
    pub text_decorations: crate::style::TextDecorations,
    pub text_paint: crate::style::TextPaint,
    /// Line-left and line-right `text-overflow`.
    pub text_overflow: [crate::style::TextOverflow; 2],
    pub clamp: crate::style::LineClamp,
    pub table: crate::style::TableStyle,
    pub baseline: crate::style::BaselineAlignment,
    pub text_box: crate::style::TextBox,
    pub initial_letter: Option<crate::style::InitialLetter>,
    pub initial_letter_align: crate::style::InitialLetterAlign,
    pub ruby: crate::style::RubyStyle,
    pub flex: crate::style::FlexStyle,
    pub bookmark: crate::style::BookmarkStyle,
    pub grid: crate::style::GridStyle,
    pub multicol: crate::style::MulticolStyle,
    pub will_change: crate::style::will_change::WillChange,
    /// `view-transition-name` other than `none`.
    pub view_transition_named: bool,
    pub aspect_ratio: crate::style::AspectRatio,
    pub containment: crate::style::ContainmentStyle,
    #[serde(skip)]
    pub anchor: crate::style::anchor_values::AnchorStyle,
    /// `visibility: force-hidden` (CSS Anchor Positioning 1 §6.6), which descendants inherit
    /// and cannot override.
    #[serde(skip)]
    pub force_hidden: bool,
    /// The nearest query container among the element's inclusive ancestors.
    #[serde(skip)]
    pub(super) containers: crate::style::QueryContainers,
    #[serde(skip)]
    pub(super) inert: crate::style::inert::InertValues,
    #[serde(skip)]
    pub(crate) svg: Arc<crate::style::svg_values::SvgStyle>,
}
impl Default for Style {
    fn default() -> Self {
        let custom_properties = crate::style::variables::Environment::default();
        Self {
            zoom: crate::style::zoom::INITIAL_ZOOM,
            effective_zoom: crate::style::zoom::INITIAL_ZOOM,
            canonical: None,
            custom_properties: custom_properties.clone(),
            inherited_custom_properties: custom_properties,
            computed_content: crate::style::generated::INITIAL_CONTENT.clone(),
            content_context: Default::default(),
            page_parent: None,
            origin: StyleOrigin::default(),
            document_root: false,
            item_parent: false,
            math_parent: false,
            inlinifying_parent: false,
            size: DEFAULT_FONT_SIZE,
            line_height: crate::style::LineHeight::Normal,
            line_height_step: 0.0,
            letter_spacing: 0.0,
            word_spacing: LengthPercentage::ZERO,
            text_spacing: crate::style::TextSpacing::default(),
            white_space: WhiteSpace::NORMAL,
            line_breaking: crate::style::LineBreaking::default(),
            hyphens: crate::style::Hyphens::default(),
            overflow_wrap: crate::style::OverflowWrap::Normal,
            text_transform: crate::style::TextTransform::default(),
            math: crate::style::math_values::Math::default(),
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
            running: None,
            float: crate::style::Float::None,
            footnote_display: crate::style::FootnoteDisplay::Block,
            footnote_policy: crate::style::FootnotePolicy::Auto,
            clear: crate::style::Clear::None,
            shape_outside: Arc::default(),
            shape_margin: LengthPercentage::ZERO,
            shape_image_threshold: 0.0,
            z_index: None,
            clip: None,
            inline_level_origin: false,
            transform: Vec::new(),
            motion: Arc::default(),
            opacity: OPAQUE,
            mix_blend_mode: crate::paint::BlendMode::Normal,
            filter: Arc::default(),
            backdrop_filter: Arc::default(),
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
            image_orientation: crate::style::ImageOrientation::default(),
            image_resolution: crate::style::ImageResolution::default(),
            object_view_box: None,
            page: crate::style::Page::default(),
            overflow: [crate::style::Overflow::Visible; 2],
            overflow_clip_margin: [crate::style::OverflowClipMargin::default(); 4],
            scroll_markers: crate::style::ScrollMarkers::default(),
            scroll_margin: [0.0; 4],
            scroll_padding: [None, None, None, None],
            rtl: false,
            writing_mode: crate::style::logical::WritingMode::default(),
            principal_flow: None,
            layout_writing_mode: crate::style::logical::WritingMode::default(),
            parent_writing_mode: crate::style::logical::WritingMode::default(),
            text_orientation: crate::style::logical::TextOrientation::default(),
            text_combine: crate::style::logical::TextCombine::default(),
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
            accent_color: None,
            widget: crate::style::WidgetStyle::default(),
            color_scheme: Default::default(),
            decoration: initial_decoration(),
            shadows: crate::style::shadow_values::Shadows::default(),
            clone_decoration: false,
            margin_trim: crate::style::MarginTrim::None,
            text_decorations: crate::style::TextDecorations::default(),
            text_paint: crate::style::TextPaint::default(),
            text_overflow: [crate::style::TextOverflow::Clip; 2],
            clamp: crate::style::LineClamp::default(),
            table: crate::style::TableStyle::default(),
            baseline: crate::style::BaselineAlignment::default(),
            text_box: crate::style::TextBox::default(),
            initial_letter: None,
            initial_letter_align: crate::style::InitialLetterAlign::default(),
            ruby: crate::style::RubyStyle::default(),
            flex: crate::style::FlexStyle::default(),
            bookmark: crate::style::BookmarkStyle::default(),
            grid: crate::style::GridStyle::default(),
            multicol: crate::style::MulticolStyle::default(),
            will_change: Default::default(),
            view_transition_named: false,
            aspect_ratio: crate::style::AspectRatio::default(),
            containment: crate::style::ContainmentStyle::default(),
            anchor: crate::style::anchor_values::AnchorStyle::default(),
            force_hidden: false,
            containers: crate::style::QueryContainers::default(),
            inert: crate::style::inert::InertValues::default(),
            svg: Arc::default(),
        }
    }
}
impl Style {
    /// `base`, the style of a box holding a first formatted line, as the line's contents inherit
    /// it through `::first-line` style `line` (CSS Pseudo-Elements 4 §2.1.3): standard inherited
    /// properties come from `line`, except `writing-mode`, `direction` and `text-orientation`;
    /// all others, and custom properties, from `base`.
    pub(crate) fn through_first_line(base: &Self, line: &Self) -> Self {
        if base.canonical.is_some() || line.canonical.is_some() {
            return Self::through_first_line(base.canonical(), line.canonical()).used();
        }
        let mut style = base.clone();
        properties::inherit(&mut style, line);
        // Copying leaves `currentcolor` text paint resolved for `base`; `line` resolved its own.
        style.text_paint.clone_from(&line.text_paint);
        style.rtl = base.rtl;
        style.writing_mode = base.writing_mode;
        style.text_orientation = base.text_orientation;
        style
    }
    /// Font-name environment owned by the element's tree scope.
    pub(crate) fn element_font_namespace(&self) -> crate::fonts::FontNamespace {
        self.element_font_namespace
    }
    /// Font-name environment of the declared `font-family`, where font-specific names resolve.
    pub(crate) fn font_namespace(&self) -> crate::fonts::FontNamespace {
        self.font_namespace
    }

    /// The content list replacing an element's contents (CSS Content 3 §1), if any.
    pub(crate) fn content_list(&self) -> Option<&crate::style::GeneratedContent> {
        self.computed_content.items()
    }
    /// The image a `<content-replacement>` fills the element with, making it a replaced
    /// element.
    pub(crate) fn content_replacement(&self) -> Option<&crate::paint::BackgroundImage> {
        self.content_list()
            .and_then(crate::style::GeneratedContent::replacement)
    }

    pub(crate) fn has_inside_marker(&self) -> bool {
        self.list_style_position == ListStylePosition::Inside
            && (self.list_style_type != ListStyleType::None
                || self.list_style_image.is_displayable())
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
        let mut style = Self {
            effective_zoom: self.effective_zoom,
            ..Self::default()
        };
        let used = self.canonical.is_some();
        if used {
            style = style.used();
        }
        properties::inherit(&mut style, self);
        style.display = display;
        style.parent_writing_mode = self.writing_mode;
        style.custom_properties = self.custom_properties.clone();
        style.inherited_custom_properties = self.custom_properties.clone();
        style.language.clone_from(&self.language);
        style.font_candidates = self.font_candidates.clone();
        style.element_font_namespace = self.element_font_namespace;
        style.root_font.clone_from(&self.root_font);
        style.containers = self.containers.clone();
        style.force_hidden = self.force_hidden;
        style.grid.initial_justify_items(&self.grid);
        style.text_decorations.propagated = if self.display == Display::Contents {
            self.text_decorations.propagated.clone()
        } else {
            self.text_decorations.propagated(if used {
                crate::style::INITIAL_ZOOM
            } else {
                self.effective_zoom
            })
        };
        style.reset_current_colors();
        // Layout-only adjustments inherit here; CSS computation keeps its original basis.
        style.canonical = self
            .canonical
            .as_ref()
            .map(|canonical| Arc::new(canonical.anonymous(style.display.clone())));
        style
    }
    pub(super) fn normalize_display(&mut self, relative: &Relative<'_>) {
        if self.writing_mode != self.parent_writing_mode {
            self.display = match self.display {
                Display::Inline => Display::InlineBlock,
                Display::Block => Display::FlowRoot,
                Display::ListItem => Display::FlowRootListItem,
                _ => self.display.clone(),
            };
        }
        if self.document_root && self.display == Display::Contents {
            self.display = Display::Block;
        }
        if let Tree::Element(element) = relative.tree {
            adjust_box_generation(self, element);
        }
        if self.position.is_out_of_flow() {
            self.inline_level_origin = self.display.is_inline_level();
        }
        // Running elements leave the flow as absolutely positioned boxes do (CSS GCPM 3
        // §1.2.1) and are laid out as block boxes in page-margin boxes.
        if self.footnote() && self.footnote_display == crate::style::FootnoteDisplay::Inline {
            // CSS GCPM 3 §2.3: the footnote is placed in the footnote area as an inline
            // element, of its inner display type.
            self.display = match self.display.blockified() {
                Display::Block => Display::Inline,
                Display::ListItem => Display::InlineListItem,
                display => display.inlinified(),
            };
        } else if self.document_root
            || self.item_parent
            || self.position.is_out_of_flow()
            || self.position == crate::style::Position::Running
            || self.floating().is_some()
            || self.footnote()
        {
            self.display = self.display.blockified();
        } else if self.inlinifying_parent {
            self.display = self.display.inlinified();
        }
    }

    /// A run-in flow box inlinifies its contents, and an inlinified inline box its own
    /// in-flow children; `display: contents` passes its parent's on.
    fn inlinifies_children(&self) -> bool {
        match &self.display {
            Display::RunIn(inner) => inner.is_inline_box() || inner.is_ruby(),
            // CSS Ruby 1 §2.2: ruby boxes hold only inline-level content.
            display if display.is_ruby() || *display == Display::BlockRuby => true,
            display => {
                (display.is_inline_box() || *display == Display::Contents)
                    && self.inlinifying_parent
            }
        }
    }

    /// Its parent holds only inline-level content, having inlinified it.
    pub(crate) fn inlinified(&self) -> bool {
        self.inlinifying_parent
    }

    pub(super) fn independent_box(&self) -> bool {
        self.document_root || self.item_parent
    }

    pub(crate) fn used_writing_mode(&self) -> crate::style::logical::WritingMode {
        self.principal_flow
            .map_or(self.writing_mode, |flow| flow.writing_mode)
    }
    pub(crate) fn vertical_typographic_mode(&self) -> bool {
        matches!(
            self.used_writing_mode(),
            crate::style::logical::WritingMode::VerticalRl
                | crate::style::logical::WritingMode::VerticalLr
        ) && self.text_orientation != crate::style::logical::TextOrientation::Sideways
    }
    pub(crate) fn vertical_font_units(&self) -> [bool; 2] {
        use crate::style::logical::{TextOrientation, WritingMode};
        if !matches!(
            self.writing_mode,
            WritingMode::VerticalRl | WritingMode::VerticalLr
        ) {
            return [false; 2];
        }
        [
            self.text_orientation == TextOrientation::Upright,
            self.text_orientation != TextOrientation::Sideways,
        ]
    }
    pub(crate) fn used_rtl(&self) -> bool {
        self.principal_flow.map_or(self.rtl, |flow| flow.rtl)
            && !(matches!(
                self.used_writing_mode(),
                crate::style::logical::WritingMode::VerticalRl
                    | crate::style::logical::WritingMode::VerticalLr
            ) && self.text_orientation == crate::style::logical::TextOrientation::Upright)
    }

    /// Whether `float: footnote` moves the element to the footnote area: floating applies
    /// to it (CSS GCPM 3 §2.2).
    pub(crate) fn footnote(&self) -> bool {
        self.float == crate::style::Float::Footnote
            && !(self.document_root
                || self.item_parent
                || self.math_parent
                || self.position.leaves_flow())
    }

    pub(crate) fn floating(&self) -> Option<crate::style::FloatSide> {
        if self.document_root
            || self.item_parent
            || self.math_parent
            || self.position.is_out_of_flow()
        {
            None
        } else {
            self.float.side(self.used_rtl())
        }
    }

    /// The used `justify-self` of a block-level box in block layout; it does not apply to
    /// floats or absolutely positioned boxes (CSS Align 3 §6.1.1).
    pub(crate) fn block_justify_self(
        &self,
    ) -> Option<(crate::style::Alignment, crate::style::OverflowPosition)> {
        (self.floating().is_none() && !self.position.is_out_of_flow())
            .then(|| self.grid.used_justify_self(self.grid.parent_justify_items))
    }

    pub(crate) fn same_text_style(&self, other: &Self) -> bool {
        self.writing_mode == other.writing_mode
            && self.text_orientation == other.text_orientation
            && self.text_combine == other.text_combine
            && self.size == other.size
            && self.line_height == other.line_height
            && self.letter_spacing == other.letter_spacing
            && self.word_spacing == other.word_spacing
            && self.text_spacing == other.text_spacing
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
            && self.text_paint == other.text_paint
            && self.visibility == other.visibility
    }
    /// Initial `currentcolor` border, outline and column rule colours take the element's colour.
    fn reset_current_colors(&mut self) {
        for border in &mut self.decoration.borders {
            border.color = self.color;
        }
        self.decoration.outline.color = self.color;
        self.multicol.rule.color = self.color;
    }
    pub(super) fn backgrounds_mut(&mut self) -> &mut crate::paint::Backgrounds {
        Arc::make_mut(&mut self.decoration.backgrounds)
    }
    pub(super) fn masks_mut(&mut self) -> &mut crate::paint::Masks {
        Arc::make_mut(&mut self.decoration.masks)
    }
    pub(super) fn border_image(&self) -> &crate::paint::BorderImage {
        &self.decoration.border_image
    }
    pub(super) fn border_image_mut(&mut self) -> &mut crate::paint::BorderImage {
        Arc::make_mut(&mut self.decoration.border_image)
    }
    pub(super) fn mask_border(&self) -> &crate::paint::BorderImage {
        &self.decoration.masks.border
    }
    pub(super) fn mask_border_mut(&mut self) -> &mut crate::paint::BorderImage {
        Arc::make_mut(&mut self.masks_mut().border)
    }
    /// Opacity below one makes the box a stacking context painted as one group (CSS Color 4 §15).
    pub(crate) fn is_translucent(&self) -> bool {
        self.opacity < OPAQUE
    }
    /// Opacity, blending, `isolation: isolate`, filters and backdrop filters make the box a
    /// stacking context composited as one group (Compositing 1 §3.4, Filter Effects 1 §5,
    /// Filter Effects 2 §2.1).
    pub(crate) fn is_grouped(&self) -> bool {
        self.is_translucent()
            || self.mix_blend_mode != crate::paint::BlendMode::Normal
            || self.isolate
            || self.is_filtered()
            || self.has_backdrop_filter()
            || self.decoration.masks.is_masked()
    }
    /// A `filter` other than `none`.
    pub(crate) fn is_filtered(&self) -> bool {
        !self.filter.is_empty()
    }
    /// A `backdrop-filter` other than `none`.
    pub(crate) fn has_backdrop_filter(&self) -> bool {
        !self.backdrop_filter.is_empty()
    }
    /// The box forms a Backdrop Root (Filter Effects 2 §3), which bounds the backdrop its
    /// descendants' backdrop filters read; the document root is the page's.
    pub(crate) fn is_backdrop_root(&self) -> bool {
        self.is_translucent()
            || self.mix_blend_mode != crate::paint::BlendMode::Normal
            || self.is_filtered()
            || self.has_backdrop_filter()
            || self.decoration.masks.is_masked()
            || self.will_change.backdrop_root
            || self.view_transition_named
    }
    /// A stacking context from a `will-change` hint (CSS Will Change 1 §3.1), a view
    /// transition name, which also makes the box a backdrop root (CSS View Transitions 1 §3),
    /// sticky positioning (CSS Position 3 §2.2) or layout and paint containment (CSS
    /// Containment 2 §3.2, §3.4).
    pub(crate) fn forced_stacking_context(&self) -> bool {
        self.will_change.stacking_context
            || self.view_transition_named
            || self.position == crate::style::Position::Sticky
            || self.contains_positioned()
    }
    /// `z-index` where it applies; `will-change: z-index` makes `auto` a stacking context.
    pub(crate) fn stacking_z_index(&self) -> Option<i32> {
        self.z_index.or(self.will_change.z_index.then_some(0))
    }
    /// Generates the containing block of absolutely positioned descendants (CSS Position 3
    /// §2.1).
    pub(crate) fn contains_absolute(&self) -> bool {
        self.position != crate::style::Position::Static
            || self.contains_fixed()
            || self.will_change.absolute_containing_block
    }
    /// Generates the containing block of absolutely positioned descendants as a non-atomic
    /// inline box, which transforms (CSS Transforms 1 §3) and layout and paint containment
    /// (CSS Containment 2 §3.2, §3.4) do not apply to.
    pub(crate) fn inline_contains_absolute(&self) -> bool {
        self.position != crate::style::Position::Static
            || self.inline_contains_fixed()
            || self.will_change.absolute_containing_block
    }
    /// Generates the containing block of fixed positioned descendants as a non-atomic inline
    /// box: only through `will-change` (CSS Will Change 1 §3.1).
    pub(crate) fn inline_contains_fixed(&self) -> bool {
        self.will_change.fixed_containing_block
    }
    /// Generates the containing block of fixed positioned descendants (CSS Transforms 1 §3,
    /// CSS Transforms 2 §5, CSS Containment 2 §3.2 and §3.4); a filter or backdrop filter
    /// does unless on the document root (Filter Effects 1 §5, Filter Effects 2 §2.1).
    pub(crate) fn contains_fixed(&self) -> bool {
        self.has_transform_effects()
            || self.will_change.fixed_containing_block
            || self.contains_positioned()
            || ((self.is_filtered() || self.has_backdrop_filter()) && !self.document_root)
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
    writing_mode: crate::style::logical::WritingMode,
    vertical_units: [bool; 2],
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
        self.computed_element(element, parent, resolver, false)
    }

    /// Every computed value of an element, even one whose box `display: none` suppresses: SVG
    /// content there can still be referenced and rendered elsewhere (SVG 2 §3.2).
    pub(crate) fn computed_without_box(
        &self,
        element: Element<'_>,
        parent: &Style,
        resolver: &dyn ResourceResolver,
    ) -> Result<Style, StyleError> {
        self.computed_element(element, parent, resolver, true)
    }

    fn computed_element(
        &self,
        element: Element<'_>,
        parent: &Style,
        resolver: &dyn ResourceResolver,
        without_box: bool,
    ) -> Result<Style, StyleError> {
        let parent = parent.canonical();
        let (seed, cascaded) = self.prepare_element(element, parent, resolver)?;
        let original = cascaded.clone();
        let (style, winners) = crate::style::resolution::resolve_values(
            self,
            seed.clone(),
            parent,
            cascaded,
            resolver,
            &crate::style::resolution::ValueContext {
                tree: Tree::Element(element),
                viewport: self.page_area(),
                page_font: false,
            },
        )?;
        let style = self.finish_values(
            (element, None, None),
            parent,
            style,
            winners,
            resolver,
            without_box,
        )?;
        self.position_options(
            (element, None, None),
            parent,
            &seed,
            &original,
            style,
            resolver,
        )
    }

    fn position_options(
        &self,
        target: (
            Element<'_>,
            Option<crate::style::PseudoElement>,
            Option<usize>,
        ),
        parent: &Style,
        seed: &Style,
        original: &Cascaded,
        mut style: Style,
        resolver: &dyn ResourceResolver,
    ) -> Result<Style, StyleError> {
        let (element, pseudo, page) = target;
        if style.position.is_out_of_flow() && !style.anchor.fallbacks.is_empty() {
            style = self.fallback_options(target, parent, seed, original, &style, resolver)?;
        }
        if page.is_none() && style.position.is_out_of_flow() && style.anchor.anchors() {
            style.anchor.pages =
                self.page_variants((element, pseudo), parent, seed, original, resolver)?;
        }
        style.origin = StyleOrigin::of(element, pseudo, parent);
        for option in Arc::make_mut(&mut style.anchor.options) {
            option.origin = style.origin.clone();
        }
        if page.is_none() {
            self.record_anchor(element, pseudo, &style);
        }
        Ok(style)
    }

    /// The style of `target` with each of its `position-try-fallbacks` options (CSS Anchor
    /// Positioning 1 §5), as the option its anchor frame chose. Kept apart from
    /// `position_options`, which every element's computation runs.
    fn fallback_options(
        &self,
        (element, pseudo, page): (
            Element<'_>,
            Option<crate::style::PseudoElement>,
            Option<usize>,
        ),
        parent: &Style,
        seed: &Style,
        original: &Cascaded,
        style: &Style,
        resolver: &dyn ResourceResolver,
    ) -> Result<Style, StyleError> {
        use crate::style::anchor_values::{Tactic, TryOption};
        use crate::style::cascade::Origin;
        use crate::style::declaration::{Declared, PropertyDeclaration};
        let protected = original.important_properties();
        let mut options = vec![style.clone()];
        let frame = self
            .anchor_frames
            .frame(crate::style::anchor_values::key(element.id(), pseudo), page);
        let containing = frame.map_or_else(crate::style::logical::Flow::default, |frame| {
            frame.containing_flow
        });
        for option in style.anchor.fallbacks.iter() {
            let mut cascade = original.clone();
            let (declarations, tactic) = match option {
                TryOption::Area(area) => (
                    vec![PropertyDeclaration {
                        declared: Declared::Longhand(
                            Property::PositionArea,
                            Specified::Value(area.clone()),
                        ),
                        location: None,
                        important: false,
                        origin: Origin::PositionFallback,
                        layer: crate::style::layers::LayerId::Unlayered,
                        font_namespace: style.element_font_namespace(),
                        namespaces: None,
                    }],
                    Tactic::IDENTITY,
                ),
                TryOption::Rule { rule, tactic } => {
                    let declarations = if let Some(rule) = rule {
                        let Some(declarations) = self.position_try(rule) else {
                            continue;
                        };
                        declarations.to_vec()
                    } else {
                        Vec::new()
                    };
                    (declarations, Tactic::of(tactic, containing))
                }
            };
            let declarations: Vec<_> = declarations
                .into_iter()
                .map(|mut declaration| {
                    declaration.origin = Origin::PositionFallback;
                    declaration
                })
                .collect();
            cascade.extend(crate::style::cascade::cascade(&declarations));
            let (mut option_seed, mut winners) = crate::style::resolution::resolve_values(
                self,
                seed.clone(),
                parent,
                cascade.clone(),
                resolver,
                &crate::style::resolution::ValueContext {
                    tree: Tree::Element(element),
                    viewport: self.page_area(),
                    page_font: false,
                },
            )?;
            if tactic != Tactic::IDENTITY {
                let transformed: Vec<_> = Property::ALL
                    .iter()
                    .copied()
                    .filter(|property| {
                        crate::style::anchor_values::accepted(*property)
                            && !crate::style::logical::is_flow_relative(*property)
                    })
                    .map(|property| PropertyDeclaration {
                        declared: Declared::Longhand(
                            property,
                            winners
                                .get(&tactic.source_property(property))
                                .cloned()
                                .unwrap_or(Specified::Wide(
                                    crate::style::wide_keywords::WideKeyword::Unset,
                                )),
                        ),
                        location: None,
                        important: false,
                        origin: Origin::PositionFallback,
                        layer: crate::style::layers::LayerId::Unlayered,
                        font_namespace: option_seed
                            .anchor
                            .namespaces
                            .get(&tactic.source_property(property))
                            .copied()
                            .unwrap_or_else(|| style.element_font_namespace()),
                        namespaces: None,
                    })
                    .collect();
                cascade.extend(crate::style::cascade::cascade(&transformed));
                (option_seed, winners) = crate::style::resolution::resolve_values(
                    self,
                    seed.clone(),
                    parent,
                    cascade,
                    resolver,
                    &crate::style::resolution::ValueContext {
                        tree: Tree::Element(element),
                        viewport: self.page_area(),
                        page_font: false,
                    },
                )?;
            }
            option_seed.anchor.tactic = tactic;
            option_seed.anchor.protected = protected.clone();
            let mut option_style = self.finish_values(
                (element, pseudo, page),
                parent,
                option_seed,
                winners,
                resolver,
                false,
            )?;
            crate::style::anchor_values::transform_computed(&mut option_style, tactic, containing);
            option_style.anchor.option = options.len();
            options.push(option_style);
        }
        let chosen = frame.map_or(0, |frame| frame.option);
        let mut style = options.get(chosen).unwrap_or(&options[0]).clone();
        style.anchor.options = Arc::from(options);
        Ok(style)
    }

    /// The style of `target` on each page its anchor frames lie on.
    fn page_variants(
        &self,
        (element, pseudo): (Element<'_>, Option<crate::style::PseudoElement>),
        parent: &Style,
        seed: &Style,
        original: &Cascaded,
        resolver: &dyn ResourceResolver,
    ) -> Result<Arc<[(usize, Style)]>, StyleError> {
        let mut variants = Vec::new();
        for page in self
            .anchor_frames
            .pages(crate::style::anchor_values::key(element.id(), pseudo))
        {
            let (page_seed, winners) = crate::style::resolution::resolve_values(
                self,
                seed.clone(),
                parent,
                original.clone(),
                resolver,
                &crate::style::resolution::ValueContext {
                    tree: Tree::Element(element),
                    viewport: self.page_area(),
                    page_font: false,
                },
            )?;
            let page_style = self.finish_values(
                (element, pseudo, Some(page)),
                parent,
                page_seed,
                winners,
                resolver,
                false,
            )?;
            let page_style = self.position_options(
                (element, pseudo, Some(page)),
                parent,
                seed,
                original,
                page_style,
                resolver,
            )?;
            variants.push((page, page_style));
        }
        Ok(Arc::from(variants))
    }

    pub(super) fn computed_values(
        &self,
        element: Element<'_>,
        parent: &Style,
        style: Style,
        winners: Winners,
        resolver: &dyn ResourceResolver,
    ) -> Result<Style, StyleError> {
        let mut style = self.finish_values(
            (element, None, None),
            parent,
            style,
            winners,
            resolver,
            false,
        )?;
        style.origin = StyleOrigin::of(element, None, parent);
        self.record_anchor(element, None, &style);
        Ok(style)
    }

    fn finish_values(
        &self,
        target: (
            Element<'_>,
            Option<crate::style::PseudoElement>,
            Option<usize>,
        ),
        parent: &Style,
        mut style: Style,
        mut winners: Winners,
        resolver: &dyn ResourceResolver,
        without_box: bool,
    ) -> Result<Style, StyleError> {
        let (element, pseudo, page) = target;
        let mut inherited = self.inherited(parent, self.page_area(), resolver);
        inherited.tree = Tree::Element(element);
        inherited.anchoring = Some(crate::style::anchor_values::Anchoring {
            page,
            scope: None,
            frames: &self.anchor_frames,
            key: crate::style::anchor_values::key(element.id(), pseudo),
            tactic: style.anchor.tactic,
        });
        Self::display(&mut style, parent, &mut winners, &inherited)?;
        // The page context inherits the root's computed values even without a root box.
        if !without_box
            && style.display == Display::None
            && element
                .parent(crate::dom::TreeRelation::Inheritance)
                .is_some()
        {
            return Ok(style);
        }
        let relative = self.text_basis(&mut style, parent, &mut winners, inherited, resolver)?;
        let mut style = Self::apply_properties(style, parent, winners, &relative)?;
        if pseudo.is_none() {
            self.record_container(element.id(), &mut style);
        }
        style.anchor.frame = self
            .anchor_frames
            .frame(crate::style::anchor_values::key(element.id(), pseudo), page)
            .cloned()
            .map(Arc::new);
        if page.is_some()
            || !style.position.is_out_of_flow()
            || !style.anchor.anchors()
            || self
                .anchor_frames
                .pages(crate::style::anchor_values::key(element.id(), pseudo))
                .len()
                <= 1
        {
            crate::style::anchor_values::finish(
                &mut style,
                self.anchor_frames
                    .frame(crate::style::anchor_values::key(element.id(), pseudo), page),
            );
        }
        Ok(style)
    }

    pub(super) fn prepare_element(
        &self,
        element: Element<'_>,
        parent: &Style,
        resolver: &dyn ResourceResolver,
    ) -> Result<(Style, Cascaded), StyleError> {
        let mut style = parent.anonymous(Display::Inline);
        // The root's parent carries no registrations; its descendants inherit them.
        for environment in [
            &mut style.custom_properties,
            &mut style.inherited_custom_properties,
        ] {
            *environment =
                crate::style::variables::Variables::registered(environment, &self.registrations);
        }
        style.grid.parent_justify_items = parent.grid.default_justify();
        style.element_font_namespace = self.font_namespace(Some(element.tree_scope()));
        style.document_root = element.is_in_document_tree()
            && element
                .parent(crate::dom::TreeRelation::Inheritance)
                .is_none();
        style.item_parent = parent.display.is_item_container()
            || (parent.display == Display::Contents && parent.item_parent);
        style.math_parent = matches!(parent.display, Display::InlineMath | Display::BlockMath);
        style.inlinifying_parent = parent.inlinifies_children();
        crate::style::table_values::structure(&mut style, element);
        element.language().clone_into(&mut style.language);
        if parent.root_font.is_none() {
            style.quotes = style.quotes.inherited(&style.language);
        }
        let (mut cascaded, mut order) =
            self.rule_winners(element, None, &parent.containers, resolver)?;
        for default in
            crate::style::user_agent::declarations(element, &self.document_base, &self.environment)?
        {
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
        if element.is_widget() {
            style.widget.author_decorated = cascaded
                .author_cascades(&crate::style::widget_values::NATIVE_APPEARANCE_DISABLERS)?;
        }
        if self.becomes_base_select(element, parent, &style, &cascaded, resolver)? {
            return self.prepare_element(element, parent, resolver);
        }
        Ok((style, cascaded))
    }

    /// Whether `element` is a `select` that its cascaded `appearance` first gives base
    /// appearance (CSS UI 4 §7.2), which it records so that the user-agent styles of base
    /// appearance (HTML §15.5.16) match it and its contents.
    fn becomes_base_select(
        &self,
        element: Element<'_>,
        parent: &Style,
        seed: &Style,
        cascaded: &Cascaded,
        resolver: &dyn ResourceResolver,
    ) -> Result<bool, StyleError> {
        if element.namespace() != crate::dom::HTML_NAMESPACE
            || element.name() != "select"
            || element.is_base_select()
        {
            return Ok(false);
        }
        let (_, winners) = crate::style::resolution::resolve_values(
            self,
            seed.clone(),
            parent,
            cascaded.clone(),
            resolver,
            &crate::style::resolution::ValueContext {
                tree: Tree::Element(element),
                viewport: self.page_area(),
                page_font: false,
            },
        )?;
        // `appearance` is a keyword that does not inherit, so only `inherit` reads another
        // element's value.
        let appearance = match winners.get(&Property::Appearance) {
            Some(Specified::Value(value)) => crate::style::Appearance::from_component(value)?,
            Some(Specified::Wide(crate::style::wide_keywords::WideKeyword::Inherit)) => {
                parent.widget.appearance
            }
            _ => crate::style::Appearance::default(),
        };
        let base = matches!(
            appearance,
            crate::style::Appearance::Base | crate::style::Appearance::BaseSelect
        );
        if base {
            element.set_base_select();
        }
        Ok(base)
    }

    /// The page area, which is the viewport in paged media (CSS Page 3 §3).
    pub(crate) fn page_area(&self) -> [f32; 2] {
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
            intrinsic: [None, None],
        }
    }

    /// The first page area's rectangle, including the page box's padding and border insets.
    pub(crate) fn page_content_box(&self) -> crate::paint::Rect {
        self.page_geometry().content_box()
    }

    /// `ancestors` are the element's inclusive-ancestor query containers; a pseudo-element's
    /// include its originating element (CSS Conditional 5 §5).
    fn rule_winners(
        &self,
        element: Element<'_>,
        pseudo: Option<crate::style::PseudoElement>,
        ancestors: &crate::style::QueryContainers,
        resolver: &dyn ResourceResolver,
    ) -> Result<(Cascaded, usize), StyleError> {
        let mut winners = Cascaded::default();
        let mut order = 0;
        let mut scopes = crate::style::scope::ScopeCache::new(element);
        let mut containers = std::collections::HashMap::new();
        // SVG 2 §5.5.3: a use-element shadow tree's style sheets are its referenced element's,
        // in the shadow tree's own context.
        let instance = element.stylesheet_scope() != element.tree_scope();
        for &index in &self.rule_indices[crate::style::selector::target_index(pseudo)] {
            let rule = &self.rules[index];
            let encapsulation_order = if instance {
                element.encapsulation_order()
            } else {
                rule.encapsulation_order
            };
            if let Some(container) = rule.container
                && !self.container_rule_matches(
                    container,
                    element,
                    ancestors,
                    resolver,
                    &mut containers,
                )?
            {
                continue;
            }
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
                            encapsulation_order,
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
        Ok((winners, order))
    }

    pub(crate) fn pseudo_with_resolver(
        &self,
        element: Element<'_>,
        parent: &Style,
        pseudo: crate::style::PseudoElement,
        resolver: &dyn ResourceResolver,
    ) -> Result<Option<(Style, crate::style::GeneratedContent)>, StyleError> {
        let parent = parent.canonical();
        let (seed, original) = self.prepare_pseudo(element, parent, pseudo, resolver)?;
        let (mut style, mut winners) = crate::style::resolution::resolve_values(
            self,
            seed.clone(),
            parent,
            original.clone(),
            resolver,
            &crate::style::resolution::ValueContext {
                tree: Tree::Element(element),
                viewport: self.page_area(),
                page_font: false,
            },
        )?;
        let mut inherited = self.inherited(parent, self.page_area(), resolver);
        inherited.tree = Tree::Element(element);
        inherited.anchoring = Some(crate::style::anchor_values::Anchoring {
            page: None,
            scope: None,
            frames: &self.anchor_frames,
            key: crate::style::anchor_values::key(element.id(), Some(pseudo)),
            tactic: crate::style::anchor_values::Tactic::IDENTITY,
        });
        let declared = winners.get(&Property::Content).cloned();
        let Some(content) = Self::generated(&mut winners, &mut style, parent, &inherited)? else {
            return Ok(None);
        };
        Self::display(&mut style, parent, &mut winners, &inherited)?;
        if style.display == Display::None {
            return Ok(None);
        }
        let relative = self.text_basis(&mut style, parent, &mut winners, inherited, resolver)?;
        let mut style = Self::apply_properties(style, parent, winners, &relative)?;
        let content = Self::content_images(declared, content, &mut style, parent, &relative)?;
        style.anchor.frame = self
            .anchor_frames
            .frame(
                crate::style::anchor_values::key(element.id(), Some(pseudo)),
                None,
            )
            .cloned()
            .map(Arc::new);
        let frame = style.anchor.frame.clone();
        if self
            .anchor_frames
            .pages(crate::style::anchor_values::key(element.id(), Some(pseudo)))
            .len()
            <= 1
        {
            crate::style::anchor_values::finish(&mut style, frame.as_deref());
        }
        let style = self.position_options(
            (element, Some(pseudo), None),
            parent,
            &seed,
            &original,
            style,
            resolver,
        )?;
        Ok(Some((style, content)))
    }

    /// The `::marker` of list item `element` (CSS Lists 3 §3.1): its computed style and the
    /// `content` replacing the list item's marker image or string; `None` where `content:
    /// none` suppresses the marker box.
    pub(crate) fn marker_with_resolver(
        &self,
        element: Element<'_>,
        parent: &Style,
        resolver: &dyn ResourceResolver,
    ) -> Result<Option<(Style, Option<crate::style::GeneratedContent>)>, StyleError> {
        let style = self.typographic_pseudo_with_resolver(
            element,
            parent,
            crate::style::PseudoElement::Marker,
            resolver,
        )?;
        Ok(match style.computed_content.as_ref() {
            crate::style::generated::ComputedContent::Contents => Some((style, None)),
            crate::style::generated::ComputedContent::None => None,
            crate::style::generated::ComputedContent::Items { content, .. } => {
                let content = content.clone();
                Some((style, Some(content)))
            }
        })
    }

    /// The computed style of a pseudo-element that holds its originating element's content or
    /// marker rather than `content` it generates: `::marker`, `::first-line`, `::first-letter`
    /// and its sub-pseudo-elements. Every property has a computed value; the formatting
    /// structure applies only those its pseudo-element takes.
    pub(crate) fn typographic_pseudo_with_resolver(
        &self,
        element: Element<'_>,
        parent: &Style,
        pseudo: crate::style::PseudoElement,
        resolver: &dyn ResourceResolver,
    ) -> Result<Style, StyleError> {
        let parent = parent.canonical();
        let (style, winners) = self.prepare_pseudo(element, parent, pseudo, resolver)?;
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
        let declared = winners.get(&Property::Content).cloned();
        let content = Self::generated(&mut winners, &mut style, parent, &inherited)?;
        Self::display(&mut style, parent, &mut winners, &inherited)?;
        let relative = self.text_basis(&mut style, parent, &mut winners, inherited, resolver)?;
        let mut style = Self::apply_properties(style, parent, winners, &relative)?;
        if let Some(content) = content {
            Self::content_images(declared, content, &mut style, parent, &relative)?;
        }
        style.origin = StyleOrigin::of(element, Some(pseudo), parent);
        Ok(style)
    }

    /// The style of an element-backed pseudo-element of `element`, to which `content` does
    /// not apply (CSS Pseudo 4 §4).
    pub(crate) fn element_backed_with_resolver(
        &self,
        element: Element<'_>,
        parent: &Style,
        pseudo: crate::style::PseudoElement,
        resolver: &dyn ResourceResolver,
    ) -> Result<Style, StyleError> {
        let parent = parent.canonical();
        let (style, winners) = self.prepare_pseudo(element, parent, pseudo, resolver)?;
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
        winners.remove(&Property::Content);
        let mut inherited = self.inherited(parent, self.page_area(), resolver);
        inherited.tree = Tree::Element(element);
        Self::display(&mut style, parent, &mut winners, &inherited)?;
        let relative = self.text_basis(&mut style, parent, &mut winners, inherited, resolver)?;
        Self::apply_properties(style, parent, winners, &relative)
    }

    pub(super) fn prepare_pseudo(
        &self,
        element: Element<'_>,
        parent: &Style,
        pseudo: crate::style::PseudoElement,
        resolver: &dyn ResourceResolver,
    ) -> Result<(Style, Cascaded), StyleError> {
        let (winners, _) =
            self.rule_winners(element, Some(pseudo), &parent.containers, resolver)?;
        let mut style = parent.anonymous(Display::Inline);
        style.grid.parent_justify_items = parent.grid.default_justify();
        style.element_font_namespace = self.font_namespace(Some(element.tree_scope()));
        style.content_context = if pseudo == crate::style::PseudoElement::Marker {
            crate::style::generated::ContentContext::Marker
        } else {
            crate::style::generated::ContentContext::BeforeAfter
        };
        style.item_parent = parent.display.is_item_container()
            || (parent.display == Display::Contents && parent.item_parent);
        style.math_parent = matches!(parent.display, Display::InlineMath | Display::BlockMath);
        style.inlinifying_parent = parent.inlinifies_children();
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

    /// A pseudo-element computes `content` before its other properties, against its parent's
    /// font and colour; the `<image>`s in it compute again against its own.
    fn content_images(
        declared: Option<Specified>,
        content: crate::style::GeneratedContent,
        style: &mut Style,
        parent: &Style,
        relative: &Relative<'_>,
    ) -> Result<crate::style::GeneratedContent, StyleError> {
        let Some(declared) = declared.filter(|_| content.has_images()) else {
            return Ok(content);
        };
        properties::apply(Property::Content, &declared, style, parent, relative)?;
        Ok(style.computed_content.items().cloned().unwrap_or(content))
    }

    fn display(
        style: &mut Style,
        parent: &Style,
        winners: &mut Winners,
        relative: &Relative<'_>,
    ) -> Result<(), StyleError> {
        if let Some(value) = winners.remove(&Property::WritingMode) {
            properties::apply(Property::WritingMode, &value, style, parent, relative)?;
        }
        if let Some(value) = winners.remove(&Property::TextOrientation) {
            properties::apply(Property::TextOrientation, &value, style, parent, relative)?;
        }
        if let Some(value) = winners.remove(&Property::Position) {
            properties::apply(Property::Position, &value, style, parent, relative)?;
        }
        if let Some(value) = winners.remove(&Property::Float) {
            properties::apply(Property::Float, &value, style, parent, relative)?;
        }
        if let Some(value) = winners.remove(&Property::FootnoteDisplay) {
            properties::apply(Property::FootnoteDisplay, &value, style, parent, relative)?;
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
            vertical: [false; 2],
            size,
            faces: faces(),
            line: Line::Known {
                height: line_height,
                size,
                faces: Box::new(faces()),
            },
        }
    }

    pub(super) fn initial_relative(&self) -> Relative<'static> {
        Relative {
            counter_styles: Some(self.counter_styles.clone()),
            ..Relative::with_initial_font(self.environment.relative.initial_font.clone())
        }
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
                viewport: Viewport::Known(viewport, false),
                paper: self.paper,
                tree: Tree::Root,
                containers: parent.containers.clone(),
                query_sizes: Some(&self.query_sizes),
                ..self.initial_relative()
            },
            Some(root) => Relative {
                physical_unit_scale: self.environment.relative.physical_unit_scale,
                font: self
                    .basis(
                        parent.size,
                        parent.line_height,
                        &parent.font_candidates,
                        parent.font_selection(),
                        &parent.font_adjustments,
                        resolver,
                    )
                    .with_vertical_units(parent.vertical_font_units()),
                root: self
                    .basis(
                        root.size,
                        root.line_height,
                        &root.candidates,
                        root.selection,
                        &root.adjustments,
                        resolver,
                    )
                    .with_vertical_units(root.vertical_units),
                viewport: Viewport::Known(
                    viewport,
                    parent
                        .root_font
                        .as_ref()
                        .is_some_and(|root| root.writing_mode.vertical()),
                ),
                paper: self.paper,
                tree: Tree::Root,
                grammar_only: false,
                work: Default::default(),
                initial_page_font_size: false,
                initial_font: self.environment.relative.initial_font.clone(),
                color_scheme: self.used_color_scheme(&parent.color_scheme),
                counter_styles: Some(self.counter_styles.clone()),
                accent_color: parent.accent_color,
                containers: parent.containers.clone(),
                query_sizes: Some(&self.query_sizes),
                anchoring: None,
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
        if root && let Viewport::Known(size, _) = inherited.viewport {
            inherited.viewport = Viewport::Known(size, style.writing_mode.vertical());
        }
        if let Some(value) = winners.remove(&Property::ColorScheme) {
            properties::apply(Property::ColorScheme, &value, style, parent, &inherited)?;
        }
        let color_scheme = self.used_color_scheme(&style.color_scheme);
        // `AccentColor` in `color` takes the element's accent colour, whose `currentcolor`
        // is then recomputed against the element's own colour.
        let accent = winners.remove(&Property::AccentColor);
        if let Some(value) = &accent {
            properties::apply(Property::AccentColor, value, style, parent, &inherited)?;
        }
        let initial_page_font_size = inherited.initial_page_font_size;
        for property in [
            Property::MathDepth,
            Property::MathStyle,
            Property::MathShift,
            Property::FontSize,
        ]
        .iter()
        .chain(&FONT_SELECTION)
        {
            inherited.initial_page_font_size =
                initial_page_font_size && *property == Property::FontSize;
            if let Some(value) = winners.remove(property) {
                properties::apply(*property, &value, style, parent, &inherited)?;
            }
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
            physical_unit_scale,
            font: inherited_font,
            root: inherited_root,
            viewport,
            paper,
            tree,
            work,
            containers,
            query_sizes,
            anchoring,
            ..
        } = inherited;
        let own = |line| {
            FontBasis::Known {
                vertical: [false; 2],
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
            }
            .with_vertical_units(style.vertical_font_units())
        };
        // On the root, root-relative units measure the root's font except that `rlh` keeps
        // the initial line height until `line-height` computes.
        let line_basis = Relative {
            physical_unit_scale,
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
            counter_styles: Some(self.counter_styles.clone()),
            accent_color: style.accent_color,
            containers: containers.clone(),
            query_sizes,
            anchoring,
        };
        if let Some(value) = winners.remove(&Property::LineHeight) {
            properties::apply(Property::LineHeight, &value, style, parent, &line_basis)?;
        }
        if root {
            self.capture_root_font(style);
        }
        let mut relative = Relative {
            physical_unit_scale,
            font: self
                .basis(
                    style.size,
                    style.line_height,
                    &style.font_candidates,
                    style.font_selection(),
                    &style.font_adjustments,
                    resolver,
                )
                .with_vertical_units(style.vertical_font_units()),
            root: if root {
                self.basis(
                    style.size,
                    style.line_height,
                    &style.font_candidates,
                    style.font_selection(),
                    &style.font_adjustments,
                    resolver,
                )
                .with_vertical_units(style.vertical_font_units())
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
            counter_styles: Some(self.counter_styles.clone()),
            accent_color: style.accent_color,
            containers,
            query_sizes,
            anchoring,
        };
        // The root's initial `canvastext` follows its own colour scheme.
        let initial = Specified::Wide(crate::style::wide_keywords::WideKeyword::Initial);
        if let Some(value) = winners
            .remove(&Property::Color)
            .or_else(|| root.then_some(initial))
        {
            properties::apply(Property::Color, &value, style, parent, &relative)?;
        }
        if let Some(value) = &accent {
            properties::apply(Property::AccentColor, value, style, parent, &relative)?;
            relative.accent_color = style.accent_color;
        }
        style.decoration = initial_decoration();
        style.reset_current_colors();
        style.text_decorations.declared.color = style.color;
        style.text_paint.resolve(&relative, &style.computed_color)?;
        style.shadows.resolve(&relative, &style.computed_color)?;
        crate::style::svg_values::SvgStyle::resolve(
            &mut style.svg,
            &relative,
            &style.computed_color,
        )?;
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
            writing_mode: style.writing_mode,
            vertical_units: style.vertical_font_units(),
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
        if let Some(value) = winners.remove(&Property::Zoom) {
            properties::apply(Property::Zoom, &value, &mut style, parent, relative)?;
        }
        style.effective_zoom = crate::style::accumulate_zoom(parent.effective_zoom, style.zoom)?;
        for property in Property::BorderStyle.longhands().iter().chain([
            &Property::OutlineStyle,
            &Property::Direction,
            &Property::PositionAnchor,
            &Property::PositionArea,
        ]) {
            if let Some(value) = winners.remove(property) {
                properties::apply(*property, &value, &mut style, parent, relative)?;
            }
        }
        let initial = Specified::Wide(crate::style::wide_keywords::WideKeyword::Initial);
        for property in Property::BorderWidth
            .longhands()
            .iter()
            .chain([&Property::OutlineWidth])
        {
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
            && matches!(relative.tree, Tree::Element(element) if element.namespace() == crate::dom::MATHML_NAMESPACE)
        {
            style.white_space = WhiteSpace::NOWRAP;
            style.flex.justify = crate::style::Alignment::Normal;
            style.flex.align_content = crate::style::Alignment::Normal;
            style.flex.justify_overflow = crate::style::OverflowPosition::Default;
            style.flex.align_content_overflow = crate::style::OverflowPosition::Default;
            style.flex.align_self_overflow = crate::style::OverflowPosition::Default;
            style.grid.justify_self_overflow = crate::style::OverflowPosition::Default;
            style.flex.align_self = crate::style::Alignment::Auto;
            style.grid.justify_self = crate::style::Alignment::Auto;
        }
        // Out-of-flow boxes and the contents of atomic inlines receive no decorations, nor
        // do ruby annotations beyond their annotation container's own (CSS Ruby 1 §4.4).
        if style.position.is_out_of_flow()
            || style.floating().is_some()
            || (style.display.is_atomic_inline() && !parent.display.is_item_container())
            || style.display == Display::RubyTextContainer
            || (style.display == Display::RubyText && parent.display != Display::RubyTextContainer)
        {
            style.text_decorations.propagated.clear();
        }
        if style.display.is_ruby() {
            style.unicode_bidi = style.unicode_bidi.forced_isolation();
        }
        let finite_zoomed_length = |length: f32| (length * style.effective_zoom).is_finite();
        if style.decoration.shadows.iter().any(|shadow| {
            [
                shadow.offset[0],
                shadow.offset[1],
                shadow.blur,
                shadow.spread,
            ]
            .into_iter()
            .any(|length| !finite_zoomed_length(length))
        }) {
            return Err("non-finite zoomed box-shadow dimensions".into());
        }
        if style.text_paint.shadows.iter().any(|shadow| {
            [shadow.offset[0], shadow.offset[1], shadow.blur]
                .into_iter()
                .any(|length| !finite_zoomed_length(length))
        }) {
            return Err("non-finite zoomed text-shadow dimensions".into());
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
