use crate::style::{
    StyleError,
    box_values::unsupported,
    component::Component,
    counter_style::Representation,
    counter_style::{CounterStyle, Scope},
    grammar::{Keyword, Label, Production},
    quotes::{QuoteAction, QuoteDepth, Quotes},
    units::Relative,
};

/// CSS Lists 3 §4.7: the style of `counter()` and `counters()` without one.
const DEFAULT_COUNTER_STYLE: &str = "decimal";

/// What `content()` takes from its element (CSS GCPM 3 §1.1.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize)]
pub(crate) enum ElementContent {
    Text,
    Before,
    After,
    FirstLetter,
    Marker,
}
impl ElementContent {
    /// The `content()` argument that names this source.
    fn keyword(self) -> Keyword {
        match self {
            Self::Text => Keyword::Text,
            Self::Before => Keyword::Before,
            Self::After => Keyword::After,
            Self::FirstLetter => Keyword::FirstLetter,
            Self::Marker => Keyword::Marker,
        }
    }
}
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
enum GeneratedPart {
    Text(String),
    Leader(String),
    Counter(CounterRequest),
    NamedString(NamedStringRequest),
    /// `element()` (CSS GCPM 3 §1.2.2), the whole content of a page-margin box.
    RunningElement(NamedStringRequest),
    Target(TargetRequest),
    Page(CounterStyle),
    Pages(CounterStyle),
    Element(ElementContent),
    Quote(QuoteAction),
    /// An anonymous inline replaced element, which text-only uses omit (CSS Content 3 §2.2).
    Image(crate::paint::BackgroundImage),
}
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub(crate) struct GeneratedContent(Vec<GeneratedPart>);

/// What the `<image>` values of a content list compute against: the element's values and its
/// `currentcolor`.
pub(crate) struct ImageBasis<'a, 'r> {
    pub relative: &'a Relative<'r>,
    pub current: &'a crate::paint::AbsoluteColor,
}

