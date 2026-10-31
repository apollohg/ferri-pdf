//! HTML loading, stylesheet discovery, and styled document construction.
mod content;
mod controls;
mod counters;
mod element_images;
mod embedded;
pub(crate) mod encoding;
pub(crate) mod first_letter;
mod first_line;
mod first_page;
mod formatting;
pub(crate) mod frames;
mod frameset;
mod generated;
mod inline;
mod lists;
pub(crate) mod mathml;
mod media;
mod metadata;
mod navigation;
mod page;
pub(crate) mod ruby;
pub(crate) mod scroll_markers;
mod semantics;
mod strings;
mod stylesheets;
pub(crate) mod svg;
mod tables;

use crate::dom::DocumentTree;
use crate::resources::{ResourceError, ResourceResolver};
use crate::style::{PrintSettings, SourceDiagnostic, StyleError, Stylesheet};

pub(crate) use counters::CounterSnapshot;
pub(crate) use element_images::{ElementMeasurements, ElementState, ShapeSamples};
pub(crate) use formatting::{
    BlockBox, BlockContent, Footnotes, InitialLetterBox, MAX_FORMATTING_WORK, PageBox,
    PageFootnoteArea, PageMarginBox,
};
pub(crate) use frames::{Frame, FrameDocument};
pub(crate) use generated::{GeneratedValues, InlineFootnotePlacement, PageStart, Paginated};
pub(crate) use inline::{
    Anchor, CaseRun, InlineBox, MAX_INLINE_TEXT_BYTES, OBJECT_REPLACEMENT_CHARACTER, OffsetMap,
    Paragraph, is_css_whitespace, narrow_width, transform_text,
};
pub(crate) use metadata::DocumentMetadata;
pub(crate) use navigation::{Bookmark, BoxNavigation, Heading, LinkTarget, Navigation};
pub(crate) use page::{PageBoundary, PageContent, page_axes};
pub(crate) use semantics::{InlineEvent, SemanticEvent};
pub(crate) use strings::{RunningElement, RunningStringAssignment};

