//! One document renders to the same PDF bytes on every run: within a process, and across
//! processes, whose hash seeds and allocation addresses differ.
mod common;

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

/// Renders of each fixture per mode: enough that output varying in one run of four fails.
const RENDERS: usize = 24;
/// The CLI's argument for reading the document from stdin and writing the PDF to stdout.
const STDIO: &str = "-";

/// SVG shapes whose CSS mask images rasterize while painting: two from one image, one from
/// another. Each painted mask is a new image, which must not take a dropped mask's identity.
const MASK_IMAGES: &str = "<!doctype html><style>
.same{mask-image:url(../assets/image-pattern.png);mask-size:100% 100%}
.other{mask-image:url(../assets/border-image-nine.png);mask-size:100% 100%}
</style><svg width=120 height=40><rect class=same width=30 height=30 fill=green />
<rect class=same x=40 width=30 height=30 fill=green />
<rect class=other x=80 width=30 height=30 fill=green /></svg>";

/// Paint servers, masks, clips, filters, shadows and blending in HTML and SVG.
const PAINT: &str = "<!doctype html><style>
div{width:60pt;height:30pt;margin:6pt;display:inline-block}
.linear{background:linear-gradient(45deg,red,blue 40%,green)}
.radial{background:radial-gradient(circle,yellow,purple);box-shadow:4pt 4pt 6pt rgba(0,0,0,.5)}
.conic{background:conic-gradient(red,green,blue,red);border-radius:12pt;filter:blur(2pt)}
.masked{background:teal;mask:linear-gradient(black,transparent),url(../assets/image-pattern.png) 0 0/10pt 10pt}
.blend{background:orange;mix-blend-mode:multiply;opacity:.6;text-shadow:2pt 2pt 3pt blue}
</style><div class=linear></div><div class=radial></div><div class=conic></div>
<div class=masked></div><div class=blend>blend</div>
<svg width=200 height=100><defs>
<pattern id=dots width=10 height=10 patternUnits=userSpaceOnUse><circle cx=5 cy=5 r=3 fill=navy /></pattern>
<linearGradient id=fade><stop offset=0 stop-color=red /><stop offset=1 stop-color=blue stop-opacity=.2 /></linearGradient>
<radialGradient id=glow><stop offset=0 stop-color=white /><stop offset=1 stop-color=black /></radialGradient>
<mask id=half><rect width=50 height=100 fill=url(#glow) /></mask>
<clipPath id=round><circle cx=150 cy=50 r=40 /></clipPath>
<filter id=soft><feGaussianBlur stdDeviation=2 /><feOffset dx=3 dy=3 /></filter></defs>
<rect width=100 height=50 fill=url(#dots) /><rect y=50 width=100 height=50 fill=url(#fade) mask=url(#half) />
<rect x=100 width=100 height=100 fill=url(#glow) clip-path=url(#round) filter=url(#soft) /></svg>";

/// Embedded and system fonts, raster and vector images, links, destinations, outlines and
/// page counters over several pages.
const DOCUMENT: &str = "<!doctype html><style>
@font-face{font-family:Web;src:url(../assets/DejaVuSans.ttf)}
@font-face{font-family:Web;font-weight:bold;src:url(../assets/DejaVuSans-Bold.ttf)}
@page{size:200pt 200pt;margin:20pt;@bottom-center{content:counter(page) ' / ' counter(pages)}}
h1,h2{font:bold 12pt Web;break-before:page}h1{break-before:auto}
p{font:9pt serif}em{font-family:Web;font-style:italic}
img{width:30pt;height:30pt}
</style><h1 id=start>Start</h1><p>Plain <em>emphasis</em> <b>bold</b> and
<a href=https://example.com />an external link</a> and <a href=#end>a link ahead</a>.</p>
<img src=../assets/image-pattern.png><img src=../assets/image-pattern.jpg>
<img src=../assets/image-grid.avif><img src=../assets/image-grid.heic>
<img src=../assets/image-pattern.svg><img src=../assets/image-high-depth.png>
<h2>Middle</h2><p>Text in <span style=font-family:Web>Web</span> with <a href=#start>a link back</a>.</p>
<h2 id=end>End</h2><p>The <a href=#missing>last</a> page.</p>";

const FIXTURES: &[(&str, &str)] = &[
    ("mask images", MASK_IMAGES),
    ("paint", PAINT),
    ("document", DOCUMENT),
];

fn fixtures_dir() -> &'static Path {
    Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/oracle/fixtures"
    ))
}

/// The run numbers whose PDF differs from the first run's, and the byte where each first
/// differs, for the failure message.
fn variations(pdfs: &[Vec<u8>]) -> Vec<(usize, usize)> {
    let first = &pdfs[0];
    pdfs.iter()
        .enumerate()
        .skip(1)
        .filter(|(_, pdf)| *pdf != first)
        .map(|(run, pdf)| {
            let offset = first
                .iter()
                .zip(pdf)
                .position(|(a, b)| a != b)
                .unwrap_or_else(|| first.len().min(pdf.len()));
            (run, offset)
        })
        .collect()
}

fn assert_identical(label: &str, mode: &str, pdfs: &[Vec<u8>]) {
    let varied = variations(pdfs);
    assert!(
        varied.is_empty(),
        "{label}: {} of {} {mode} renders differ from the first (run, first differing byte): \
         {varied:?}; sizes {:?}",
        varied.len(),
        pdfs.len(),
        pdfs.iter().map(Vec::len).collect::<Vec<_>>()
    );
}

#[test]
fn repeated_renders_in_one_process_write_identical_pdfs() {
    let options = common::local_options();
    for (label, html) in FIXTURES {
        let pdfs: Vec<Vec<u8>> = (0..RENDERS)
            .map(|_| {
                ferri_pdf::render_html(html, &options)
                    .unwrap_or_else(|error| panic!("{label}: render failed: {error}"))
                    .pdf
            })
            .collect();
        assert_identical(label, "in-process", &pdfs);
    }
}

#[test]
fn renders_in_fresh_processes_write_identical_pdfs() {
    for (label, html) in FIXTURES {
        let pdfs: Vec<Vec<u8>> = (0..RENDERS)
            .map(|_| {
                let mut child = Command::new(env!("CARGO_BIN_EXE_ferri-pdf"))
                    .args([STDIO, STDIO])
                    .current_dir(fixtures_dir())
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
                    .unwrap();
                child
                    .stdin
                    .take()
                    .unwrap()
                    .write_all(html.as_bytes())
                    .unwrap();
                let output = child.wait_with_output().unwrap();
                assert!(
                    output.status.success(),
                    "{label}: CLI render failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
                output.stdout
            })
            .collect();
        assert_identical(label, "fresh-process", &pdfs);
    }
}