pub(crate) struct GeneratedOutput {
    text: String,
    leaders: Vec<std::ops::Range<usize>>,
    /// Images with the text offsets they stand at, in text order.
    images: Vec<(usize, crate::paint::BackgroundImage)>,
}
impl From<Representation> for GeneratedOutput {
    fn from(representation: Representation) -> Self {
        let mut images = Vec::new();
        let text = representation_text(representation, 0, &mut images);
        Self {
            text,
            leaders: Vec::new(),
            images,
        }
    }
}
impl GeneratedOutput {
    pub(crate) fn text(&self) -> &str {
        &self.text
    }
    pub(crate) fn leaders(&self) -> &[std::ops::Range<usize>] {
        &self.leaders
    }
    pub(crate) fn images(&self) -> &[(usize, crate::paint::BackgroundImage)] {
        &self.images
    }
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum ContentContext {
    #[default]
    Element,
    BeforeAfter,
    /// `::marker`, whose `normal` content is the list item's marker image or string.
    Marker,
    Margin,
}
#[derive(Clone, Debug, PartialEq)]
pub(super) enum ComputedContent {
    Contents,
    None,
    Items {
        content: GeneratedContent,
        alternative: Option<GeneratedContent>,
    },
}
pub(super) static INITIAL_CONTENT: std::sync::LazyLock<std::sync::Arc<ComputedContent>> =
    std::sync::LazyLock::new(|| std::sync::Arc::new(ComputedContent::Contents));
impl ComputedContent {
    pub(super) fn supported(&self) -> bool {
        match self {
            Self::Items {
                content,
                alternative,
            } => {
                content.supported() && alternative.as_ref().is_none_or(GeneratedContent::supported)
            }
            _ => true,
        }
    }
    pub(super) fn from_component(
        value: &Component,
        context: ContentContext,
        scope: &Scope<'_>,
        images: &ImageBasis<'_, '_>,
    ) -> Result<std::sync::Arc<Self>, StyleError> {
        if let Some(element) = value.find(Label::Production(Production::RunningElementFn)) {
            // CSS GCPM 3 §1.2.2: `element()` is valid only in page-margin boxes; elsewhere it
            // is invalid at computed-value time and `content` takes its initial value.
            if context != ContentContext::Margin {
                return Ok(if context == ContentContext::BeforeAfter {
                    std::sync::Arc::new(Self::None)
                } else {
                    INITIAL_CONTENT.clone()
                });
            }
            let Component::Function(Keyword::Element, Some(arguments)) = element.unwrap() else {
                return Err(unsupported(element));
            };
            return Ok(std::sync::Arc::new(Self::Items {
                content: GeneratedContent(vec![GeneratedPart::RunningElement(page_value_request(
                    arguments, element,
                )?)]),
                alternative: None,
            }));
        }
        let value = match value.keyword() {
            Some(Keyword::Normal) if context != ContentContext::BeforeAfter => {
                return Ok(INITIAL_CONTENT.clone());
            }
            Some(Keyword::Normal | Keyword::None) => Self::None,
            _ => Self::Items {
                content: GeneratedContent::from_component(value, scope, Some(images))?
                    .ok_or_else(|| unsupported(value))?,
                alternative: alternative(value)
                    .map(|value| GeneratedContent::from_list(&value, scope, Some(images)))
                    .transpose()?,
            },
        };
        Ok(std::sync::Arc::new(value))
    }
    /// The content list replacing an element's or pseudo-element's contents, if any.
    pub(crate) fn items(&self) -> Option<&GeneratedContent> {
        match self {
            Self::Items { content, .. } => Some(content),
            Self::Contents | Self::None => None,
        }
    }
}
fn alternative(value: &Component) -> Option<Component> {
    match value.unwrap() {
        Component::Group(values) => {
            if let Some(index) = values
                .iter()
                .position(|value| matches!(value.unwrap(), Component::Delim('/')))
            {
                return Some(match &values[index + 1..] {
                    [value] => value.clone(),
                    values => Component::Group(values.to_vec()),
                });
            }
            values.iter().find_map(alternative)
        }
        Component::Slots(values) => values.iter().flatten().find_map(alternative),
        _ => None,
    }
}
const PAGE_COUNTER: &str = "page";
const PAGES_COUNTER: &str = "pages";

#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Serialize)]
pub(crate) struct CounterRequest {
    pub name: std::sync::Arc<str>,
    pub separator: Option<std::sync::Arc<str>>,
    pub style: CounterStyle,
}
impl CounterRequest {
    pub(crate) fn is_page_counter(&self) -> bool {
        matches!(self.name.as_ref(), PAGE_COUNTER | PAGES_COUNTER)
    }
    pub(crate) fn page_text(&self, page: usize, pages: usize) -> Option<String> {
        let value = match self.name.as_ref() {
            PAGE_COUNTER => page,
            PAGES_COUNTER => pages,
            _ => return None,
        };
        Some(self.style.representation(value as i128).into_text())
    }

