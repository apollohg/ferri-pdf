//! CSS Text 4 wrapping, hyphenation, justification and hanging punctuation through layout.
mod common;

use common::{PROFILE, assert_labelled_pt, render_trace};
use ferri_pdf::{LayoutTrace, TextLine};

const HYPHEN: char = '\u{2010}';

fn trace(css: &str, body: &str) -> LayoutTrace {
    render_trace(&format!(
        "<!doctype html><style>{PROFILE}p{{margin:0}}{css}</style>{body}"
    ))
}

fn lines(trace: &LayoutTrace) -> Vec<&TextLine> {
    trace.pages.iter().flatten().collect()
}

fn texts(trace: &LayoutTrace) -> Vec<&str> {
    lines(trace).iter().map(|line| line.text.as_str()).collect()
}

/// The unwrapped advance of `text` in the profile font.
fn natural_width(text: &str) -> f32 {
    lines(&trace("p{white-space:nowrap}", &format!("<p>{text}</p>")))[0].width
}

/// Words that greedy filling at `width` leaves alone on a short last line.
const PARAGRAPH: &str = "Lines of text are filled one word after another until the next \
                         word would overflow and then it";

#[test]
fn balance_evens_out_lines_without_changing_their_number() {
    let width = 150.0;
    let css = format!("p{{width:{width}pt}}");
    let greedy = trace(&css, &format!("<p>{PARAGRAPH}</p>"));
    let balanced = trace(
        &format!("{css}p{{text-wrap:balance}}"),
        &format!("<p>{PARAGRAPH}</p>"),
    );
    let spread = |trace: &LayoutTrace| {
        let widths: Vec<f32> = lines(trace).iter().map(|line| line.width).collect();
        widths.iter().copied().fold(f32::MIN, f32::max)
            - widths.iter().copied().fold(f32::MAX, f32::min)
    };
    assert_eq!(
        lines(&balanced).len(),
        lines(&greedy).len(),
        "greedy {:?}, balanced {:?}",
        texts(&greedy),
        texts(&balanced)
    );
    assert!(
        spread(&balanced) < spread(&greedy) / 2.0,
        "balanced widths spread {} pt, greedy {} pt: {:?} vs {:?}",
        spread(&balanced),
        spread(&greedy),
        texts(&balanced),
        texts(&greedy)
    );
    assert!(lines(&balanced).iter().all(|line| line.width <= width));
    assert_eq!(
        texts(&balanced).join(" "),
        PARAGRAPH,
        "balancing only moves breaks"
    );
    // Groups between forced breaks balance separately.
    let groups = trace(
        &format!("{css}p{{text-wrap:balance}}"),
        &format!("<p>{PARAGRAPH}<br>short</p>"),
    );
    assert_eq!(texts(&groups).last(), Some(&"short"));
    assert_eq!(
        texts(&groups)[..texts(&groups).len() - 1],
        texts(&balanced)[..],
        "the group before the break balances as before"
    );
}

#[test]
fn balance_gives_way_to_first_fit_beyond_ten_lines() {
    let text = PARAGRAPH.repeat(4);
    let css = "p{width:150pt}";
    let greedy = trace(css, &format!("<p>{text}</p>"));
    assert!(lines(&greedy).len() > 10, "{} lines", lines(&greedy).len());
    let balanced = trace(
        &format!("{css}p{{text-wrap:balance}}"),
        &format!("<p>{text}</p>"),
    );
    assert_eq!(texts(&balanced), texts(&greedy));
}

#[test]
fn pretty_lengthens_a_short_last_line_unless_an_earlier_line_would_become_short() {
    let width = 150.0;
    let css = format!("p{{width:{width}pt}}");
    let greedy = trace(&css, &format!("<p>{PARAGRAPH}</p>"));
    let last = |trace: &LayoutTrace| lines(trace).last().unwrap().width;
    assert!(
        last(&greedy) < width / 3.0,
        "fixture needs a short greedy last line: {:?}",
        texts(&greedy)
    );
    let pretty = trace(
        &format!("{css}p{{text-wrap:pretty}}"),
        &format!("<p>{PARAGRAPH}</p>"),
    );
    assert_eq!(lines(&pretty).len(), lines(&greedy).len());
    assert!(
        last(&pretty) >= width / 3.0,
        "pretty last line {} pt: {:?}",
        last(&pretty),
        texts(&pretty)
    );
    assert!(lines(&pretty).iter().all(|line| line.width <= width));
    assert_eq!(texts(&pretty).join(" "), PARAGRAPH);
    // CSS Text 4 §5.4: rewrapping "the" alone to lengthen "river" is worse than a short end.
    let words = "Circumnavigating the Mississippi river";
    let narrow = natural_width("Mississippi river").max(natural_width("Circumnavigating"));
    let css = format!("p{{width:{narrow}pt}}");
    let greedy = trace(&css, &format!("<p>{words}</p>"));
    assert_eq!(
        texts(&greedy),
        ["Circumnavigating", "the Mississippi", "river"]
    );
    let pretty = trace(
        &format!("{css}p{{text-wrap:pretty}}"),
        &format!("<p>{words}</p>"),
    );
    assert_eq!(texts(&pretty), texts(&greedy));
}

