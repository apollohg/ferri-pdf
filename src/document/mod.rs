//! HTML loading, stylesheet discovery, and styled document construction.
mod content;
mod controls;
mod counters;
mod element_images;
mod embedded;
pub(crate) mod encoding;
mod first_letter;
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
    BlockBox, BlockContent, InitialLetterBox, PageBox, PageFootnoteArea, PageMarginBox,
};
pub(crate) use frames::{Frame, FrameDocument};
pub(crate) use generated::{GeneratedValues, PageStart, Paginated};
pub(crate) use inline::{
    Anchor, InlineBox, MAX_INLINE_TEXT_BYTES, OBJECT_REPLACEMENT_CHARACTER, OffsetMap, Paragraph,
    is_css_whitespace, narrow_width,
};
pub(crate) use metadata::DocumentMetadata;
pub(crate) use navigation::{Bookmark, BoxNavigation, Heading, LinkTarget, Navigation};
pub(crate) use page::PageBoundary;
pub(crate) use semantics::{InlineEvent, SemanticEvent};
pub(crate) use strings::{RunningElement, RunningStringAssignment};

#[derive(Debug, thiserror::Error)]
pub enum DocumentError {
    #[error("{0}")]
    Tree(#[from] crate::dom::DomError),
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
    #[error("stylesheet {reference} is unavailable: {error}")]
    UnavailableSource {
        base: crate::resources::ResourceBase,
        reference: String,
        #[source]
        error: ResourceError,
    },
}

/// An owned document whose text and style ranges have been resolved together.
pub(crate) struct StyledDocument {
    root: BlockBox,
    /// Footnote element boxes in document order (CSS GCPM 3 §2.2).
    footnotes: Vec<BlockBox>,
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
        let navigation = navigation::NavigationBuilder::new(
            &self.html,
            &self.base,
            self.capture_fragment_targets,
            self.document_url.clone(),
            &self.source_base,
        )?;
        formatting::build(
            &formatting::Input {
                html: &self.html,
                embedded: &self.embedded,
                images,
                elements,
                definitions,
            },
            styles,
            &self.base,
            resolver,
            navigation,
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
                tree: DocumentTree::parse(html)?,
                encoding: "UTF-8".into(),
                diagnostics: Vec::new(),
            },
            options,
            resolver,
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
                tree: DocumentTree::parse(html)?,
                encoding: encoding.into(),
                diagnostics: Vec::new(),
            },
            options,
            resolver,
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
        let (decoded, html) = encoding::decode_document(bytes, None)?;
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
        )
    }

    fn from_input(
        input: HtmlInput,
        options: &ParseOptions<'_>,
        resolver: &dyn ResourceResolver,
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
        let sources = stylesheets::discover(&html, base, &document_encoding, additional, user)?;
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
        let links = navigation::NavigationBuilder::new(
            &html,
            base,
            capture_fragment_targets,
            sources.document_url.clone(),
            options.base,
        )?;
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
        } = formatting::build(
            &formatting::Input {
                html: &html,
                embedded: &embedded,
                images: &images,
                elements: &ElementMeasurements::default(),
                definitions: &svg::SvgDefinitions::default(),
            },
            &mut styles,
            base,
            resolver,
            links,
            Default::default(),
        )?;
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
            element_measurements: Box::default(),
            element_renderings: Box::default(),
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
        container_sizes: std::collections::HashMap<crate::dom::ElementId, [f32; 2]>,
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
        Ok(Self {
            root,
            footnotes,
            styles,
            diagnostics: Vec::new(),
            navigation: navigation.finish(&source.html, &generated)?,
            running_strings: strings.finish(&source.html)?,
            generated,
            metadata: self.metadata.clone(),
            canvas,
            pages,
            svg_definitions,
            page_counters: page_counters.into_iter().collect(),
            element_measurements: self.element_measurements.clone(),
            element_renderings: self.element_renderings.clone(),
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
        sizes: std::collections::HashMap<crate::dom::ElementId, [f32; 2]>,
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
        self.navigation = navigation.finish(&source.html, &generated)?;
        self.running_strings = strings.finish(&source.html)?;
        self.root = root;
        self.footnotes = footnotes;
        self.generated = generated;
        self.canvas = canvas;
        self.pages = pages;
        self.svg_definitions = svg_definitions;
        self.page_counters = page_counters.into_iter().collect();
        for diagnostic in self.styles.take_diagnostics() {
            if !self.diagnostics.iter().any(
                |known| matches!(known, DocumentDiagnostic::Css(known) if *known == diagnostic),
            ) {
                self.diagnostics.push(DocumentDiagnostic::Css(diagnostic));
            }
        }
        Ok(())
    }

    pub(crate) fn into_diagnostics(mut self) -> Vec<DocumentDiagnostic> {
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
        self.diagnostics
    }

    pub(crate) fn page_box(&self) -> &PageBox {
        self.pages.first()
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
    pub(crate) fn footnotes(&self) -> &[BlockBox] {
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
