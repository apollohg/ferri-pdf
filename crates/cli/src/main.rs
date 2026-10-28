#![expect(
    clippy::print_stderr,
    reason = "the command line reports warnings, progress and errors on stderr"
)]
use ferri_pdf::{
    FontPreferences, HtmlPart, ImageDpi, ImageOptions, JpegQuality, RenderOptions, Url,
    render_bytes, render_parts,
};
use std::io::{self, Read, Write};
use std::{
    env, fs,
    path::{Path, PathBuf},
    process::ExitCode,
};

const USAGE: &str = "usage: ferri-pdf INPUT.html OUTPUT.pdf [--trace] [--headings HEADINGS.json] [--font-serif FAMILY] [--font-sans-serif FAMILY] [--font-monospace FAMILY] [--font-fallback FAMILY] [--jpeg-quality 1..100] [--image-dpi DPI] [--part INPUT.html ...]";
const JPEG_QUALITY_FLAG: &str = "--jpeg-quality";
const IMAGE_DPI_FLAG: &str = "--image-dpi";
const PART_FLAG: &str = "--part";
const FONT_SERIF_FLAG: &str = "--font-serif";
const FONT_SANS_SERIF_FLAG: &str = "--font-sans-serif";
const FONT_MONOSPACE_FLAG: &str = "--font-monospace";
const FONT_FALLBACK_FLAG: &str = "--font-fallback";
const FIRST_FLAG_ARGUMENT: usize = 3;
const TRACE_EXTENSION: &str = "trace.json";

struct FileIdentity {
    path: PathBuf,
    existing: Option<same_file::Handle>,
}
impl FileIdentity {
    fn new(path: &Path) -> io::Result<Self> {
        let path = resolved_path(path)?;
        let existing = match same_file::Handle::from_path(&path) {
            Ok(handle) => Some(handle),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(error),
        };
        Ok(Self { path, existing })
    }
    fn aliases(&self, other: &Self) -> bool {
        self.path == other.path
            || matches!((&self.existing, &other.existing), (Some(first), Some(second)) if first == second)
    }
}

fn resolved_path(path: &Path) -> io::Result<PathBuf> {
    match fs::canonicalize(path) {
        Ok(path) => Ok(path),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let parent = path
                .parent()
                .filter(|path| !path.as_os_str().is_empty())
                .unwrap_or(Path::new("."));
            match fs::read_link(path) {
                Ok(target) => resolved_path(&parent.join(target)),
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::NotFound | io::ErrorKind::InvalidInput
                    ) =>
                {
                    let name = path.file_name().ok_or_else(|| {
                        io::Error::new(io::ErrorKind::InvalidInput, "output must name a file")
                    })?;
                    Ok(fs::canonicalize(parent)?.join(name))
                }
                Err(error) => Err(error),
            }
        }
        Err(error) => Err(error),
    }
}

