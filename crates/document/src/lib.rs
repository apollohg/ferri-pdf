//! HTML loading, stylesheet discovery, and styled document construction.
mod content;
mod counters;
pub mod encoding;
mod first_page;
mod formatting;
mod generated;
mod inline;
mod lists;
mod metadata;
mod navigation;
mod page;
mod semantics;
mod strings;
mod stylesheets;
mod svg;
mod tables;

use ferri_pdf_dom::DocumentTree;
use ferri_pdf_resources::{ResourceError, ResourceResolver};
use ferri_pdf_style::{PrintSettings, SourceDiagnostic, StyleError, Stylesheet};

pub use formatting::{BlockBox, BlockContent, PageBox, PageMarginBox};
pub use generated::GeneratedValues;
pub use inline::{
    Anchor, InlineBox, InlineLink, MAX_INLINE_TEXT_BYTES, OBJECT_REPLACEMENT_CHARACTER, Paragraph,
    TextSpan, is_css_whitespace,
};
pub use metadata::DocumentMetadata;
pub use navigation::{Bookmark, BoxNavigation, FragmentTargets, Heading, LinkTarget, Navigation};
pub use page::PageBoundary;
pub use semantics::{InlineEvent, SemanticEvent};
pub use strings::RunningStringAssignment;

#[derive(Debug, thiserror::Error)]
pub enum DocumentError {
    #[error("{0}")]
    Tree(#[from] ferri_pdf_dom::DomError),
    #[error("{0}")]
    Image(#[from] ferri_pdf_images::ImageError),
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
    UnavailableImage(#[from] ferri_pdf_images::ImageFailure),
    #[error("image omitted during PDF encoding: {error}")]
    ImageDecode { error: String },
    #[error("{0}")]
    Css(ferri_pdf_style::StyleDiagnostic),
    #[error("decoding {reference:?} as {encoding} replaced malformed byte sequences")]
    Decoding {
        base: ferri_pdf_resources::ResourceBase,
        reference: Option<String>,
        encoding: String,
    },
    #[error("stylesheet {reference} is unavailable: {error}")]
    UnavailableSource {
        base: ferri_pdf_resources::ResourceBase,
        reference: String,
        #[source]
        error: ResourceError,
    },
}

/// An owned document whose text and style ranges have been resolved together.
pub struct StyledDocument {
    root: BlockBox,
    styles: Stylesheet,
    diagnostics: Vec<DocumentDiagnostic>,
    navigation: Navigation,
    running_strings: Vec<RunningStringAssignment>,
    generated: generated::GeneratedReferences,
    source: std::rc::Rc<FormattingSource>,
    metadata: DocumentMetadata,
    canvas: ferri_pdf_graphics::BoxDecoration,
    pages: page::PageProvider,
}

struct FormattingSource {
    html: DocumentTree,
    base: ferri_pdf_resources::ResourceBase,
    source_base: ferri_pdf_resources::ResourceBase,
    document_url: Option<ferri_pdf_resources::Url>,
    capture_fragment_targets: bool,
}

impl FormattingSource {
    fn build(
        &self,
        styles: &mut Stylesheet,
        values: GeneratedValues,
        images: std::rc::Rc<std::cell::RefCell<ferri_pdf_images::ImageCache>>,
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
            &self.html,
            styles,
            &self.base,
            resolver,
            navigation,
            values,
            Some(images),
        )
    }
}

/// Borrowed inputs shared by string and byte document parsing.
#[derive(Clone, Copy)]
pub struct ParseOptions<'a> {
    pub base: &'a ferri_pdf_resources::ResourceBase,
    pub additional: &'a [String],
    pub user: &'a [String],
    pub settings: &'a PrintSettings,
    /// Retain targets and unresolved fragment intents for a multipart rendering job.
    pub capture_fragment_targets: bool,
}

struct HtmlInput {
    tree: DocumentTree,
    encoding: String,
    diagnostics: Vec<DocumentDiagnostic>,
}

impl StyledDocument {
    pub fn parse(
        html: &str,
        base: &ferri_pdf_resources::ResourceBase,
        additional: &[String],
        resolver: &dyn ResourceResolver,
    ) -> Result<Self, DocumentError> {
        Self::parse_with_origins(html, base, additional, &[], resolver)
    }

    pub fn parse_with_origins(
        html: &str,
        base: &ferri_pdf_resources::ResourceBase,
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

    pub fn parse_with_settings(
        html: &str,
        base: &ferri_pdf_resources::ResourceBase,
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
            },
            resolver,
        )
    }

