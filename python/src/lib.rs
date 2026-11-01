//! Python bindings: `ferri_pdf.render` and `ferri_pdf.render_parts` over the Rust library.

use pyo3::create_exception;
use pyo3::exceptions::PyException;

create_exception!(
    ferri_pdf,
    RenderError,
    PyException,
    "Rendering failed; invalid markup and missing resources never raise it."
);
create_exception!(
    ferri_pdf,
    ContentLostError,
    RenderError,
    "A resource limit left content out of the PDF; `diagnostics` lists the losses and `part` the one-based part, if any."
);

mod resolver;

#[pyo3::pymodule]
#[pyo3(name = "ferri_pdf")]
mod ferri_pdf_module {
    use std::path::PathBuf;

    use ferri_pdf::{
        ColorScheme, ContentLoss, ContrastPreference, Error, FontPreferences, HeadingLocation,
        HtmlPart, ImageDpi, ImageOptions, JpegQuality, LayoutTrace, MotionPreference,
        PrintPreferences, PrintSettings, RenderDiagnostic, ResourceBase, ResourceResolver, Url,
    };
    use pyo3::exceptions::{PyTypeError, PyValueError};
    use pyo3::prelude::*;
    use pyo3::types::{PyBytes, PyString, PyTuple};

    use super::resolver::Resolver;
    #[pymodule_export]
    use super::resolver::Resource;
    #[pymodule_export]
    use super::{ContentLostError, RenderError};

    const LIGHT: &str = "light";
    const DARK: &str = "dark";
    const NO_PREFERENCE: &str = "no-preference";
    const LESS: &str = "less";
    const MORE: &str = "more";
    const CUSTOM: &str = "custom";
    const REDUCE: &str = "reduce";
    const DIAGNOSTICS_ATTRIBUTE: &str = "diagnostics";
    const PART_ATTRIBUTE: &str = "part";
    /// Matches the Rust library, whose default resolver fetches HTTP(S) resources.
    const ALLOW_HTTP: bool = true;

    #[pymodule_init]
    fn init(module: &Bound<'_, PyModule>) -> PyResult<()> {
        module.add("__version__", env!("CARGO_PKG_VERSION"))
    }

