use ferri_pdf::RenderOptions;
use std::path::Path;

#[allow(
    dead_code,
    reason = "each test binary uses a subset of the shared helpers"
)]
pub(crate) const PROFILE: &str = include_str!("../oracle/fixtures/profile.css");

pub(crate) fn local_options() -> RenderOptions {
    RenderOptions {
        base: Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/oracle/fixtures")
            .into(),
        include_layout: true,
        ..Default::default()
    }
}

/// `local_options` that render documents whose content resource limits leave out.
#[allow(
    dead_code,
    reason = "each test binary uses a subset of the shared helpers"
)]
pub(crate) fn lossy_options() -> RenderOptions {
    RenderOptions {
        content_loss: ferri_pdf::ContentLoss::Render,
        ..local_options()
    }
}

#[allow(
    dead_code,
    reason = "each test binary uses a subset of the shared helpers"
)]
pub(crate) fn render_trace(html: &str) -> ferri_pdf::LayoutTrace {
    ferri_pdf::render_html(html, &local_options())
        .expect("render test HTML")
        .layout
        .expect("layout requested")
}

#[allow(
    dead_code,
    reason = "each test binary uses a subset of the shared helpers"
)]
pub(crate) fn assert_pt(actual: f32, expected: f32) {
    assert_labelled_pt("value", actual, expected);
}

#[allow(
    dead_code,
    reason = "each test binary uses a subset of the shared helpers"
)]
pub(crate) fn assert_labelled_pt(label: &str, actual: f32, expected: f32) {
    const POINT_TOLERANCE: f32 = 0.001;
    assert!(
        (actual - expected).abs() <= POINT_TOLERANCE,
        "{label}: expected {expected} pt, got {actual} pt (tolerance {POINT_TOLERANCE} pt)"
    );
}

#[allow(
    dead_code,
    reason = "each test binary uses a subset of the shared helpers"
)]
pub(crate) fn painted_box(trace: &ferri_pdf::LayoutTrace, color: [u8; 3]) -> ferri_pdf::Rect {
    let boxes: Vec<_> = trace
        .display_lists
        .iter()
        .flatten()
        .filter_map(|item| match item {
            ferri_pdf::DisplayItem::Box(paint)
                if paint.decoration.background.rgb == color
                    && paint.decoration.background.alpha == 1.0 =>
            {
                Some(paint.rect)
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        boxes.len(),
        1,
        "expected one {color:?} box, found {boxes:?}"
    );
    boxes[0]
}

#[allow(
    dead_code,
    reason = "each test binary uses a subset of the shared helpers"
)]
/// The boxes painted with background `rgb`, mapped through the transforms and frames that
/// hold them into page coordinates.
pub(crate) fn page_boxes(trace: &ferri_pdf::LayoutTrace, rgb: [u8; 3]) -> Vec<ferri_pdf::Rect> {
    use ferri_pdf::DisplayItem;
    const IDENTITY: [f32; 6] = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
    fn compose([a, b, c, d, e, f]: [f32; 6], [g, h, i, j, k, l]: [f32; 6]) -> [f32; 6] {
        [
            a * g + c * h,
            b * g + d * h,
            a * i + c * j,
            b * i + d * j,
            a * k + c * l + e,
            b * k + d * l + f,
        ]
    }
    fn point([a, b, c, d, e, f]: [f32; 6], [x, y]: [f32; 2]) -> [f32; 2] {
        [a * x + c * y + e, b * x + d * y + f]
    }
    fn collect(
        items: &[DisplayItem],
        outer: [f32; 6],
        rgb: [u8; 3],
        found: &mut Vec<ferri_pdf::Rect>,
    ) {
        let mut stack = vec![outer];
        for item in items {
            let matrix = *stack
                .last()
                .expect("the outer transform stays on the stack");
            match item {
                DisplayItem::Box(paint)
                    if paint.decoration.background.rgb == rgb
                        && paint.decoration.background.alpha == 1.0 =>
                {
                    let rect = paint.rect;
                    let [first, second] = [
                        point(matrix, [rect.x, rect.y]),
                        point(matrix, [rect.x + rect.width, rect.y + rect.height]),
                    ];
                    found.push(ferri_pdf::Rect {
                        x: first[0].min(second[0]),
                        y: first[1].min(second[1]),
                        width: (first[0] - second[0]).abs(),
                        height: (first[1] - second[1]).abs(),
                    });
                }
                DisplayItem::Frame(frame) => {
                    collect(&frame.items, compose(matrix, frame.matrix), rgb, found);
                }
                DisplayItem::PushTransform(transform) => stack.push(compose(matrix, *transform)),
                DisplayItem::PushClip(_)
                | DisplayItem::PushOpacity(_)
                | DisplayItem::PushBlend(_)
                | DisplayItem::PushFilter(_)
                | DisplayItem::PushMask(_)
                | DisplayItem::PushBackdropRoot => stack.push(matrix),
                DisplayItem::Pop => {
                    stack.pop();
                }
                _ => {}
            }
        }
    }
    let mut found = Vec::new();
    for page in &trace.display_lists {
        collect(page, IDENTITY, rgb, &mut found);
    }
    found
}