#[test]
fn word_break_and_line_break_change_wrapping_and_min_content() {
    let word = "abcdefghij";
    let all = trace(
        "p{width:30pt;word-break:break-all}",
        &format!("<p>{word} {word}</p>"),
    );
    assert!(lines(&all).len() > 2, "{:?}", texts(&all));
    assert!(lines(&all).iter().all(|line| line.width <= 30.0));
    assert_eq!(texts(&all).concat().replace(' ', ""), word.repeat(2));
    let keep = trace(
        "p{width:40pt;word-break:keep-all}",
        "<p>漢字漢字漢字 漢字</p>",
    );
    assert_eq!(texts(&keep), ["漢字漢字漢字", "漢字"]);
    // `word-break` and `line-break: anywhere` feed min-content, unlike overflow-wrap
    // break-word, which shrink-to-fit tables measure.
    for (css, wraps) in [
        ("word-break:break-all", true),
        ("line-break:anywhere", true),
        ("word-break:break-word", true),
        ("word-break:normal", false),
    ] {
        let table = trace(
            &format!("table{{width:36pt;border-spacing:0;{css}}}td{{padding:0}}"),
            &format!("<table><tr><td>{word}</td></tr></table>"),
        );
        assert_eq!(lines(&table).len() > 1, wraps, "{css}: {:?}", texts(&table));
    }
}

#[test]
fn automatic_hyphens_use_hyphenate_character_and_language() {
    let css = "p{width:90pt;hyphens:auto}";
    let english = trace(css, "<p lang=en>international representatives</p>");
    assert!(
        texts(&english).iter().any(|text| text.ends_with(HYPHEN)),
        "{:?}",
        texts(&english)
    );
    assert_eq!(
        texts(&english).concat().replace([HYPHEN, ' '], ""),
        "internationalrepresentatives"
    );
    let unknown = trace(css, "<p lang=xx>international representatives</p>");
    assert!(
        texts(&unknown).iter().all(|text| !text.ends_with(HYPHEN)),
        "no resource behaves as manual: {:?}",
        texts(&unknown)
    );
    let custom = trace(
        &format!("{css}p{{hyphenate-character:'=\\2010'}}"),
        "<p lang=en>international representatives</p>",
    );
    assert!(
        texts(&custom)
            .iter()
            .any(|text| text.ends_with("=\u{2010}")),
        "{:?}",
        texts(&custom)
    );
    let silent = trace(
        &format!("{css}p{{hyphenate-character:''}}"),
        "<p lang=en>international representatives</p>",
    );
    assert_eq!(
        texts(&silent).concat().replace(' ', ""),
        "internationalrepresentatives",
        "an empty string breaks without a visible character"
    );
    assert!(lines(&silent).len() > 2, "{:?}", texts(&silent));
    let manual = trace(
        "p{width:45pt;hyphens:manual;hyphenate-character:'~'}",
        "<p>ex&shy;ample</p>",
    );
    assert_eq!(texts(&manual), ["ex~", "ample"], "soft hyphens use it too");
}