fn validate_output_paths(inputs: &[&Path], outputs: &[(&str, Option<&Path>)]) -> io::Result<()> {
    let inputs = inputs
        .iter()
        .map(|path| Ok((*path, FileIdentity::new(path)?)))
        .collect::<io::Result<Vec<_>>>()?;
    let mut destinations: Vec<(&str, &Path, FileIdentity)> = Vec::new();
    for &(role, path) in outputs {
        if let Some(path) = path {
            let destination = FileIdentity::new(path)?;
            for (other_role, other_path, other) in &destinations {
                if destination.aliases(other) {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!(
                            "{role} {} aliases {other_role} {}",
                            path.display(),
                            other_path.display()
                        ),
                    ));
                }
            }
            for (input_path, input) in &inputs {
                if destination.aliases(input) {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!(
                            "{role} {} aliases input HTML {}",
                            path.display(),
                            input_path.display()
                        ),
                    ));
                }
            }
            destinations.push((role, path, destination));
        }
    }
    Ok(())
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = env::args_os().collect();
    if args.len() < FIRST_FLAG_ARGUMENT {
        return Err(USAGE.into());
    }
    let mut part_paths = Vec::new();
    let mut trace = false;
    let mut headings = None;
    let mut fonts = FontPreferences::default();
    let mut images = ImageOptions::default();
    let mut flags = args[FIRST_FLAG_ARGUMENT..].iter();
    while let Some(flag) = flags.next() {
        if flag == "--trace" && !trace {
            trace = true;
        } else if flag == PART_FLAG {
            let path = Path::new(flags.next().ok_or(USAGE)?);
            if path == Path::new("-") {
                return Err(
                    "--part requires a filename; stdin is available for the first input".into(),
                );
            }
            part_paths.push(path);
        } else if flag == "--headings" && headings.is_none() {
            headings = Some(Path::new(flags.next().ok_or(USAGE)?));
        } else if flag == JPEG_QUALITY_FLAG && images.jpeg_quality.is_none() {
            let quality = flags
                .next()
                .ok_or(USAGE)?
                .to_str()
                .ok_or("JPEG quality must be valid UTF-8")?
                .parse()?;
            images.jpeg_quality = Some(JpegQuality::new(quality)?);
        } else if flag == IMAGE_DPI_FLAG && images.dpi.is_none() {
            let dpi = flags
                .next()
                .ok_or(USAGE)?
                .to_str()
                .ok_or("image DPI must be valid UTF-8")?
                .parse::<f32>()
                .map_err(|error| format!("{IMAGE_DPI_FLAG}: {error}"))?;
            images.dpi =
                Some(ImageDpi::new(dpi).map_err(|error| format!("{IMAGE_DPI_FLAG}: {error}"))?);
        } else {
            let families = match flag.to_str() {
                Some(FONT_SERIF_FLAG) => &mut fonts.serif,
                Some(FONT_SANS_SERIF_FLAG) => &mut fonts.sans_serif,
                Some(FONT_MONOSPACE_FLAG) => &mut fonts.monospace,
                Some(FONT_FALLBACK_FLAG) => &mut fonts.fallback,
                _ => return Err(USAGE.into()),
            };
            let family = flags
                .next()
                .ok_or(USAGE)?
                .to_str()
                .ok_or("font family must be valid UTF-8")?;
            families.push(family.to_owned());
        }
    }
    let source = Path::new(&args[1]);
    let output = Path::new(&args[2]);
    let stdin = source == Path::new("-");
    let stdout = output == Path::new("-");
    if stdout && trace {
        return Err("--trace requires a PDF filename so its sidecar has a destination".into());
    }
    let trace_path = trace.then(|| output.with_extension(TRACE_EXTENSION));
    if headings.is_some_and(|path| path == Path::new("-")) {
        return Err("--headings requires a JSON filename".into());
    }
    let input_paths: Vec<_> = part_paths
        .iter()
        .copied()
        .chain((!stdin).then_some(source))
        .collect();
    let output_paths = [
        ("PDF output", (!stdout).then_some(output)),
        ("trace output", trace_path.as_deref()),
        ("heading-map output", headings),
    ];
    validate_output_paths(&input_paths, &output_paths)?;
    let options = RenderOptions {
        base: if stdin {
            Path::new(".")
        } else {
            source.parent().unwrap_or(Path::new("."))
        }
        .into(),
        include_layout: trace,
        images,
        print: ferri_pdf::PrintSettings {
            fonts,
            ..Default::default()
        },
        ..Default::default()
    };
    let html = if stdin {
        let mut html = Vec::new();
        io::stdin().lock().read_to_end(&mut html)?;
        html
    } else {
        fs::read(source)?
    };
    let (pdf, page_count, located_headings, layout_json, diagnostics) = if part_paths.is_empty() {
        let rendered = render_bytes(&html, &options)?;
        let layout = rendered
            .layout
            .as_ref()
            .map(serde_json::to_vec_pretty)
            .transpose()?;
        (
            rendered.pdf,
            rendered.page_count,
            rendered.headings,
            layout,
            rendered
                .diagnostics
                .into_iter()
                .map(|diagnostic| diagnostic.to_string())
                .collect::<Vec<_>>(),
        )
    } else {
        let additional = part_paths
            .iter()
            .map(fs::read)
            .collect::<io::Result<Vec<_>>>()?;
        let file_base = |path: &Path| -> Result<Url, Box<dyn std::error::Error>> {
            Url::from_file_path(std::path::absolute(path)?)
                .map_err(|()| "input path cannot form a file URL".into())
        };
        let first = HtmlPart::from_bytes(&html);
        let mut parts = vec![if stdin {
            first
        } else {
            first.with_base(file_base(source)?)
        }];
        for (bytes, path) in additional.iter().zip(&part_paths) {
            parts.push(HtmlPart::from_bytes(bytes).with_base(file_base(path)?));
        }
        let rendered = render_parts(&parts, &options)?;
        let layout = if trace {
            Some(serde_json::to_vec_pretty(&rendered.parts.iter().map(|part|
                serde_json::json!({"first_page": part.first_page, "layout": part.layout})
            ).collect::<Vec<_>>())?)
        } else {
            None
        };
        let mut located = Vec::new();
        let mut diagnostics = Vec::new();
        for (index, part) in rendered.parts.into_iter().enumerate() {
            located.extend(part.headings);
            diagnostics.extend(
                part.diagnostics
                    .into_iter()
                    .map(|diagnostic| format!("part {}: {diagnostic}", index + 1)),
            );
        }
        (
            rendered.pdf,
            rendered.page_count,
            located,
            layout,
            diagnostics,
        )
    };
    for diagnostic in diagnostics {
        eprintln!("warning: {diagnostic}");
    }
    validate_output_paths(&input_paths, &output_paths)?;
    if stdout {
        let mut stream = io::stdout().lock();
        stream.write_all(&pdf)?;
        stream.flush()?;
    } else {
        fs::write(output, &pdf)?;
    }
    if let Some(layout) = layout_json {
        fs::write(output.with_extension(TRACE_EXTENSION), layout)?;
    }
    if let Some(path) = headings {
        fs::write(path, serde_json::to_vec_pretty(&located_headings)?)?;
    }
    eprintln!("{} pages -> {}", page_count, output.display());
    Ok(())
}
fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