    #[derive(Clone)]
    enum Html {
        Text(String),
        Bytes(Vec<u8>),
    }
    impl Html {
        fn extract(html: &Bound<'_, PyAny>) -> PyResult<Self> {
            if let Ok(text) = html.cast::<PyString>() {
                Ok(Self::Text(text.to_str()?.to_owned()))
            } else if let Ok(bytes) = html.cast::<PyBytes>() {
                Ok(Self::Bytes(bytes.as_bytes().to_vec()))
            } else {
                Err(PyTypeError::new_err(format!(
                    "HTML must be str or bytes, not {}",
                    html.get_type().name()?
                )))
            }
        }
        fn part(&self) -> HtmlPart<'_> {
            match self {
                Self::Text(text) => HtmlPart::from_html(text),
                Self::Bytes(bytes) => HtmlPart::from_bytes(bytes),
            }
        }
    }

    fn resource_base(
        base: Option<PathBuf>,
        base_url: Option<&str>,
    ) -> PyResult<Option<ResourceBase>> {
        match (base, base_url) {
            (Some(_), Some(_)) => Err(PyValueError::new_err(
                "pass either base or base_url, not both",
            )),
            (Some(directory), None) => Ok(Some(directory.into())),
            (None, Some(url)) => Url::parse(url)
                .map(|url| Some(ResourceBase::Url(url)))
                .map_err(|error| PyValueError::new_err(format!("base_url {url:?}: {error}"))),
            (None, None) => Ok(None),
        }
    }

    fn parse_color_scheme(value: &str) -> PyResult<ColorScheme> {
        match value {
            LIGHT => Ok(ColorScheme::Light),
            DARK => Ok(ColorScheme::Dark),
            _ => Err(PyValueError::new_err(format!(
                "color_scheme must be {LIGHT:?} or {DARK:?}, not {value:?}"
            ))),
        }
    }

    fn parse_contrast(value: &str) -> PyResult<ContrastPreference> {
        match value {
            NO_PREFERENCE => Ok(ContrastPreference::NoPreference),
            LESS => Ok(ContrastPreference::Less),
            MORE => Ok(ContrastPreference::More),
            CUSTOM => Ok(ContrastPreference::Custom),
            _ => Err(PyValueError::new_err(format!(
                "contrast must be {NO_PREFERENCE:?}, {LESS:?}, {MORE:?} or {CUSTOM:?}, not {value:?}"
            ))),
        }
    }

    fn parse_reduced_motion(value: &str) -> PyResult<MotionPreference> {
        match value {
            NO_PREFERENCE => Ok(MotionPreference::NoPreference),
            REDUCE => Ok(MotionPreference::Reduce),
            _ => Err(PyValueError::new_err(format!(
                "reduced_motion must be {NO_PREFERENCE:?} or {REDUCE:?}, not {value:?}"
            ))),
        }
    }

    fn parse_jpeg_quality(value: i64) -> PyResult<JpegQuality> {
        u8::try_from(value)
            .ok()
            .and_then(|value| JpegQuality::new(value).ok())
            .ok_or_else(|| {
                PyValueError::new_err(format!(
                    "jpeg_quality must be from {} to {}, not {value}",
                    JpegQuality::MIN,
                    JpegQuality::MAX
                ))
            })
    }

    /// Settings shared by every render that uses them; CSS controls page size and formatting.
    #[pyclass(frozen)]
    struct RenderOptions {
        options: ferri_pdf::RenderOptions,
        allow_http: bool,
        file_root: Option<PathBuf>,
        fetch: Option<Py<PyAny>>,
    }

    impl RenderOptions {
        fn resolver(&self, py: Python<'_>) -> Resolver {
            Resolver::new(
                self.allow_http,
                self.file_root.clone(),
                self.fetch.as_ref().map(|fetch| fetch.clone_ref(py)),
            )
        }
    }

    /// Run `render` without the interpreter lock, then raise what stopped a `fetch` callback
    /// in preference to the render's own result.
    fn detached<T: Send>(
        py: Python<'_>,
        resolver: Resolver,
        render: impl FnOnce(&dyn ResourceResolver) -> Result<T, Error> + Send,
    ) -> PyResult<T> {
        let (rendered, fatal) = py.detach(|| {
            let rendered = render(resolver.as_dyn());
            (rendered, resolver.into_fatal())
        });
        if let Some(error) = fatal {
            return Err(error);
        }
        rendered.map_err(|error| render_error(py, error))
    }

    impl Default for RenderOptions {
        fn default() -> Self {
            Self {
                options: ferri_pdf::RenderOptions::default(),
                allow_http: ALLOW_HTTP,
                file_root: None,
                fetch: None,
            }
        }
    }

    #[pymethods]
    impl RenderOptions {
        #[new]
        #[pyo3(signature = (
            *,
            base = None,
            base_url = None,
            stylesheets = Vec::new(),
            user_stylesheets = Vec::new(),
            paper = None,
            color_scheme = LIGHT,
            contrast = NO_PREFERENCE,
            reduced_motion = NO_PREFERENCE,
            serif = Vec::new(),
            sans_serif = Vec::new(),
            monospace = Vec::new(),
            fallback = Vec::new(),
            system_fonts = false,
            font_dirs = Vec::new(),
            image_dpi = None,
            jpeg_quality = None,
            include_layout = false,
            allow_content_loss = false,
            allow_http = ALLOW_HTTP,
            file_root = None,
            fetch = None,
        ))]
        #[expect(
            clippy::too_many_arguments,
            clippy::fn_params_excessive_bools,
            reason = "Python keyword arguments mirror the Rust RenderOptions fields"
        )]
        fn new(
            base: Option<PathBuf>,
            base_url: Option<&str>,
            stylesheets: Vec<String>,
            user_stylesheets: Vec<String>,
            paper: Option<(f32, f32)>,
            color_scheme: &str,
            contrast: &str,
            reduced_motion: &str,
            serif: Vec<String>,
            sans_serif: Vec<String>,
            monospace: Vec<String>,
            fallback: Vec<String>,
            system_fonts: bool,
            font_dirs: Vec<PathBuf>,
            image_dpi: Option<f32>,
            jpeg_quality: Option<i64>,
            include_layout: bool,
            allow_content_loss: bool,
            allow_http: bool,
            file_root: Option<PathBuf>,
            fetch: Option<Bound<'_, PyAny>>,
        ) -> PyResult<Self> {
            if let Some(fetch) = fetch.as_ref().filter(|fetch| !fetch.is_callable()) {
                return Err(PyTypeError::new_err(format!(
                    "fetch must be callable, not {}",
                    fetch.get_type().name()?
                )));
            }
            let mut print = PrintSettings {
                preferences: PrintPreferences {
                    color_scheme: parse_color_scheme(color_scheme)?,
                    contrast: parse_contrast(contrast)?,
                    reduced_motion: parse_reduced_motion(reduced_motion)?,
                },
                fonts: FontPreferences {
                    sans_serif,
                    serif,
                    monospace,
                    fallback,
                    system_fonts,
                    font_dirs,
                },
                ..PrintSettings::default()
            };
            if let Some((width, height)) = paper {
                print.paper = [width, height];
            }
            print
                .validate()
                .map_err(|error| PyValueError::new_err(error.to_string()))?;
            Ok(Self {
                options: ferri_pdf::RenderOptions {
                    base: resource_base(base, base_url)?.unwrap_or_default(),
                    stylesheets,
                    user_stylesheets,
                    print,
                    include_layout,
                    images: ImageOptions {
                        jpeg_quality: jpeg_quality.map(parse_jpeg_quality).transpose()?,
                        dpi: image_dpi.map(ImageDpi::new).transpose().map_err(|error| {
                            PyValueError::new_err(format!("image_dpi: {error}"))
                        })?,
                    },
                    content_loss: if allow_content_loss {
                        ContentLoss::Render
                    } else {
                        ContentLoss::Fail
                    },
                },
                allow_http,
                file_root,
                fetch: fetch.map(Bound::unbind),
            })
        }
    }

    /// One document of a multipart render, with its own resource base.
    #[pyclass(frozen)]
    struct Part {
        html: Html,
        base: Option<ResourceBase>,
    }

    #[pymethods]
    impl Part {
        #[new]
        #[pyo3(signature = (html, *, base = None, base_url = None))]
        fn new(
            html: &Bound<'_, PyAny>,
            base: Option<PathBuf>,
            base_url: Option<&str>,
        ) -> PyResult<Self> {
            Ok(Self {
                html: Html::extract(html)?,
                base: resource_base(base, base_url)?,
            })
        }
    }

    /// A recovered input problem, or with `loses_content`, content a resource limit left out.
    #[pyclass(frozen, get_all, str = "{message}")]
    struct Diagnostic {
        message: String,
        loses_content: bool,
    }

    #[pymethods]
    impl Diagnostic {
        fn __repr__(&self) -> String {
            format!(
                "Diagnostic(message={:?}, loses_content={})",
                self.message,
                if self.loses_content { "True" } else { "False" }
            )
        }
    }

    /// A located HTML heading; `page` is one-based and `top` is in points from the page top.
    #[pyclass(frozen, get_all)]
    struct Heading {
        id: Option<String>,
        level: u8,
        text: String,
        page: usize,
        top: f32,
    }

    #[pymethods]
    impl Heading {
        fn __repr__(&self) -> String {
            format!(
                "Heading(level={}, text={:?}, page={})",
                self.level, self.text, self.page
            )
        }
    }

    /// A completed render: the PDF bytes, located headings and recovered-input diagnostics.
    #[pyclass(frozen, get_all)]
    struct RenderOutput {
        pdf: Py<PyBytes>,
        page_count: usize,
        headings: Py<PyTuple>,
        diagnostics: Py<PyTuple>,
        layout: Option<Py<PyAny>>,
    }

    /// One part of a multipart render; heading pages count from the start of the combined PDF.
    #[pyclass(frozen, get_all)]
    struct PartOutput {
        first_page: usize,
        page_count: usize,
        headings: Py<PyTuple>,
        diagnostics: Py<PyTuple>,
        layout: Option<Py<PyAny>>,
    }

    /// A completed multipart render: the combined PDF and each part's results in input order.
    #[pyclass(frozen, get_all)]
    struct MultipartOutput {
        pdf: Py<PyBytes>,
        page_count: usize,
        parts: Py<PyTuple>,
    }

    fn diagnostics(py: Python<'_>, diagnostics: Vec<RenderDiagnostic>) -> PyResult<Py<PyTuple>> {
        PyTuple::new(
            py,
            diagnostics.into_iter().map(|diagnostic| Diagnostic {
                loses_content: diagnostic.loses_content(),
                message: diagnostic.to_string(),
            }),
        )
        .map(Bound::unbind)
    }

    fn headings(py: Python<'_>, headings: Vec<HeadingLocation>) -> PyResult<Py<PyTuple>> {
        PyTuple::new(
            py,
            headings.into_iter().map(|heading| Heading {
                id: heading.id,
                level: heading.level,
                text: heading.text,
                page: heading.page,
                top: heading.top,
            }),
        )
        .map(Bound::unbind)
    }

    fn layout(py: Python<'_>, layout: Option<&LayoutTrace>) -> PyResult<Option<Py<PyAny>>> {
        layout
            .map(|trace| {
                let json = serde_json::to_string(trace).map_err(|error| {
                    RenderError::new_err(format!("layout serialization: {error}"))
                })?;
                Ok(py.import("json")?.call_method1("loads", (json,))?.unbind())
            })
            .transpose()
    }

    fn render_error(py: Python<'_>, error: Error) -> PyErr {
        let message = error.to_string();
        let (part, lost) = match error {
            Error::ContentLost { diagnostics } => (None, diagnostics),
            Error::Part { part, source } => match *source {
                Error::ContentLost { diagnostics } => (Some(part), diagnostics),
                _ => return RenderError::new_err(message),
            },
            _ => return RenderError::new_err(message),
        };
        let error = ContentLostError::new_err(message);
        let attached = diagnostics(py, lost).and_then(|lost| {
            let value = error.value(py);
            value.setattr(DIAGNOSTICS_ATTRIBUTE, lost)?;
            value.setattr(PART_ATTRIBUTE, part)
        });
        match attached {
            Ok(()) => error,
            Err(attach_error) => attach_error,
        }
    }

    /// Render one HTML document (str, or bytes decoded by the HTML encoding rules) to PDF.
    #[pyfunction]
    #[pyo3(signature = (html, options = None))]
    fn render(
        py: Python<'_>,
        html: &Bound<'_, PyAny>,
        options: Option<&Bound<'_, RenderOptions>>,
    ) -> PyResult<RenderOutput> {
        let html = Html::extract(html)?;
        let defaults = RenderOptions::default();
        let options = options.map_or(&defaults, Bound::get);
        let rendered = detached(py, options.resolver(py), |resolver| match &html {
            Html::Text(text) => ferri_pdf::render_with_resolver(text, &options.options, resolver),
            Html::Bytes(bytes) => {
                ferri_pdf::render_bytes_with_resolver(bytes, &options.options, resolver)
            }
        })?;
        Ok(RenderOutput {
            pdf: PyBytes::new(py, &rendered.pdf).unbind(),
            page_count: rendered.page_count,
            layout: layout(py, rendered.layout.as_ref())?,
            headings: headings(py, rendered.headings)?,
            diagnostics: diagnostics(py, rendered.diagnostics)?,
        })
    }

    /// Render independent documents into one PDF; each part (str, bytes or Part) starts a page.
    #[pyfunction]
    #[pyo3(signature = (parts, options = None))]
    fn render_parts(
        py: Python<'_>,
        parts: &Bound<'_, PyAny>,
        options: Option<&Bound<'_, RenderOptions>>,
    ) -> PyResult<MultipartOutput> {
        let parts = parts
            .try_iter()?
            .map(|item| {
                let item = item?;
                match item.cast::<Part>() {
                    Ok(part) => Ok((part.get().html.clone(), part.get().base.clone())),
                    Err(_) => Html::extract(&item).map(|html| (html, None)),
                }
            })
            .collect::<PyResult<Vec<_>>>()?;
        let defaults = RenderOptions::default();
        let options = options.map_or(&defaults, Bound::get);
        let rendered = detached(py, options.resolver(py), |resolver| {
            let parts: Vec<_> = parts
                .iter()
                .map(|(html, base)| match base {
                    Some(base) => html.part().with_base(base.clone()),
                    None => html.part(),
                })
                .collect();
            ferri_pdf::render_parts_with_resolver(&parts, &options.options, resolver)
        })?;
        let outputs = rendered
            .parts
            .into_iter()
            .map(|part| {
                Ok(PartOutput {
                    first_page: part.first_page,
                    page_count: part.page_count,
                    layout: layout(py, part.layout.as_ref())?,
                    headings: headings(py, part.headings)?,
                    diagnostics: diagnostics(py, part.diagnostics)?,
                })
            })
            .collect::<PyResult<Vec<_>>>()?;
        Ok(MultipartOutput {
            pdf: PyBytes::new(py, &rendered.pdf).unbind(),
            page_count: rendered.page_count,
            parts: PyTuple::new(py, outputs)?.unbind(),
        })
    }
}