    pub fn parse_with_options(
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

    pub fn parse_bytes(
        bytes: &[u8],
        base: &ferri_pdf_resources::ResourceBase,
        additional: &[String],
        resolver: &dyn ResourceResolver,
    ) -> Result<Self, DocumentError> {
        Self::parse_bytes_with_origins(bytes, base, additional, &[], resolver)
    }

    pub fn parse_bytes_with_origins(
        bytes: &[u8],
        base: &ferri_pdf_resources::ResourceBase,
        additional: &[String],
        user: &[String],
        resolver: &dyn ResourceResolver,
    ) -> Result<Self, DocumentError> {
        Self::parse_bytes_with_settings(
            bytes,
            base,
            additional,
            user,
            &PrintSettings::default(),
            resolver,
        )
    }

    pub fn parse_bytes_with_settings(
        bytes: &[u8],
        base: &ferri_pdf_resources::ResourceBase,
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
            },
            resolver,
        )
    }

    pub fn parse_bytes_with_options(
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
        } = *options;
        let HtmlInput {
            tree: mut html,
            encoding: document_encoding,
            mut diagnostics,
        } = input;
        let sources = stylesheets::discover(&html, base, &document_encoding, additional, user)?;
        html.set_document_urls(sources.document_url.clone(), sources.base.resolve("")?);
        let base = &sources.base;
        let mut styles =
            Stylesheet::prepare(&html, &sources, resolver, settings, encoding::stylesheet)?;
        let links = navigation::NavigationBuilder::new(
            &html,
            base,
            capture_fragment_targets,
            sources.document_url.clone(),
            options.base,
        )?;
        let formatting::Formatted {
            root,
            navigation: links,
            strings,
            generated,
            canvas,
            pages,
        } = formatting::build(
            &html,
            &mut styles,
            base,
            resolver,
            links,
            Default::default(),
            None,
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
            styles,
            diagnostics,
            navigation: links.finish(&html, &generated)?,
            running_strings: strings.finish(&html)?,
            generated,
            metadata: DocumentMetadata::from_html(&html),
            canvas,
            pages,
            source: std::rc::Rc::new(FormattingSource {
                html,
                base: base.clone(),
                source_base: options.base.clone(),
                document_url: sources.document_url,
                capture_fragment_targets,
            }),
        })
    }

    pub fn resolve_generated(
        &self,
        pages: usize,
        destinations: &[Option<usize>],
        strings: &mut dyn FnMut(
            ferri_pdf_style::PageStyleTarget,
            &ferri_pdf_style::NamedStringRequest,
        ) -> Result<String, StyleError>,
    ) -> Result<Option<GeneratedValues>, DocumentError> {
        let values = self.generated.resolve(
            &self.source.html,
            pages,
            &mut |owner| {
                self.navigation
                    .element_destinations
                    .get(&owner)
                    .and_then(|index| destinations.get(*index))
                    .copied()
                    .flatten()
            },
            strings,
        )?;
        Ok((!self.generated.matches(&values)).then_some(values))
    }

    /// Creates independent formatting state for a borrowed deferred-layout request.
    pub fn fork_for_layout(
        &self,
        values: GeneratedValues,
        resolver: &dyn ResourceResolver,
    ) -> Result<Self, DocumentError> {
        let source = self.source.clone();
        let mut styles = self.styles.fork_for_layout();
        let formatting::Formatted {
            root,
            navigation,
            strings,
            generated,
            canvas,
            pages,
        } = source.build(&mut styles, values, self.pages.images(), resolver)?;
        Ok(Self {
            root,
            styles,
            diagnostics: Vec::new(),
            navigation: navigation.finish(&source.html, &generated)?,
            running_strings: strings.finish(&source.html)?,
            generated,
            metadata: self.metadata.clone(),
            canvas,
            pages,
            source,
        })
    }

    pub fn reformat(
        &mut self,
        values: GeneratedValues,
        resolver: &dyn ResourceResolver,
    ) -> Result<(), DocumentError> {
        let source = &self.source;
        let formatting::Formatted {
            root,
            navigation,
            strings,
            generated,
            canvas,
            pages,
        } = source.build(&mut self.styles, values, self.pages.images(), resolver)?;
        self.navigation = navigation.finish(&source.html, &generated)?;
        self.running_strings = strings.finish(&source.html)?;
        self.root = root;
        self.generated = generated;
        self.canvas = canvas;
        self.pages = pages;
        for diagnostic in self.styles.take_diagnostics() {
            if !self.diagnostics.iter().any(
                |known| matches!(known, DocumentDiagnostic::Css(known) if *known == diagnostic),
            ) {
                self.diagnostics.push(DocumentDiagnostic::Css(diagnostic));
            }
        }
        Ok(())
    }

    pub fn into_diagnostics(mut self) -> Vec<DocumentDiagnostic> {
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

    pub fn page_box(&self) -> &PageBox {
        self.pages.first()
    }

    pub fn resolve_page(
        &self,
        request: ferri_pdf_style::PageRequest,
        resolver: &dyn ResourceResolver,
    ) -> Result<std::sync::Arc<PageBox>, DocumentError> {
        self.pages
            .resolve(&self.styles, self.root.style(), request, resolver)
    }

    pub fn root(&self) -> &BlockBox {
        &self.root
    }
    pub fn styles(&self) -> &Stylesheet {
        &self.styles
    }
    pub fn navigation(&self) -> &Navigation {
        &self.navigation
    }
    /// Document-order string assignments with element values captured and page values deferred.
    pub fn running_strings(&self) -> &[RunningStringAssignment] {
        &self.running_strings
    }
    pub fn metadata(&self) -> &DocumentMetadata {
        &self.metadata
    }
    pub fn canvas_background(&self) -> &ferri_pdf_graphics::BoxDecoration {
        &self.canvas
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferri_pdf_resources::FileResolver;

    fn paragraphs<'a>(block: &'a BlockBox, output: &mut Vec<&'a Paragraph>) {
        for child in block.children() {
            match child {
                BlockContent::Events(_) => {}
                BlockContent::Block(child) | BlockContent::Positioned(child) => {
                    paragraphs(child, output);
                }
                BlockContent::Inline(paragraph) => output.push(paragraph),
            }
        }
    }

    #[test]
    fn html_newline_normalization_does_not_reinterpret_preserved_character_references() {
        let document = StyledDocument::parse(
            "<pre>A&#13;B\rC</pre>",
            &ferri_pdf_resources::ResourceBase::default(),
            &[],
            &FileResolver::default(),
        )
        .unwrap();
        let mut content = Vec::new();
        paragraphs(document.root(), &mut content);
        assert_eq!(content.len(), 1);
        assert_eq!(
            content[0].text(),
            "A\rB\nC",
            "literal CR is normalized by HTML parsing; an entity-produced control remains a font-coverage decision, not an invented forced line break"
        );
    }
}