/// The texts of the lines painted inside the frames of `page`, such as footnote bodies and
/// running elements, that the block-axis clips of the slices holding them leave visible, by
/// their left edge, each such column top to bottom.
#[allow(
    dead_code,
    reason = "each test binary uses a subset of the shared helpers"
)]
pub(crate) fn visible_frame_columns(
    trace: &ferri_pdf::LayoutTrace,
    page: usize,
) -> Vec<(f32, Vec<String>)> {
    use ferri_pdf::DisplayItem;
    fn visit(
        items: &[DisplayItem],
        lines: Option<&[ferri_pdf::TextLine]>,
        origin: [f32; 2],
        clips: &mut Vec<[f32; 2]>,
        found: &mut Vec<(f32, f32, String)>,
    ) {
        let depth = clips.len();
        let mut opened = Vec::new();
        for item in items {
            match item {
                DisplayItem::PushClip(clip) if clip.axes == [false, true] => {
                    clips.push([
                        origin[1] + clip.rect.y,
                        origin[1] + clip.rect.y + clip.rect.height,
                    ]);
                    opened.push(true);
                }
                DisplayItem::Pop => {
                    if opened.pop() == Some(true) {
                        clips.pop();
                    }
                }
                item if item.is_push() => opened.push(false),
                DisplayItem::Text { line } => {
                    let Some(lines) = lines else { continue };
                    let line = &lines[*line];
                    let baseline = origin[1] + line.baseline;
                    if clips
                        .iter()
                        .all(|[top, bottom]| (*top..*bottom).contains(&baseline))
                    {
                        found.push((origin[0] + line.x, baseline, line.text.clone()));
                    }
                }
                DisplayItem::Frame(frame) => visit(
                    &frame.items,
                    Some(&frame.lines),
                    [origin[0] + frame.matrix[4], origin[1] + frame.matrix[5]],
                    clips,
                    found,
                ),
                _ => {}
            }
        }
        clips.truncate(depth);
    }
    let mut found = Vec::new();
    visit(
        &trace.display_lists[page],
        None,
        [0.0; 2],
        &mut Vec::new(),
        &mut found,
    );
    found.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
    let mut columns: Vec<(f32, Vec<String>)> = Vec::new();
    for (x, _, text) in found {
        match columns.last_mut() {
            Some((left, texts)) if (*left - x).abs() < 1.0 => texts.push(text),
            _ => columns.push((x, vec![text])),
        }
    }
    columns
}

/// The column rules of `page` whose color is `rgb`, painted as left borders.
#[allow(
    dead_code,
    reason = "each test binary uses a subset of the shared helpers"
)]
pub(crate) fn column_rules(
    trace: &ferri_pdf::LayoutTrace,
    page: usize,
    rgb: [u8; 3],
) -> Vec<ferri_pdf::Rect> {
    const LEFT: usize = 3;
    fn collect(items: &[ferri_pdf::DisplayItem], rgb: [u8; 3], found: &mut Vec<ferri_pdf::Rect>) {
        for item in items {
            match item {
                ferri_pdf::DisplayItem::Box(paint)
                    if paint.decoration.borders[LEFT].color.rgb == rgb
                        && paint.decoration.borders[LEFT].width > 0.0 =>
                {
                    found.push(paint.rect);
                }
                ferri_pdf::DisplayItem::Frame(frame) => {
                    let mut nested = Vec::new();
                    collect(&frame.items, rgb, &mut nested);
                    found.extend(nested.into_iter().map(|rect| ferri_pdf::Rect {
                        x: rect.x + frame.matrix[4],
                        y: rect.y + frame.matrix[5],
                        ..rect
                    }));
                }
                _ => {}
            }
        }
    }
    let mut found = Vec::new();
    collect(&trace.display_lists[page], rgb, &mut found);
    found
}

/// The largest magnitude a number in the output may have: ISO 32000-1 Annex C.2 Table C.1
/// limits integers to 2^31 - 1, and readers built on 32-bit numbers represent no larger value.
#[allow(
    dead_code,
    reason = "each test binary uses a subset of the shared helpers"
)]
pub(crate) const PDF_NUMBER_LIMIT: f64 = 2_147_483_647.0;