    /// First-page selection needs box existence, not the numeric glyphs; `None` when only
    /// the counter values can tell.
    pub(crate) fn box_generation_text(&self) -> Option<String> {
        match (&self.style, &self.separator) {
            (CounterStyle::None, Some(separator)) if !separator.is_empty() => None,
            (CounterStyle::None, _) => Some(String::new()),
            (style, _) if style.may_be_empty() => None,
            (style, _) => Some(
                style
                    .representation(crate::style::INITIAL_COUNTER_VALUE)
                    .into_text(),
            ),
        }
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum TargetReferenceError {
    #[error("invalid URL")]
    InvalidUrl,
    #[error("reference points outside the current document")]
    ExternalDocument,
    #[error("reference has no nonempty fragment")]
    MissingFragment,
    #[error("fragment does not identify an element ID")]
    MissingId,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Serialize)]
pub(crate) enum TargetValue {
    Counter(CounterRequest),
    Text(ElementContent),
}
#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Serialize)]
pub(crate) struct TargetRequest {
    pub reference: std::sync::Arc<str>,
    pub value: TargetValue,
}
pub(crate) type TargetText<'a> = dyn FnMut(&TargetRequest) -> Result<String, StyleError> + 'a;

/// Supplies a counter's representation, whose images only content that lays out boxes shows.
pub(crate) type CounterText<'a> =
    dyn FnMut(&CounterRequest) -> Result<Representation, StyleError> + 'a;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash, serde::Serialize)]
pub(crate) enum NamedStringMode {
    #[default]
    First,
    Start,
    Last,
    FirstExcept,
}
#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Serialize)]
pub(crate) struct NamedStringRequest {
    pub name: std::sync::Arc<str>,
    pub mode: NamedStringMode,
}
pub(crate) type NamedStringText<'a> =
    dyn FnMut(&NamedStringRequest) -> Result<String, StyleError> + 'a;
struct PageContent<'a, 's> {
    number: usize,
    total: usize,
    strings: Option<&'a mut NamedStringText<'s>>,
}

/// Supplies the text that `content()` takes from the element being rendered.
pub(crate) type ElementContentText<'a> =
    dyn FnMut(ElementContent) -> Result<String, StyleError> + 'a;

impl GeneratedContent {
    pub(crate) fn has_quote_actions(&self) -> bool {
        self.0
            .iter()
            .any(|part| matches!(part, GeneratedPart::Quote(_)))
    }
    pub(crate) fn advance_quote_depth(&self, depth: &mut QuoteDepth) {
        for part in &self.0 {
            if let GeneratedPart::Quote(action) = part {
                depth.apply(*action, &Quotes::default());
            }
        }
    }

    pub(crate) fn supported(&self) -> bool {
        self.0.iter().all(|part| {
            !matches!(
                part,
                GeneratedPart::Element(ElementContent::FirstLetter | ElementContent::Marker)
            )
        })
    }
    /// CSS Content 3 §1: a content list of a single `<image>` is a `<content-replacement>`,
    /// which makes its element a replaced element filled with the image.
    pub(crate) fn replacement(&self) -> Option<&crate::paint::BackgroundImage> {
        match self.0.as_slice() {
            [GeneratedPart::Image(image)] => Some(image),
            _ => None,
        }
    }
    pub(crate) fn has_images(&self) -> bool {
        self.0
            .iter()
            .any(|part| matches!(part, GeneratedPart::Image(_)))
    }
    /// `content(text)`, the initial `bookmark-label` (CSS GCPM 3 §4.2).
    pub(crate) fn element_text_content() -> Self {
        Self(vec![GeneratedPart::Element(ElementContent::Text)])
    }
    /// `normal` and `none` generate nothing. Alternative text after `/` describes the content
    /// for assistive technology and does not render.
    pub(super) fn from_component(
        value: &Component,
        scope: &Scope<'_>,
        images: Option<&ImageBasis<'_, '_>>,
    ) -> Result<Option<Self>, StyleError> {
        if matches!(value.keyword(), Some(Keyword::Normal | Keyword::None)) {
            return Ok(None);
        }
        if let (Some(image), Some(images)) = (
            value.find(Label::Production(Production::ContentReplacement)),
            images,
        ) {
            return crate::style::background_values::image(image, images.relative, images.current)
                .map(|image| Some(Self(vec![GeneratedPart::Image(image)])));
        }
        let list = value
            .find(Label::Production(Production::ContentList))
            .ok_or_else(|| unsupported(value))?;
        Self::from_list(list, scope, images).map(Some)
    }
    /// A content list, such as `bookmark-label`'s (CSS GCPM 3 §1.1); `images` computes its
    /// `<image>` values where the grammar allows them.
    pub(super) fn from_list(
        list: &Component,
        scope: &Scope<'_>,
        images: Option<&ImageBasis<'_, '_>>,
    ) -> Result<Self, StyleError> {
        list.items()
            .iter()
            .map(|item| part(item, scope, images))
            .collect::<Result<_, _>>()
            .map(Self)
    }
    pub(crate) fn page_text(
        &self,
        page: usize,
        pages: usize,
        limit: usize,
    ) -> Result<String, StyleError> {
        self.page_text_with_quotes(page, pages, limit, &Quotes::default())
    }
    pub(crate) fn page_text_with_quotes(
        &self,
        page: usize,
        pages: usize,
        limit: usize,
        quotes: &Quotes,
    ) -> Result<String, StyleError> {
        self.render(
            Some(PageContent {
                number: page,
                total: pages,
                strings: None,
            }),
            limit,
            None,
            None,
            quotes,
            &mut QuoteDepth::default(),
        )
        .map(|output| output.text)
    }
    #[cfg(test)]
    pub(crate) fn page_text_with_strings(
        &self,
        page: usize,
        pages: usize,
        limit: usize,
        quotes: &Quotes,
        strings: &mut NamedStringText<'_>,
    ) -> Result<String, StyleError> {
        self.render(
            Some(PageContent {
                number: page,
                total: pages,
                strings: Some(strings),
            }),
            limit,
            None,
            None,
            quotes,
            &mut QuoteDepth::default(),
        )
        .map(|output| output.text)
    }
    pub(crate) fn element_text_with_quotes(
        &self,
        element: crate::dom::Element<'_>,
        limit: usize,
        quotes: &Quotes,
        depth: &mut QuoteDepth,
    ) -> Result<String, StyleError> {
        let _ = element;
        self.render(None, limit, None, None, quotes, depth)
            .map(|output| output.text)
    }
    pub(crate) fn element_text_with_sources(
        &self,
        limit: usize,
        content: &mut ElementContentText<'_>,
        counters: &mut CounterText<'_>,
        quotes: &Quotes,
        depth: &mut QuoteDepth,
    ) -> Result<String, StyleError> {
        self.render(None, limit, Some(content), Some(counters), quotes, depth)
            .map(|output| output.text)
    }
    /// Captures element and document-counter values while retaining page-dependent parts.
    pub(crate) fn bind_element_sources(
        &self,
        limit: usize,
        content: &mut ElementContentText<'_>,
        counters: &mut CounterText<'_>,
    ) -> Result<Self, StyleError> {
        self.bind_parts(limit, &mut |part| match part {
            GeneratedPart::Element(source) => content(*source).map(Some),
            GeneratedPart::Counter(request) => {
                counters(request).map(Representation::into_text).map(Some)
            }
            _ => Ok(None),
        })
    }