#[test]
fn text_justify_selects_the_justification_opportunities() {
    let body = "<p>Justify these words across the measure of the line box here.</p>";
    let width = 160.0;
    let base = format!("p{{width:{width}pt;text-align:justify}}");
    for justify in ["auto", "inter-word", "inter-character", "distribute"] {
        let justified = trace(&format!("{base}p{{text-justify:{justify}}}"), body);
        let first = lines(&justified)[0];
        assert_labelled_pt(&format!("{justify} first line"), first.width, width);
    }
    let words = trace(&format!("{base}p{{text-justify:inter-word}}"), body);
    let characters = trace(&format!("{base}p{{text-justify:inter-character}}"), body);
    let spacing = |line: &TextLine| -> Vec<f32> {
        line.runs
            .iter()
            .flat_map(|run| run.glyphs().iter().map(|glyph| glyph.x_spacing))
            .collect()
    };
    let word_spacing = spacing(lines(&words)[0]);
    let character_spacing = spacing(lines(&characters)[0]);
    assert!(
        word_spacing.iter().filter(|&&space| space > 0.0).count()
            < character_spacing
                .iter()
                .filter(|&&space| space > 0.0)
                .count(),
        "inter-character spreads over more gaps: {word_spacing:?} vs {character_spacing:?}"
    );
    assert_eq!(
        character_spacing.last(),
        Some(&0.0),
        "no gap after the line's last character"
    );
    let none = trace(&format!("{base}p{{text-justify:none}}"), body);
    let ragged = trace(&format!("p{{width:{width}pt}}"), body);
    assert_eq!(texts(&none), texts(&ragged));
    for (actual, expected) in lines(&none).iter().zip(lines(&ragged)) {
        assert_labelled_pt(
            "text-justify: none keeps start alignment",
            actual.width,
            expected.width,
        );
        assert_labelled_pt("start edge", actual.x, expected.x);
    }
}

#[test]
fn hanging_punctuation_sits_outside_the_line_box() {
    let width = 150.0;
    let body = "<p>\u{201c}Quoted words hang their marks.\u{201d}</p>";
    let plain = trace(&format!("p{{width:{width}pt}}"), body);
    let hung = trace(
        &format!("p{{width:{width}pt;hanging-punctuation:first last}}"),
        body,
    );
    // The hanging advance is the glyph's advance in the line, kerning included.
    let edge_advance = |line: &TextLine, last: bool| {
        let run = if last {
            line.runs.last()
        } else {
            line.runs.first()
        }
        .unwrap();
        let glyph = if last {
            run.glyphs().last()
        } else {
            run.glyphs().first()
        }
        .unwrap();
        glyph.advance(run.font_size)
    };
    let quote = edge_advance(lines(&plain)[0], false);
    assert_labelled_pt(
        "first: the opening quote hangs into the start margin",
        lines(&hung)[0].x,
        lines(&plain)[0].x - quote,
    );
    let right = trace(
        &format!("p{{width:{width}pt;text-align:right;hanging-punctuation:last}}"),
        body,
    );
    let line = *lines(&right).last().unwrap();
    let close = edge_advance(lines(&plain).last().unwrap(), true);
    assert_labelled_pt(
        "last: the closing quote hangs past the end edge",
        line.x + line.width,
        lines(&plain)[0].x + width + close,
    );
    // A comma that would not fit hangs; force-end hangs it even when it fits.
    let words = "aaaa bbbb,";
    let fit = natural_width("aaaa bbbb");
    let css = format!("p{{width:{fit}pt;text-align:right}}");
    let overflowing = trace(&css, &format!("<p>{words}</p>"));
    assert_eq!(texts(&overflowing), ["aaaa", "bbbb,"]);
    let allowed = trace(
        &format!("{css}p{{hanging-punctuation:allow-end}}"),
        &format!("<p>{words}</p>"),
    );
    assert_eq!(
        texts(&allowed),
        [words],
        "allow-end lets the comma hang to fit"
    );
    let roomy = format!("p{{width:{}pt;text-align:right}}", fit + 20.0);
    let allow_fitting = trace(
        &format!("{roomy}p{{hanging-punctuation:allow-end}}"),
        &format!("<p>{words}</p>"),
    );
    let force_fitting = trace(
        &format!("{roomy}p{{hanging-punctuation:force-end}}"),
        &format!("<p>{words}</p>"),
    );
    let end = |trace: &LayoutTrace| {
        let line = lines(trace)[0];
        line.x + line.width
    };
    assert_labelled_pt(
        "allow-end does not hang a comma that fits",
        end(&allow_fitting),
        lines(&plain)[0].x + fit + 20.0,
    );
    assert_labelled_pt(
        "force-end always hangs",
        end(&force_fitting),
        lines(&plain)[0].x + fit + 20.0 + edge_advance(lines(&force_fitting)[0], true),
    );
}

#[test]
fn preserve_spaces_converts_segment_breaks_and_tabs_to_spaces() {
    let preserved = trace(
        "p{white-space-collapse:preserve-spaces}",
        "<p>a  b\nc\td</p>",
    );
    assert_eq!(texts(&preserved), ["a  b c d"]);
    let breaks = trace("p{white-space:pre-line}", "<p>a  b\nc</p>");
    assert_eq!(texts(&breaks), ["a b", "c"]);
    let unwrapped = trace(
        "p{width:20pt;text-wrap-mode:nowrap}",
        "<p>several words never wrap</p>",
    );
    assert_eq!(texts(&unwrapped), ["several words never wrap"]);
}