/// Why `pdf` is not a valid PDF whose numbers lie within [`PDF_NUMBER_LIMIT`]: that it does not
/// parse, or each token of its objects and content streams that is a non-finite or
/// exponent-form number (which ISO 32000-1 §7.3.3 number syntax lacks) or a number past the
/// limit. Empty for a valid PDF.
#[allow(
    dead_code,
    reason = "each test binary uses a subset of the shared helpers"
)]
pub(crate) fn pdf_number_faults(pdf: &[u8]) -> Vec<String> {
    use hayro::hayro_syntax::object::{Name, Object};
    /// Streams holding binary data rather than PDF syntax: images, font programs, ICC
    /// profiles, sampled functions, cross-reference streams and XML metadata.
    fn is_binary(stream: &hayro::hayro_syntax::object::Stream<'_>) -> bool {
        let dict = stream.dict();
        let named = |key: &[u8], values: &[&[u8]]| {
            dict.get::<Name<'_>>(key)
                .is_some_and(|name| values.contains(&name.as_ref()))
        };
        named(
            b"Subtype",
            &[b"Image", b"Type1C", b"CIDFontType0C", b"OpenType"],
        ) || named(b"Type", &[b"XRef", b"Metadata"])
            || [&b"Length1"[..], b"Length2", b"Length3", b"N"]
                .iter()
                .any(|key| dict.contains_key(key))
            || dict.get::<i32>(b"FunctionType") == Some(0)
    }
    let parsed = match hayro::hayro_syntax::Pdf::new(pdf.to_vec()) {
        Ok(parsed) => parsed,
        Err(error) => return vec![format!("the PDF does not parse: {error:?}")],
    };
    let mut faults = Vec::new();
    scan_pdf_tokens("file", pdf, true, &mut faults);
    for object in parsed.objects() {
        if let Object::Stream(stream) = object
            && !is_binary(&stream)
        {
            let id = stream.obj_id();
            match stream.decoded() {
                Ok(data) => scan_pdf_tokens(&format!("stream {id:?}"), &data, false, &mut faults),
                Err(error) => faults.push(format!("stream {id:?} does not decode: {error:?}")),
            }
        }
    }
    faults
}

/// Records in `faults` each number token of the PDF syntax in `data` that is not a finite
/// number within [`PDF_NUMBER_LIMIT`] in PDF number syntax; with `skip_streams`, stream data is
/// passed over.
fn scan_pdf_tokens(source: &str, data: &[u8], skip_streams: bool, faults: &mut Vec<String>) {
    const DELIMITERS: &[u8] = b"()<>[]{}/%";
    const STREAM_END: &[u8] = b"endstream";
    let regular =
        |byte: u8| !byte.is_ascii_whitespace() && byte != 0 && !DELIMITERS.contains(&byte);
    let mut index = 0;
    while index < data.len() {
        let byte = data[index];
        match byte {
            b'%' => {
                while index < data.len() && !matches!(data[index], b'\r' | b'\n') {
                    index += 1;
                }
            }
            b'(' => {
                let mut depth = 0;
                while index < data.len() {
                    match data[index] {
                        b'\\' => index += 1,
                        b'(' => depth += 1,
                        b')' => depth -= 1,
                        _ => {}
                    }
                    index += 1;
                    if depth == 0 {
                        break;
                    }
                }
            }
            b'<' if data.get(index + 1) != Some(&b'<') => {
                while index < data.len() && data[index] != b'>' {
                    index += 1;
                }
                index += 1;
            }
            b'/' => {
                index += 1;
                while index < data.len() && regular(data[index]) {
                    index += 1;
                }
            }
            _ if regular(byte) => {
                let start = index;
                while index < data.len() && regular(data[index]) {
                    index += 1;
                }
                let token = String::from_utf8_lossy(&data[start..index]);
                if skip_streams && token == "stream" {
                    index = data[index..]
                        .windows(STREAM_END.len())
                        .position(|window| window == STREAM_END)
                        .map_or(data.len(), |offset| index + offset + STREAM_END.len());
                    continue;
                }
                let Ok(value) = token.parse::<f64>() else {
                    continue;
                };
                let digits = token.trim_start_matches(['+', '-']);
                let pdf_syntax = digits
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || byte == b'.')
                    && digits.bytes().filter(|&byte| byte == b'.').count() <= 1;
                if !pdf_syntax || !value.is_finite() {
                    faults.push(format!(
                        "{source} at byte {start}: `{token}` is not a PDF number"
                    ));
                } else if value.abs() > PDF_NUMBER_LIMIT {
                    faults.push(format!(
                        "{source} at byte {start}: {token} is past the limit {PDF_NUMBER_LIMIT}"
                    ));
                }
            }
            _ => index += 1,
        }
    }
}
