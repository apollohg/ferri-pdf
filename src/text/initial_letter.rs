//! Initial letter sizing and block-axis positioning (CSS Inline 3 §§7.4–7.6). Lengths are in
//! points, downwards from the originating line box's top, against the containing block's
//! root inline box alone: the contents of the impacted lines do not move the letter.
use crate::style::{InitialLetter, LetterAlignment, Style, TextEdge};
use crate::text::{TextContext, TextError, line_boxes::BoxMetrics};
use unicode_script::{Script, UnicodeScript};

/// Scripts whose initial letters align their ideographic ink box (§7.4).
const IDEOGRAPHIC_SCRIPTS: [Script; 5] = [
    Script::Han,
    Script::Hangul,
    Script::Hiragana,
    Script::Katakana,
    Script::Yi,
];
/// Scripts whose initial letters align their hanging baseline (§7.4, the second rule's
/// evident intent: the Indic scripts the UA style sheet aligns `hanging`).
const HANGING_SCRIPTS: [Script; 4] = [
    Script::Devanagari,
    Script::Bengali,
    Script::Gurmukhi,
    Script::Tibetan,
];

/// The over and under points `edges` name on a box with `metrics`: how far they lie above
/// and below its baseline.
fn points(metrics: &BoxMetrics, edges: [TextEdge; 2]) -> [f32; 2] {
    [-metrics.edge(edges[0], false), metrics.edge(edges[1], true)]
}

/// The containing block's root inline box an initial letter sizes and aligns to (§7.5.3).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Surrounding {
    /// The over alignment point above the root baseline.
    pub over: f32,
    /// The under alignment point below the root baseline.
    pub under: f32,
    pub line_height: f32,
    /// The root baseline below the line box top.
    pub baseline: f32,
}
impl Surrounding {
    /// The root inline box `root`'s alignment points for `alignment`.
    pub(crate) fn of(
        context: &TextContext<'_>,
        root: &Style,
        alignment: LetterAlignment,
    ) -> Result<Self, TextError> {
        let metrics = BoxMetrics::of(context, root)?;
        let [top, bottom] = metrics.extent();
        let [over, under] = match alignment {
            LetterAlignment::Alphabetic => points(&metrics, [TextEdge::Cap, TextEdge::Alphabetic]),
            LetterAlignment::Ideographic => points(&metrics, [TextEdge::IdeographicInk; 2]),
            LetterAlignment::Hanging => points(&metrics, [TextEdge::Hanging, TextEdge::Alphabetic]),
            LetterAlignment::Leading => [-top, bottom],
        };
        Ok(Self {
            over,
            under,
            line_height: metrics.line_height(),
            baseline: -top,
        })
    }
    /// The distance from the first line's over point to the `lines`th line's under point,
    /// which a letter `lines` lines tall spans.
    pub(crate) fn span(&self, lines: f32) -> f32 {
        (lines - 1.0) * self.line_height + self.over + self.under
    }
    /// The first line's over point.
    fn first_over(&self) -> f32 {
        self.baseline - self.over
    }
    /// The `line`th line's under point.
    pub(crate) fn under_of(&self, line: u32) -> f32 {
        self.baseline + self.under + line.saturating_sub(1) as f32 * self.line_height
    }
    /// Where a letter whose alignment points lie `height` apart puts its over point (§7.6.1):
    /// a letter at least as tall as its sink aligns its under point to the sink's line,
    /// a shorter one its over point to the first line's.
    pub(crate) fn over_position(&self, initial: InitialLetter, height: f32) -> f32 {
        if initial.size >= initial.sink as f32 {
            self.under_of(initial.sink) - height
        } else {
            self.first_over()
        }
    }
}

/// The edges an inline initial letter of `text` aligns: its ideographic ink box, its hanging
/// and alphabetic baselines, or its cap height and alphabetic baseline (§7.4).
fn letter_edges(text: &str) -> [TextEdge; 2] {
    let scripts: Vec<Script> = text.chars().map(|ch| ch.script()).collect();
    if scripts
        .iter()
        .any(|script| IDEOGRAPHIC_SCRIPTS.contains(script))
    {
        [TextEdge::IdeographicInk; 2]
    } else if scripts
        .iter()
        .any(|script| HANGING_SCRIPTS.contains(script))
    {
        [TextEdge::Hanging, TextEdge::Alphabetic]
    } else {
        [TextEdge::Cap, TextEdge::Alphabetic]
    }
}

/// How many times its computed font sizes an inline initial letter styled `letter` holding
/// `text` uses, and the edges that hold its alignment points (§7.5.3): those points span
/// `initial.size` lines of `surrounding`. Border-box alignment instead fits the font's text
/// box, inside the letter's block-axis padding and borders `edges`, to that span. A font
/// without extent between those edges keeps its size.
pub(crate) fn font_scale(
    context: &TextContext<'_>,
    surrounding: &Surrounding,
    letter: &Style,
    text: &str,
    edges: f32,
) -> Result<(f32, [TextEdge; 2]), TextError> {
    let initial = letter
        .initial_letter
        .expect("an initial letter has a size and sink");
    let (aligned, target) = if letter.initial_letter_align.border_box {
        ([TextEdge::Text; 2], surrounding.span(initial.size) - edges)
    } else {
        (letter_edges(text), surrounding.span(initial.size))
    };
    let [over, under] = points(&BoxMetrics::of(context, letter)?, aligned);
    let extent = over + under;
    Ok((
        if extent > 0.0 {
            (target / extent).max(0.0)
        } else {
            1.0
        },
        aligned,
    ))
}