    pub(crate) fn target_requests(&self) -> impl Iterator<Item = &TargetRequest> {
        self.0.iter().filter_map(|part| match part {
            GeneratedPart::Target(request) => Some(request),
            _ => None,
        })
    }
    /// The running element a page-margin box's `element()` content places.
    pub(crate) fn running_element(&self) -> Option<&NamedStringRequest> {
        match self.0.as_slice() {
            [GeneratedPart::RunningElement(request)] => Some(request),
            _ => None,
        }
    }
    pub(crate) fn named_string_requests(&self) -> impl Iterator<Item = &NamedStringRequest> {
        self.0.iter().filter_map(|part| match part {
            GeneratedPart::NamedString(request) => Some(request),
            _ => None,
        })
    }
    pub(crate) fn bind_named_strings(
        &self,
        limit: usize,
        strings: &mut NamedStringText<'_>,
    ) -> Result<Self, StyleError> {
        self.bind_parts(limit, &mut |part| match part {
            GeneratedPart::NamedString(request) => strings(request).map(Some),
            _ => Ok(None),
        })
    }
    pub(crate) fn bind_targets(
        &self,
        limit: usize,
        targets: &mut TargetText<'_>,
    ) -> Result<Self, StyleError> {
        self.bind_parts(limit, &mut |part| match part {
            GeneratedPart::Target(request) => targets(request).map(Some),
            _ => Ok(None),
        })
    }
    fn bind_parts(
        &self,
        limit: usize,
        resolve: &mut dyn FnMut(&GeneratedPart) -> Result<Option<String>, StyleError>,
    ) -> Result<Self, StyleError> {
        let mut bytes = 0usize;
        let parts = self
            .0
            .iter()
            .map(|part| {
                let bound = resolve(part)?.map_or_else(|| part.clone(), GeneratedPart::Text);
                if let GeneratedPart::Text(text) | GeneratedPart::Leader(text) = &bound {
                    bytes = bytes
                        .checked_add(text.len())
                        .filter(|bytes| *bytes <= limit)
                        .ok_or(StyleError::GeneratedContentLimit { limit })?;
                }
                Ok(bound)
            })
            .collect::<Result<_, StyleError>>()?;
        Ok(Self(parts))
    }