#[derive(Debug, thiserror::Error)]
pub(crate) enum DocumentError {
    #[error("{0}")]
    Image(#[from] crate::images::ImageError),
    #[error("{0}")]
    Style(#[from] StyleError),
    #[error("{0}")]
    Resource(#[from] ResourceError),
    #[error("{0}")]
    Formatting(String),
    #[error("formatted inline text exceeds {limit} bytes")]
    TextExpansionLimit { limit: usize },
}

/// Recoverable input/source diagnostics retained with the rendering result.
#[derive(Debug, thiserror::Error)]
pub enum DocumentDiagnostic {
    #[error("{0}")]
    UnavailableImage(#[from] crate::images::ImageFailure),
    #[error("image omitted during PDF encoding: {error}")]
    ImageDecode { error: String },
    #[error("{0}")]
    Css(crate::style::StyleDiagnostic),
    #[error("{0}")]
    Degraded(crate::style::Degradation),
    #[error("{0}; the document content past it is ignored")]
    TreeLimit(crate::dom::DomError),
    #[error("{0}")]
    Layout(#[from] crate::layout::LayoutDiagnostic),
    #[error("{0}")]
    Paint(#[from] crate::writer::PaintDiagnostic),
    #[error("decoding {reference:?} as {encoding} replaced malformed byte sequences")]
    Decoding {
        base: crate::resources::ResourceBase,
        reference: Option<String>,
        encoding: String,
    },
    #[error("embedded resource {reference} is unavailable: {error}")]
    UnavailableEmbedded {
        base: crate::resources::ResourceBase,
        reference: String,
        #[source]
        error: ResourceError,
    },
    #[error("nested navigables beyond {limit} levels show nothing")]
    NavigableDepth { limit: usize },
    #[error("nested document {reference} of type {media_type} has no renderer and shows nothing")]
    UnrenderedDocument {
        reference: String,
        media_type: String,
    },
    #[error("{0}")]
    UnavailableFont(#[from] crate::fonts::FontFailure),
    #[error("stylesheet {reference} is unavailable: {error}")]
    UnavailableSource {
        base: crate::resources::ResourceBase,
        reference: String,
        #[source]
        error: ResourceError,
    },
    #[error("optional restyling past the {limit}-unit formatting work budget was skipped")]
    SkippedFormattingWork { limit: usize },
    /// Formatting kept within one of its limits by leaving out what exceeds it.
    #[error("{resource} exceeds {limit}: {fallback}")]
    FormattingLimit {
        resource: &'static str,
        limit: usize,
        fallback: &'static str,
    },
}

/// Records, once, that formatting skipped optional work past its budget's `limit`.
fn record_skipped_work(diagnostics: &mut Vec<DocumentDiagnostic>, limit: Option<usize>) {
    if let Some(limit) = limit
        && !diagnostics
            .iter()
            .any(|known| matches!(known, DocumentDiagnostic::SkippedFormattingWork { .. }))
    {
        diagnostics.push(DocumentDiagnostic::SkippedFormattingWork { limit });
    }
}

/// An owned document whose text and style ranges have been resolved together.
pub(crate) struct StyledDocument {
    root: BlockBox,
    /// Footnote element boxes in document order (CSS GCPM 3 §2.2).
    footnotes: Footnotes,
    styles: Stylesheet,
    diagnostics: Vec<DocumentDiagnostic>,
    navigation: Navigation,
    running_strings: Vec<RunningStringAssignment>,
    generated: generated::GeneratedReferences,
    source: std::rc::Rc<FormattingSource>,
    metadata: DocumentMetadata,
    canvas: crate::graphics::BoxDecoration,
    pages: page::PageProvider,
    svg_definitions: svg::SvgDefinitions,
    /// The document's counters where each page starts (CSS Page 3 §6.1).
    page_counters: std::collections::HashMap<usize, CounterSnapshot>,
    /// The scroll marker groups whose active markers layout determines (CSS Overflow 5
    /// §3.1.8).
    scroll_markers: scroll_markers::ScrollMarkerGroups,
    /// What the last layout measured of the content of intrinsically sized pages, and the
    /// first page sized to it.
    page_content: PageContent,
    first_page: std::sync::Arc<PageBox>,
    /// What the last layout measured of the `element()` images the next formatting resolves,
    /// and the renderings it measured, which floats' shapes sample.
    element_measurements: Box<ElementMeasurements>,
    element_renderings: Box<crate::layout::ElementRenderings>,
}

struct FormattingSource {
    html: DocumentTree,
    embedded: embedded::EmbeddedContent,
    base: crate::resources::ResourceBase,
    source_base: crate::resources::ResourceBase,
    document_url: Option<crate::resources::Url>,
    capture_fragment_targets: bool,
}

impl FormattingSource {
    fn build(
        &self,
        styles: &mut Stylesheet,
        values: GeneratedValues,
        (images, elements, definitions): (
            &std::rc::Rc<std::cell::RefCell<crate::images::ImageCache>>,
            &ElementMeasurements,
            &svg::SvgDefinitions,
        ),
        resolver: &dyn ResourceResolver,
    ) -> Result<formatting::Formatted, DocumentError> {
        let navigation = || {
            navigation::NavigationBuilder::new(
                &self.html,
                &self.base,
                self.capture_fragment_targets,
                self.document_url.clone(),
                &self.source_base,
            )
        };
        formatting::build(
            &formatting::Input {
                html: &self.html,
                embedded: &self.embedded,
                images,
                elements,
                definitions,
                work_limit: MAX_FORMATTING_WORK,
            },
            styles,
            (&self.base, resolver),
            &navigation,
            values,
        )
    }
}

/// Borrowed inputs shared by string and byte document parsing.
#[derive(Clone, Copy)]
pub(crate) struct ParseOptions<'a> {
    pub base: &'a crate::resources::ResourceBase,
    pub additional: &'a [String],
    pub user: &'a [String],
    pub settings: &'a PrintSettings,
    /// Retain targets and unresolved fragment intents for a multipart rendering job.
    pub capture_fragment_targets: bool,
    pub presentation: crate::style::Presentation,
    /// Document URLs of the navigables containing this document's, outermost first; empty for
    /// the printed document.
    pub ancestors: &'a [Option<crate::resources::Url>],
}

struct HtmlInput {
    tree: DocumentTree,
    encoding: String,
    diagnostics: Vec<DocumentDiagnostic>,
}

impl StyledDocument {
    #[cfg(test)]
    pub(crate) fn parse(
        html: &str,
        base: &crate::resources::ResourceBase,
        additional: &[String],
        resolver: &dyn ResourceResolver,
    ) -> Result<Self, DocumentError> {
        Self::parse_with_origins(html, base, additional, &[], resolver)
    }

    #[cfg(test)]
    pub(crate) fn parse_with_work_limit(
        html: &str,
        work_limit: usize,
    ) -> Result<Self, DocumentError> {
        Self::from_input(
            HtmlInput {
                tree: DocumentTree::parse(html),
                encoding: "UTF-8".into(),
                diagnostics: Vec::new(),
            },
            &ParseOptions {
                base: &crate::resources::ResourceBase::default(),
                additional: &[],
                user: &[],
                settings: &PrintSettings::default(),
                capture_fragment_targets: false,
                presentation: crate::style::Presentation::Paged,
                ancestors: &[],
            },
            &crate::resources::FileResolver::default(),
            work_limit,
        )
    }

    #[cfg(test)]
    pub(crate) fn parse_with_origins(
        html: &str,
        base: &crate::resources::ResourceBase,
        additional: &[String],
        user: &[String],
        resolver: &dyn ResourceResolver,
    ) -> Result<Self, DocumentError> {
        Self::parse_with_settings(
            html,
            base,
            additional,
            user,
            &PrintSettings::default(),
            resolver,
        )
    }

    pub(crate) fn parse_with_settings(
        html: &str,
        base: &crate::resources::ResourceBase,
        additional: &[String],
        user: &[String],
        settings: &PrintSettings,
        resolver: &dyn ResourceResolver,
    ) -> Result<Self, DocumentError> {
        Self::parse_with_options(
            html,
            &ParseOptions {
                base,
                additional,
                user,
                settings,
                capture_fragment_targets: false,
                presentation: crate::style::Presentation::Paged,
                ancestors: &[],
            },
            resolver,
        )
    }

    pub(crate) fn parse_with_options(
        html: &str,
        options: &ParseOptions<'_>,
        resolver: &dyn ResourceResolver,
    ) -> Result<Self, DocumentError> {
        options.settings.validate()?;
        Self::from_input(
            HtmlInput {
                tree: DocumentTree::parse(html),
                encoding: "UTF-8".into(),
                diagnostics: Vec::new(),
            },
            options,
            resolver,
            MAX_FORMATTING_WORK,
        )
    }

    /// A nested navigable's document, already decoded from `encoding`.
    pub(crate) fn parse_decoded(
        html: &str,
        encoding: &str,
        options: &ParseOptions<'_>,
        resolver: &dyn ResourceResolver,
    ) -> Result<Self, DocumentError> {
        options.settings.validate()?;
        Self::from_input(
            HtmlInput {
                tree: DocumentTree::parse(html),
                encoding: encoding.into(),
                diagnostics: Vec::new(),
            },
            options,
            resolver,
            MAX_FORMATTING_WORK,
        )
    }

    pub(crate) fn parse_bytes_with_settings(
        bytes: &[u8],
        base: &crate::resources::ResourceBase,
        additional: &[String],
        user: &[String],
        settings: &PrintSettings,
        resolver: &dyn ResourceResolver,
    ) -> Result<Self, DocumentError> {
        Self::parse_bytes_with_options(
            bytes,
            &ParseOptions {
                base,
                additional,
                user,
                settings,
                capture_fragment_targets: false,
                presentation: crate::style::Presentation::Paged,
                ancestors: &[],
            },
            resolver,
        )
    }

    pub(crate) fn parse_bytes_with_options(
        bytes: &[u8],
        options: &ParseOptions<'_>,
        resolver: &dyn ResourceResolver,
    ) -> Result<Self, DocumentError> {
        options.settings.validate()?;
        let (decoded, html) = encoding::decode_document(bytes, None);
        let mut diagnostics = Vec::new();
        if decoded.had_errors {
            diagnostics.push(DocumentDiagnostic::Decoding {
                base: options.base.clone(),
                reference: None,
                encoding: decoded.encoding.clone(),
            });
        }
        Self::from_input(
            HtmlInput {
                tree: html,
                encoding: decoded.encoding,
                diagnostics,
            },
            options,
            resolver,
            MAX_FORMATTING_WORK,
        )
    }

    fn from_input(
        input: HtmlInput,
        options: &ParseOptions<'_>,
        resolver: &dyn ResourceResolver,
        work_limit: usize,
    ) -> Result<Self, DocumentError> {
        let ParseOptions {
            base,
            additional,
            user,
            settings,
            capture_fragment_targets,
            presentation,
            ancestors,
        } = *options;
        let HtmlInput {
            tree: mut html,
            encoding: document_encoding,
            mut diagnostics,
        } = input;
        diagnostics.extend(
            std::mem::take(&mut html.limits)
                .into_iter()
                .map(DocumentDiagnostic::TreeLimit),
        );
        let sources = stylesheets::discover(&html, base, &document_encoding, additional, user);
        html.set_document_urls(sources.document_url.clone(), sources.base.resolve("")?);
        let base = &sources.base;
        let mut styles = Stylesheet::prepare(
            &html,
            &sources,
            resolver,
            settings,
            presentation,
            encoding::stylesheet,
        )?;
        let images = std::rc::Rc::new(std::cell::RefCell::new(
            crate::images::ImageCache::with_fonts(styles.fonts.clone()),
        ));
        let mut navigables = ancestors.to_vec();
        navigables.push(sources.document_url.clone());
        let (embedded, embedding) = embedded::resolve(
            &mut html,
            &embedded::Embedder {
                base,
                navigable: frames::Navigable {
                    settings: settings.clone(),
                    user: user.to_vec(),
                    ancestors: navigables,
                },
                resolver,
            },
            &mut images.borrow_mut(),
        )?;
        diagnostics.extend(embedding);
        let links = || {
            navigation::NavigationBuilder::new(
                &html,
                base,
                capture_fragment_targets,
                sources.document_url.clone(),
                options.base,
            )
        };
        let formatting::Formatted {
            root,
            footnotes,
            navigation: links,
            strings,
            generated,
            canvas,
            pages,
            svg_definitions,
            page_counters,
            scroll_markers,
            skipped_work,
            diagnostics: formatting_diagnostics,
            ..
        } = formatting::build(
            &formatting::Input {
                html: &html,
                embedded: &embedded,
                images: &images,
                elements: &ElementMeasurements::default(),
                definitions: &svg::SvgDefinitions::default(),
                work_limit,
            },
            &mut styles,
            (base, resolver),
            &links,
            Default::default(),
        )?;
        record_skipped_work(&mut diagnostics, skipped_work);
        diagnostics.extend(formatting_diagnostics);
        diagnostics.extend(
            styles
                .take_source_diagnostics()
                .into_iter()
                .map(|diagnostic| match diagnostic {
                    SourceDiagnostic::Decoding {
                        base,
                        reference,
                        encoding,
                    } => DocumentDiagnostic::Decoding {
                        base,
                        reference: Some(reference),
                        encoding,
                    },
                    SourceDiagnostic::Unavailable {
                        base,
                        reference,
                        error,
                    } => DocumentDiagnostic::UnavailableSource {
                        base,
                        reference,
                        error,
                    },
                }),
        );
        diagnostics.extend(
            styles
                .take_diagnostics()
                .into_iter()
                .map(DocumentDiagnostic::Css),
        );
        let first_page = PageContent::default().size(
            pages.first(),
            0,
            crate::style::logical::Flow::used(root.style()),
        )?;
        Ok(Self {
            root,
            footnotes,
            styles,
            diagnostics,
            navigation: links.finish(&html, &generated)?,
            running_strings: strings.finish(&html)?,
            generated,
            metadata: DocumentMetadata::from_html(&html),
            canvas,
            pages,
            svg_definitions,
            page_counters: page_counters.into_iter().collect(),
            scroll_markers,
            element_measurements: Box::default(),
            element_renderings: Box::default(),
            first_page,
            page_content: PageContent::default(),
            source: std::rc::Rc::new(FormattingSource {
                html,
                embedded,
                base: base.clone(),
                source_base: options.base.clone(),
                document_url: sources.document_url,
                capture_fragment_targets,
            }),
        })
    }

    /// `destinations` holds the page index of each destination.
    pub(crate) fn resolve_generated(
        &self,
        destinations: &[Option<usize>],
        paginated: Paginated<'_>,
    ) -> Result<Option<GeneratedValues>, DocumentError> {
        let values = self.generated.resolve(
            &self.source.html,
            &mut |owner| {
                self.navigation
                    .element_destinations
                    .get(&owner)
                    .and_then(|index| destinations.get(*index))
                    .copied()
                    .flatten()
            },
            paginated,
        )?;
        Ok((!self.generated.matches(&values)).then_some(values))
    }

    /// Creates independent formatting state for a borrowed deferred-layout request.
    #[cfg(test)]
    pub(crate) fn fork_for_layout(
        &self,
        values: GeneratedValues,
        container_sizes: std::collections::HashMap<
            crate::dom::ElementId,
            crate::style::ContainerMeasurement,
        >,
        anchor_frames: crate::style::anchor_values::AnchorFrameMap,
        resolver: &dyn ResourceResolver,
    ) -> Result<Self, DocumentError> {
        let source = self.source.clone();
        let mut styles = self.styles.fork_for_layout();
        if !self.styles.query_sizes().settled(&container_sizes) {
            styles.query_sizes_mut().measured(container_sizes);
        }
        styles.anchor_frames_mut().measured(anchor_frames);
        let formatting::Formatted {
            root,
            footnotes,
            navigation,
            strings,
            generated,
            canvas,
            pages,
            svg_definitions,
            page_counters,
            scroll_markers,
            skipped_work,
            diagnostics: formatting_diagnostics,
            ..
        } = source.build(
            &mut styles,
            values,
            (
                &self.pages.images(),
                &self.element_measurements,
                &self.svg_definitions,
            ),
            resolver,
        )?;
        let first_page = self.page_content.size(
            pages.first(),
            0,
            crate::style::logical::Flow::used(root.style()),
        )?;
        let mut diagnostics = formatting_diagnostics;
        record_skipped_work(&mut diagnostics, skipped_work);
        Ok(Self {
            root,
            footnotes,
            styles,
            diagnostics,
            navigation: navigation.finish(&source.html, &generated)?,
            running_strings: strings.finish(&source.html)?,
            generated,
            metadata: self.metadata.clone(),
            canvas,
            pages,
            svg_definitions,
            page_counters: page_counters.into_iter().collect(),
            scroll_markers,
            element_measurements: self.element_measurements.clone(),
            element_renderings: self.element_renderings.clone(),
            first_page,
            page_content: self.page_content.clone(),
            source,
        })
    }

    /// The generated values the current formatting resolved.
    pub(crate) fn generated_values(&self) -> GeneratedValues {
        self.generated.values()
    }

    /// Restyling uses the query container sizes a layout measured.
    pub(crate) fn measured_containers(
        &mut self,
        sizes: std::collections::HashMap<crate::dom::ElementId, crate::style::ContainerMeasurement>,
    ) {
        self.styles.query_sizes_mut().measured(sizes);
    }

    pub(crate) fn measured_anchors(&mut self, frames: crate::style::anchor_values::AnchorFrameMap) {
        self.styles.anchor_frames_mut().measured(frames);
    }

    /// Reformatting resolves `element()` images against what a layout measured of them, and
    /// the next layout's floats sample the `renderings` it measured.
    pub(crate) fn measured_elements(
        &mut self,
        measurements: Box<ElementMeasurements>,
        renderings: Box<crate::layout::ElementRenderings>,
    ) {
        self.element_measurements = measurements;
        self.element_renderings = renderings;
    }

    /// The renderings the last layout measured.
    pub(crate) fn element_renderings(&self) -> &crate::layout::ElementRenderings {
        &self.element_renderings
    }

    /// The `element()` images of the current formatting.
    pub(crate) fn element_images(&self) -> &element_images::ElementImages {
        self.pages.elements()
    }

    pub(crate) fn reformat(
        &mut self,
        values: GeneratedValues,
        resolver: &dyn ResourceResolver,
    ) -> Result<(), DocumentError> {
        let source = &self.source;
        let formatting::Formatted {
            root,
            footnotes,
            navigation,
            strings,
            generated,
            canvas,
            pages,
            svg_definitions,
            page_counters,
            scroll_markers,
            skipped_work,
            diagnostics,
            ..
        } = source.build(
            &mut self.styles,
            values,
            (
                &self.pages.images(),
                &self.element_measurements,
                &self.svg_definitions,
            ),
            resolver,
        )?;
        record_skipped_work(&mut self.diagnostics, skipped_work);
        self.navigation = navigation.finish(&source.html, &generated)?;
        self.running_strings = strings.finish(&source.html)?;
        self.root = root;
        self.footnotes = footnotes;
        self.generated = generated;
        self.canvas = canvas;
        self.pages = pages;
        self.size_first_page()?;
        self.svg_definitions = svg_definitions;
        self.page_counters = page_counters.into_iter().collect();
        self.scroll_markers = scroll_markers;
        for diagnostic in diagnostics {
            self.diagnose(diagnostic);
        }
        for diagnostic in self.styles.take_diagnostics() {
            if !self.diagnostics.iter().any(
                |known| matches!(known, DocumentDiagnostic::Css(known) if *known == diagnostic),
            ) {
                self.diagnostics.push(DocumentDiagnostic::Css(diagnostic));
            }
        }
        Ok(())
    }

    /// Records `diagnostic` of a formatting, unless an earlier formatting reported the limit
    /// it reports.
    fn diagnose(&mut self, diagnostic: DocumentDiagnostic) {
        let repeated = self.diagnostics.iter().any(|known| {
            matches!(
                (known, &diagnostic),
                (
                    DocumentDiagnostic::FormattingLimit { resource: known, .. },
                    DocumentDiagnostic::FormattingLimit { resource, .. },
                ) if known == resource
            )
        });
        if !repeated {
            self.diagnostics.push(diagnostic);
        }
    }

    /// Omits `target`, a formatting box of the document, and the content after it in
    /// document order; whether the document holds `target`.
    pub(crate) fn omit_from(&mut self, target: *const BlockBox) -> bool {
        std::iter::once(&mut self.root)
            .chain(self.footnotes.bodies.iter_mut().map(|body| &mut body.block))
            .any(|tree| tree.omit_from(target))
            || self
                .running_strings
                .iter_mut()
                .filter_map(|assignment| assignment.element.as_mut())
                .any(|element| element.omit_from(target))
    }

    pub(crate) fn omit_content(&mut self) {
        self.root.omit_content();
        for body in &mut self.footnotes.bodies {
            body.block.omit_content();
        }
        self.running_strings
            .iter_mut()
            .filter_map(|assignment| assignment.element.as_mut())
            .for_each(RunningElement::omit_content);
    }

    /// Retains what the layout of this document degraded.
    pub(crate) fn record_layout(
        &mut self,
        diagnostics: impl IntoIterator<Item = crate::layout::LayoutDiagnostic>,
    ) {
        for diagnostic in diagnostics.into_iter().map(DocumentDiagnostic::Layout) {
            if !self.diagnostics.iter().any(|known| {
                matches!((known, &diagnostic), (DocumentDiagnostic::Layout(known), DocumentDiagnostic::Layout(new)) if known == new)
            }) {
                self.diagnostics.push(diagnostic);
            }
        }
    }

    pub(crate) fn into_diagnostics(mut self) -> Vec<DocumentDiagnostic> {
        self.diagnostics.extend(
            self.styles
                .take_degradations()
                .into_iter()
                .map(DocumentDiagnostic::Degraded),
        );
        self.diagnostics
            .extend(self.source.embedded.take_diagnostics());
        self.diagnostics.extend(
            self.pages
                .images()
                .borrow_mut()
                .take_failures()
                .into_iter()
                .map(DocumentDiagnostic::UnavailableImage),
        );
        self.diagnostics.extend(
            self.styles
                .fonts
                .take_failures()
                .into_iter()
                .map(DocumentDiagnostic::UnavailableFont),
        );
        let limited = self.pages.images().borrow_mut().take_limited();
        for diagnostic in limited {
            self.diagnose(diagnostic);
        }
        self.diagnostics
    }

    /// The first page, sized to the content the last layout measured.
    pub(crate) fn page_box(&self) -> &PageBox {
        &self.first_page
    }

    /// The page at `index`, sized to the content the last layout measured.
    pub(crate) fn sized_page(
        &self,
        page: &std::sync::Arc<PageBox>,
        index: usize,
    ) -> Result<std::sync::Arc<PageBox>, DocumentError> {
        self.page_content.size(
            page,
            index,
            crate::style::logical::Flow::used(self.root.style()),
        )
    }

    pub(crate) fn page_content(&self) -> &PageContent {
        &self.page_content
    }

    /// Later layouts size pages to the content a layout measured.
    pub(crate) fn measured_pages(&mut self, content: PageContent) -> Result<(), DocumentError> {
        self.page_content = content;
        self.size_first_page()
    }

    fn size_first_page(&mut self) -> Result<(), DocumentError> {
        self.first_page = self.sized_page(self.pages.first(), 0)?;
        Ok(())
    }

    /// See [`page::PageProvider::margin_images`].
    pub(crate) fn margin_images(
        &self,
        output: &crate::style::GeneratedOutput,
        style: &crate::style::Style,
        resolver: &dyn ResourceResolver,
    ) -> Vec<inline::GeneratedImage> {
        self.pages.margin_images(output, style, resolver)
    }

    /// See [`page::PageProvider::margin_replacement`].
    pub(crate) fn margin_replacement(
        &self,
        image: &crate::paint::BackgroundImage,
        style: &crate::style::Style,
        resolver: &dyn ResourceResolver,
    ) -> std::sync::Arc<crate::images::Image> {
        self.pages.margin_replacement(image, style, resolver)
    }

    pub(crate) fn resolve_page(
        &self,
        request: crate::style::PageRequest,
        resolver: &dyn ResourceResolver,
    ) -> Result<std::sync::Arc<PageBox>, DocumentError> {
        self.pages
            .resolve(&self.styles, self.root.style(), request, resolver)
    }

    pub(crate) fn root(&self) -> &BlockBox {
        &self.root
    }
    pub(crate) fn root_element(&self) -> crate::dom::ElementId {
        self.source.html.root_element().id()
    }
    pub(crate) fn scroll_marker_groups(&self) -> &scroll_markers::ScrollMarkerGroups {
        &self.scroll_markers
    }
    /// Whether the scroll markers' positions styling took are `states`.
    pub(crate) fn scroll_markers_settled(&self, states: &crate::dom::ScrollMarkerStates) -> bool {
        *self.source.html.scroll_markers.borrow() == *states
    }
    /// Restyling takes the scroll markers' positions a layout determined.
    pub(crate) fn measured_scroll_markers(&self, states: crate::dom::ScrollMarkerStates) {
        *self.source.html.scroll_markers.borrow_mut() = states;
    }
    pub(crate) fn footnotes(&self) -> &Footnotes {
        &self.footnotes
    }
    pub(crate) fn styles(&self) -> &Stylesheet {
        &self.styles
    }
    pub(crate) fn navigation(&self) -> &Navigation {
        &self.navigation
    }
    /// Document-order string assignments with element values captured and page values deferred.
    pub(crate) fn running_strings(&self) -> &[RunningStringAssignment] {
        &self.running_strings
    }
    pub(crate) fn metadata(&self) -> &DocumentMetadata {
        &self.metadata
    }
    /// The document's counters where `page` starts, as its page-margin boxes see them (CSS
    /// Page 3 §6.1); none before pagination tells where pages start.
    pub(crate) fn page_counters(&self, page: usize) -> Option<&CounterSnapshot> {
        self.page_counters.get(&page)
    }
    /// The SVG filters and masks that `url()` references in the document can name.
    pub(crate) fn svg_definitions(&self) -> svg::SvgDefinitions {
        self.svg_definitions.clone()
    }
    pub(crate) fn canvas_background(&self) -> &crate::graphics::BoxDecoration {
        &self.canvas
    }
}

#[cfg(test)]
mod tests;
