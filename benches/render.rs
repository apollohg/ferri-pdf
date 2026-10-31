//! Renders each HTML file repeatedly in one process and prints one JSON line of timings per
//! file. `benches/run.py` generates the corpus and drives this binary; run it directly under a
//! profiler with a high `--runs`.
#![expect(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "a benchmark reports its timings on stdout and usage on stderr"
)]

use ferri_pdf::{ContentLoss, RenderOptions, render_bytes};
use std::{env, fs, path::Path, process::ExitCode, time::Instant};

const USAGE: &str = "usage: render [--runs N] INPUT.html ...";
const RUNS_FLAG: &str = "--runs";
const DEFAULT_RUNS: usize = 5;
const MILLISECONDS_PER_SECOND: f64 = 1000.0;

fn median(sorted: &[f64]) -> f64 {
    let middle = sorted.len() / 2;
    if sorted.len().is_multiple_of(2) {
        f64::midpoint(sorted[middle - 1], sorted[middle])
    } else {
        sorted[middle]
    }
}

fn bench(path: &Path, runs: usize) -> Result<String, Box<dyn std::error::Error>> {
    let html = fs::read(path)?;
    let options = RenderOptions {
        base: path.parent().unwrap_or(Path::new(".")).into(),
        content_loss: ContentLoss::Render,
        ..Default::default()
    };
    let mut times = Vec::with_capacity(runs);
    let mut rendered = None;
    for _ in 0..runs {
        let start = Instant::now();
        let output = render_bytes(&html, &options)?;
        times.push(start.elapsed().as_secs_f64() * MILLISECONDS_PER_SECOND);
        rendered = Some(output);
    }
    let first = times[0];
    times.sort_by(f64::total_cmp);
    let output = rendered.ok_or(USAGE)?;
    let lost: Vec<_> = output
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.loses_content())
        .map(ToString::to_string)
        .collect();
    Ok(serde_json::json!({
        "document": path.file_stem().map(|stem| stem.to_string_lossy()),
        "pages": output.page_count,
        "pdf_bytes": output.pdf.len(),
        "runs": runs,
        "first_ms": first,
        "median_ms": median(&times),
        "min_ms": times[0],
        "max_ms": times[times.len() - 1],
        "content_lost": lost,
    })
    .to_string())
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut runs = DEFAULT_RUNS;
    let mut paths = Vec::new();
    let mut args = env::args_os().skip(1);
    while let Some(arg) = args.next() {
        if arg == RUNS_FLAG {
            runs = args.next().ok_or(USAGE)?.to_str().ok_or(USAGE)?.parse()?;
        } else if arg != "--bench" {
            paths.push(arg);
        }
    }
    if paths.is_empty() || runs == 0 {
        return Err(USAGE.into());
    }
    for path in paths {
        println!("{}", bench(Path::new(&path), runs)?);
    }
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