    pub(crate) fn counter_requests(&self) -> impl Iterator<Item = &CounterRequest> {
        self.0.iter().filter_map(|part| match part {
            GeneratedPart::Counter(request) => Some(request),
            _ => None,
        })
    }
    pub(crate) fn element_text_with_counters(
        &self,
        limit: usize,
        counters: &mut CounterText<'_>,
        quotes: &Quotes,
        depth: &mut QuoteDepth,
    ) -> Result<String, StyleError> {
        self.render(None, limit, None, Some(counters), quotes, depth)
            .map(|output| output.text)
    }
    pub(crate) fn element_output_with_counters(
        &self,
        limit: usize,
        counters: &mut CounterText<'_>,
        quotes: &Quotes,
        depth: &mut QuoteDepth,
    ) -> Result<GeneratedOutput, StyleError> {
        self.render(None, limit, None, Some(counters), quotes, depth)
    }
    pub(crate) fn page_output_with_strings(
        &self,
        page: usize,
        pages: usize,
        limit: usize,
        quotes: &Quotes,
        strings: &mut NamedStringText<'_>,
    ) -> Result<GeneratedOutput, StyleError> {
        self.render(
            Some(PageContent {
                number: page,
                total: pages,
                strings: Some(strings),
            }),
            limit,
            None,
            None,
            quotes,
            &mut QuoteDepth::default(),
        )
    }
    fn render(
        &self,
        mut page: Option<PageContent<'_, '_>>,
        limit: usize,
        mut content: Option<&mut ElementContentText<'_>>,
        mut counters: Option<&mut CounterText<'_>>,
        quotes: &Quotes,
        depth: &mut QuoteDepth,
    ) -> Result<GeneratedOutput, StyleError> {
        let mut text = String::new();
        let mut leaders = Vec::new();
        let mut images = Vec::new();
        for part in &self.0 {
            let fragment: std::borrow::Cow<'_, str> = match part {
                GeneratedPart::Image(image) => {
                    images.push((text.len(), image.clone()));
                    continue;
                }
                GeneratedPart::Text(value) | GeneratedPart::Leader(value) => value.as_str().into(),
                GeneratedPart::Counter(request) => {
                    let representation = counters.as_mut().ok_or_else(|| {
                        StyleError::Unsupported(
                            "document counters outside a counter context".into(),
                        )
                    })?(request)?;
                    representation_text(representation, text.len(), &mut images).into()
                }
                GeneratedPart::Target(_) => {
                    return Err(StyleError::Unsupported(
                        "target reference outside a resolved target context".into(),
                    ));
                }
                GeneratedPart::NamedString(request) => page
                    .as_mut()
                    .and_then(|page| page.strings.as_mut())
                    .ok_or_else(|| {
                        StyleError::Unsupported(
                            "named strings outside a page string context".into(),
                        )
                    })?(request)?
                .into(),
                GeneratedPart::RunningElement(_) => {
                    return Err(StyleError::Unsupported(
                        "element() outside page-margin box layout".into(),
                    ));
                }
                GeneratedPart::Quote(action) => depth.apply(*action, quotes).into(),
                GeneratedPart::Page(style) | GeneratedPart::Pages(style) => {
                    let page = page.as_ref().ok_or_else(|| {
                        StyleError::Unsupported("page counters inside document content".to_owned())
                    })?;
                    let representation =
                        style.representation(if matches!(part, GeneratedPart::Page(_)) {
                            page.number as i128
                        } else {
                            page.total as i128
                        });
                    representation_text(representation, text.len(), &mut images).into()
                }
                GeneratedPart::Element(source) => {
                    let content = content.as_mut().ok_or_else(|| {
                        StyleError::Unsupported(format!(
                            "content({}) outside an element",
                            source.keyword().name()
                        ))
                    })?;
                    content(*source)?.into()
                }
            };
            if text
                .len()
                .checked_add(fragment.len())
                .is_none_or(|size| size > limit)
            {
                return Err(StyleError::GeneratedContentLimit { limit });
            }
            if matches!(part, GeneratedPart::Leader(_)) && !fragment.is_empty() {
                leaders.push(text.len()..text.len() + fragment.len());
            }
            text.push_str(&fragment);
        }
        Ok(GeneratedOutput {
            text,
            leaders,
            images,
        })
    }
}

/// A counter representation's text, its images recorded from the text offset `at` it is
/// appended at.
fn representation_text(
    representation: Representation,
    at: usize,
    images: &mut Vec<(usize, crate::paint::BackgroundImage)>,
) -> String {
    images.extend(
        representation
            .images()
            .iter()
            .map(|(offset, image)| (at + offset, (**image).clone())),
    );
    representation.into_text()
}

fn part(
    item: &Component,
    scope: &Scope<'_>,
    images: Option<&ImageBasis<'_, '_>>,
) -> Result<GeneratedPart, StyleError> {
    if let (Component::Reference(Label::Production(Production::Image), _), Some(images)) =
        (item, images)
    {
        return crate::style::background_values::image(item, images.relative, images.current)
            .map(GeneratedPart::Image);
    }
    match item.unwrap() {
        Component::Keyword(Keyword::OpenQuote) => Ok(GeneratedPart::Quote(QuoteAction::Open)),
        Component::Keyword(Keyword::CloseQuote) => Ok(GeneratedPart::Quote(QuoteAction::Close)),
        Component::Keyword(Keyword::NoOpenQuote) => Ok(GeneratedPart::Quote(QuoteAction::NoOpen)),
        Component::Keyword(Keyword::NoCloseQuote) => Ok(GeneratedPart::Quote(QuoteAction::NoClose)),
        Component::String(text) => Ok(GeneratedPart::Text(text.to_string())),
        Component::Function(Keyword::Leader, Some(arguments)) => {
            let pattern = match arguments.unwrap() {
                Component::String(value) => value.as_ref(),
                Component::Keyword(Keyword::Dotted) => ".",
                Component::Keyword(Keyword::Solid) => "_",
                Component::Keyword(Keyword::Space) => " ",
                _ => return Err(unsupported(item)),
            };
            Ok(GeneratedPart::Leader(
                pattern
                    .chars()
                    .filter(|ch| !matches!(ch, '\n' | '\r' | '\u{000c}' | '\u{2028}' | '\u{2029}'))
                    .collect(),
            ))
        }
        Component::Function(Keyword::Counter, Some(arguments)) => {
            counter(arguments, item, false, scope)
        }
        Component::Function(Keyword::Counters, Some(arguments)) => {
            counter(arguments, item, true, scope)
        }
        Component::Function(
            function @ (Keyword::TargetCounter | Keyword::TargetCounters | Keyword::TargetText),
            Some(arguments),
        ) => target(*function, arguments, item, scope),
        Component::Function(Keyword::String, Some(arguments)) => Ok(GeneratedPart::NamedString(
            page_value_request(arguments, item)?,
        )),
        Component::Function(Keyword::Content, argument) => {
            let keyword = argument
                .as_deref()
                .and_then(|body| body.items().first())
                .and_then(Component::keyword);
            let source = match keyword {
                None | Some(Keyword::Text) => ElementContent::Text,
                Some(Keyword::Before) => ElementContent::Before,
                Some(Keyword::After) => ElementContent::After,
                Some(Keyword::FirstLetter) => ElementContent::FirstLetter,
                Some(Keyword::Marker) => ElementContent::Marker,
                Some(_) => return Err(unsupported(item)),
            };
            Ok(GeneratedPart::Element(source))
        }
        _ => Err(unsupported(item)),
    }
}

/// The name and selection keyword of `string()` and `element()` (CSS GCPM 3 §1.1.2, §1.2.2).
fn page_value_request(
    arguments: &Component,
    item: &Component,
) -> Result<NamedStringRequest, StyleError> {
    let Some(Component::Ident(name)) = arguments.items().first().map(Component::unwrap) else {
        return Err(unsupported(item));
    };
    let mut keywords = Vec::new();
    arguments.keywords(&mut keywords);
    let mode = match keywords.last().copied() {
        None | Some(Keyword::First) => NamedStringMode::First,
        Some(Keyword::Start) => NamedStringMode::Start,
        Some(Keyword::Last) => NamedStringMode::Last,
        Some(Keyword::FirstExcept) => NamedStringMode::FirstExcept,
        _ => return Err(unsupported(item)),
    };
    Ok(NamedStringRequest {
        name: std::sync::Arc::from(name.as_ref()),
        mode,
    })
}

fn target(
    function: Keyword,
    arguments: &Component,
    item: &Component,
    scope: &Scope<'_>,
) -> Result<GeneratedPart, StyleError> {
    let items = arguments.items();
    let reference = match items.first().map(Component::unwrap) {
        Some(Component::String(value) | Component::Url(value)) => {
            std::sync::Arc::from(value.as_ref())
        }
        _ => return Err(unsupported(item)),
    };
    let value = if function == Keyword::TargetText {
        let mut keywords = Vec::new();
        arguments.keywords(&mut keywords);
        TargetValue::Text(match keywords.last().copied() {
            None | Some(Keyword::Content) => ElementContent::Text,
            Some(Keyword::Before) => ElementContent::Before,
            Some(Keyword::After) => ElementContent::After,
            Some(Keyword::FirstLetter) => ElementContent::FirstLetter,
            _ => return Err(unsupported(item)),
        })
    } else {
        let name = items
            .iter()
            .skip(1)
            .find_map(|item| match item.unwrap() {
                Component::Ident(name) => Some(std::sync::Arc::from(name.as_ref())),
                _ => None,
            })
            .ok_or_else(|| unsupported(item))?;
        let separator = if function == Keyword::TargetCounters {
            Some(
                items
                    .iter()
                    .skip(1)
                    .find_map(|item| match item.unwrap() {
                        Component::String(value) => Some(std::sync::Arc::from(value.as_ref())),
                        _ => None,
                    })
                    .ok_or_else(|| unsupported(item))?,
            )
        } else {
            None
        };
        TargetValue::Counter(CounterRequest {
            name,
            separator,
            style: counter_style(arguments, scope)?,
        })
    };
    Ok(GeneratedPart::Target(TargetRequest { reference, value }))
}

fn counter_style(arguments: &Component, scope: &Scope<'_>) -> Result<CounterStyle, StyleError> {
    match arguments.find(Label::Production(Production::CounterStyle)) {
        None => Ok(scope.named(DEFAULT_COUNTER_STYLE)),
        Some(style) => scope.resolve(style),
    }
}

/// Page counters use the same counter styles as list markers.
fn counter(
    arguments: &Component,
    item: &Component,
    multiple: bool,
    scope: &Scope<'_>,
) -> Result<GeneratedPart, StyleError> {
    let Some(Component::Ident(name)) = arguments
        .find(Label::Production(Production::CounterName))
        .map(Component::unwrap)
    else {
        return Err(unsupported(item));
    };
    let style = counter_style(arguments, scope)?;
    let separator = if multiple {
        Some(
            arguments
                .items()
                .iter()
                .find_map(|item| match item.unwrap() {
                    Component::String(value) => Some(std::sync::Arc::from(value.as_ref())),
                    _ => None,
                })
                .ok_or_else(|| unsupported(item))?,
        )
    } else {
        None
    };
    match name.as_ref() {
        PAGE_COUNTER => Ok(GeneratedPart::Page(style)),
        PAGES_COUNTER => Ok(GeneratedPart::Pages(style)),
        _ => Ok(GeneratedPart::Counter(CounterRequest {
            name: std::sync::Arc::from(name.as_ref()),
            separator,
            style,
        })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::{
        grammar::Property,
        matcher::{ValueContext, match_value},
    };
    const TEST_CONTENT_LIMIT: usize = 1024;

    #[test]
    fn target_requests_preserve_reference_counter_and_text_arguments() {
        let source = "target-counter('#chapter', page, upper-roman) '/' target-counters(url('#part'), section, '.', lower-alpha) '/' target-text('#title', before)";
        let value = match_value(
            Property::Content.data().root,
            source,
            &mut 0,
            ValueContext::Element,
        )
        .unwrap()
        .expect("target grammar must accept all three reference forms");
        let content = GeneratedContent::from_component(&value, &Scope::predefined(), None)
            .unwrap()
            .unwrap();
        let requests: Vec<_> = content.target_requests().collect();
        assert_eq!(requests.len(), 3);
        assert_eq!(requests[0].reference.as_ref(), "#chapter");
        assert_eq!(
            requests[0].value,
            TargetValue::Counter(CounterRequest {
                name: "page".into(),
                separator: None,
                style: CounterStyle::predefined("upper-roman"),
            })
        );
        assert_eq!(requests[1].reference.as_ref(), "#part");
        assert_eq!(
            requests[1].value,
            TargetValue::Counter(CounterRequest {
                name: "section".into(),
                separator: Some(".".into()),
                style: CounterStyle::predefined("lower-alpha"),
            })
        );
        assert_eq!(requests[2].value, TargetValue::Text(ElementContent::Before));
        let bound = content
            .bind_targets(TEST_CONTENT_LIMIT, &mut |request| {
                Ok(match request.reference.as_ref() {
                    "#chapter" => "XII",
                    "#part" => "b.d",
                    "#title" => "Prefix",
                    other => panic!("unexpected target {other}"),
                }
                .into())
            })
            .unwrap();
        assert_eq!(
            bound.page_text(1, 1, TEST_CONTENT_LIMIT).unwrap(),
            "XII/b.d/Prefix"
        );
        assert!(
            matches!(
                content.page_text(1, 1, TEST_CONTENT_LIMIT),
                Err(StyleError::Unsupported(_))
            ),
            "an unresolved target must never silently become empty text"
        );
        assert!(
            matches!(
                content.bind_targets(2, &mut |_| Ok("long".into())),
                Err(StyleError::GeneratedContentLimit { limit: 2 })
            ),
            "target expansion must obey the existing text bound"
        );
    }

    #[test]
    fn named_string_requests_preserve_selection_modes_and_require_a_page_context() {
        let source = "string(heading) '/' string(heading,first) '/' string(heading,start) '/' string(heading,last) '/' string(heading,first-except)";
        let value = match_value(
            Property::Content.data().root,
            source,
            &mut 0,
            ValueContext::Element,
        )
        .unwrap()
        .unwrap();
        let content = GeneratedContent::from_component(&value, &Scope::predefined(), None)
            .unwrap()
            .unwrap();
        let mut modes = Vec::new();
        let text = content
            .page_text_with_strings(
                2,
                7,
                TEST_CONTENT_LIMIT,
                &Quotes::default(),
                &mut |request| {
                    assert_eq!(request.name.as_ref(), "heading");
                    modes.push(request.mode);
                    Ok(modes.len().to_string())
                },
            )
            .unwrap();
        assert_eq!(text, "1/2/3/4/5");
        assert_eq!(
            modes,
            [
                NamedStringMode::First,
                NamedStringMode::First,
                NamedStringMode::Start,
                NamedStringMode::Last,
                NamedStringMode::FirstExcept
            ]
        );
        assert!(
            matches!(
                content.page_text(2, 7, TEST_CONTENT_LIMIT),
                Err(StyleError::Unsupported(_))
            ),
            "missing page assignments must not silently become empty strings"
        );
    }
}
