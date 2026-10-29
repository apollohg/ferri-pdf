//! CSS Text 4 §8.4 `text-autospace`: spacing between characters of the classes it names that
//! adjoin on a line, without an intervening box edge, atomic inline or other character. It
//! is inserted after bidi reordering (§8.4.2), between visually adjacent characters.
use crate::document::Paragraph;
use crate::fonts::{FALLBACK_IDEOGRAPH_ADVANCE_EM, WATER_IDEOGRAPH};
use crate::style::{Style, TextAutospace, has_primary_language};
use crate::text::line_boxes::{Arranged, Slot};
use crate::text::shaping::NARROW_SPACE_FACTOR;
use crate::text::{TextContext, TextError};
use icu_properties::{
    CodePointMapData,
    props::{EastAsianWidth, GeneralCategory, GeneralCategoryGroup},
};
use std::cell::OnceCell;
use std::ops::RangeInclusive;
use unicode_script::{Script, UnicodeScript};

/// §8.4.1: inter-script spacing is 1/8 of the CJK advance measure.
const INTER_SCRIPT_IC: f32 = 0.125;
/// §8.4.1 ideographs outside the Han script: kana that are not punctuation, CJK strokes and
/// Katakana phonetic extensions.
const KANA: RangeInclusive<char> = '\u{3041}'..='\u{30ff}';
const STROKES_AND_PHONETIC_EXTENSIONS: RangeInclusive<char> = '\u{31c0}'..='\u{31ff}';
const SPACE: char = ' ';
const NO_BREAK_SPACE: char = '\u{a0}';
const FRENCH: &str = "fr";
/// French typography sets a narrow no-break space before these and after opening guillemets,
/// and a no-break space before the colon.
const NARROW_BEFORE: [char; 5] = [';', '!', '?', '\u{bb}', '\u{203a}'];
const NARROW_AFTER: [char; 2] = ['\u{ab}', '\u{2039}'];
const NO_BREAK_BEFORE: char = ':';

/// §8.4.1 character classes.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Class {
    Ideograph,
    Letter,
    Numeral,
    Other,
}

fn ideograph(ch: char) -> bool {
    if ch.is_ascii() {
        return false;
    }
    // A quotation mark or other punctuation between characters suppresses the spacing.
    let punctuation = || {
        GeneralCategoryGroup::Punctuation
            .contains(CodePointMapData::<GeneralCategory>::new().get(ch))
    };
    // Common and Inherited characters have every script in their extension set.
    let extensions = ch.script_extension();
    let han = !extensions.is_common()
        && !extensions.is_inherited()
        && extensions.contains_script(Script::Han);
    STROKES_AND_PHONETIC_EXTENSIONS.contains(&ch) || ((KANA.contains(&ch) || han) && !punctuation())
}

fn class(ch: char) -> Class {
    if ideograph(ch) {
        return Class::Ideograph;
    }
    let category = CodePointMapData::<GeneralCategory>::new().get(ch);
    let width = CodePointMapData::<EastAsianWidth>::new().get(ch);
    if (GeneralCategoryGroup::Letter.contains(category)
        || GeneralCategoryGroup::Mark.contains(category))
        && !matches!(width, EastAsianWidth::Wide | EastAsianWidth::Fullwidth)
    {
        Class::Letter
    } else if category == GeneralCategory::DecimalNumber && width != EastAsianWidth::Fullwidth {
        Class::Numeral
    } else {
        Class::Other
    }
}

fn separator(ch: char) -> bool {
    GeneralCategoryGroup::Separator.contains(CodePointMapData::<GeneralCategory>::new().get(ch))
}

fn french_punctuation(ch: char) -> bool {
    NARROW_BEFORE.contains(&ch) || NARROW_AFTER.contains(&ch) || ch == NO_BREAK_BEFORE
}

/// The spacing between two characters.
#[derive(Clone, Copy)]
enum Gap {
    InterScript,
    Narrow,
    NoBreak,
}

/// The advance of `ch` in the font that renders it for `style`, in points.
fn advance(context: &TextContext<'_>, style: &Style, ch: char) -> Result<Option<f32>, TextError> {
    Ok(context
        .fonts
        .advance(
            style.font_candidates(),
            style.font_selection(),
            &style.font_adjustments,
            style.size,
            ch,
            context.resolver,
        )?
        .map(|(advance, scale)| advance * scale * style.size))
}

/// The value in `cell`, measured on first use.
fn measured<T: Copy>(
    cell: &OnceCell<T>,
    measure: impl FnOnce() -> Result<T, TextError>,
) -> Result<T, TextError> {
    if let Some(value) = cell.get() {
        return Ok(*value);
    }
    let value = measure()?;
    Ok(*cell.get_or_init(|| value))
}

/// What an element's `text-autospace` inserts at the boundaries it contains. Gap widths are
/// measured on first use, so that no font is loaded for lines that are never laid out.
struct Spacing<'p> {
    style: &'p Style,
    value: TextAutospace,
    /// `punctuation` spacing applies: the content language is French.
    french: bool,
    /// The inter-script gap and the narrow and ordinary no-break space gaps, in points.
    inter_script: OnceCell<f32>,
    spaces: OnceCell<[f32; 2]>,
}
impl<'p> Spacing<'p> {
    fn new(style: &'p Style) -> Self {
        let value = style.text_spacing.autospace;
        Self {
            style,
            value,
            french: value.punctuation && has_primary_language(&style.language, FRENCH),
            inter_script: OnceCell::new(),
            spaces: OnceCell::new(),
        }
    }

    /// Whether the element spaces any boundary of text that has `ideographs` and French
    /// `punctuation`.
    fn active(&self, ideographs: bool, punctuation: bool) -> bool {
        (ideographs && (self.value.ideograph_alpha || self.value.ideograph_numeric))
            || (punctuation && self.french)
    }

    /// The gap between `first` and `second`, in logical order.
    fn gap(&self, first: char, second: char) -> Option<Gap> {
        let classes = [class(first), class(second)];
        if (self.value.ideograph_alpha
            && matches!(
                classes,
                [Class::Ideograph, Class::Letter] | [Class::Letter, Class::Ideograph]
            ))
            || (self.value.ideograph_numeric
                && matches!(
                    classes,
                    [Class::Ideograph, Class::Numeral] | [Class::Numeral, Class::Ideograph]
                ))
        {
            return Some(Gap::InterScript);
        }
        if !self.french || separator(first) || separator(second) {
            None
        } else if NARROW_BEFORE.contains(&second) || NARROW_AFTER.contains(&first) {
            Some(Gap::Narrow)
        } else if second == NO_BREAK_BEFORE {
            Some(Gap::NoBreak)
        } else {
            None
        }
    }

    fn width(&self, context: &TextContext<'_>, gap: Gap) -> Result<f32, TextError> {
        let style = self.style;
        if let Gap::InterScript = gap {
            return measured(&self.inter_script, || {
                Ok(advance(context, style, WATER_IDEOGRAPH)?
                    .unwrap_or(FALLBACK_IDEOGRAPH_ADVANCE_EM * style.size)
                    * INTER_SCRIPT_IC)
            });
        }
        let [narrow, no_break] = measured(&self.spaces, || {
            let space = advance(context, style, SPACE)?.unwrap_or_default();
            Ok([
                space * NARROW_SPACE_FACTOR,
                advance(context, style, NO_BREAK_SPACE)?.unwrap_or(space),
            ])
        })?;
        Ok(if let Gap::Narrow = gap {
            narrow
        } else {
            no_break
        })
    }
}

/// The last glyph of a typographic character unit on a line, in visual order.
struct Unit {
    slot: usize,
    run: usize,
    glyph: usize,
    /// Paragraph offset of the unit's base character, which decides logical order.
    offset: usize,
    base: char,
    span: usize,
    /// The unit is a single U+0020, which `replace` removes.
    space: bool,
}

/// Spacing inserted after `unit`, rendered within inline box `owner`, which replaces the
/// `removed` space.
struct Insertion<'a> {
    unit: &'a Unit,
    removed: Option<&'a Unit>,
    owner: Option<usize>,
    width: f32,
}

/// The innermost inline box containing boxes `left` and `right`; `None` is the root.
fn common_box(paragraph: &Paragraph, left: Option<usize>, right: Option<usize>) -> Option<usize> {
    let mut candidate = left;
    while let Some(index) = candidate {
        if right == Some(index) || paragraph.is_within(right, index) {
            return Some(index);
        }
        candidate = paragraph.boxes()[index].parent;
    }
    None
}

/// The `text-autospace` of a paragraph's root inline box, inline boxes and text spans.
pub(crate) struct Autospace<'p, 'c, 'f> {
    context: &'c TextContext<'f>,
    paragraph: &'p Paragraph,
    root: Spacing<'p>,
    boxes: Vec<Spacing<'p>>,
    spans: Vec<Spacing<'p>>,
}

impl<'p, 'c, 'f> Autospace<'p, 'c, 'f> {
    /// `None` when no boundary in `paragraph` can be spaced.
    pub(super) fn of(context: &'c TextContext<'f>, paragraph: &'p Paragraph) -> Option<Self> {
        let text = paragraph.text();
        let ideographs = text.chars().any(ideograph);
        let punctuation = text.chars().any(french_punctuation);
        let autospace = Self {
            context,
            paragraph,
            root: Spacing::new(paragraph.style()),
            boxes: paragraph
                .boxes()
                .iter()
                .map(|inline_box| Spacing::new(&inline_box.style))
                .collect(),
            spans: paragraph
                .spans()
                .iter()
                .map(|span| Spacing::new(&span.style))
                .collect(),
        };
        std::iter::once(&autospace.root)
            .chain(&autospace.boxes)
            .chain(&autospace.spans)
            .any(|spacing| spacing.active(ideographs, punctuation))
            .then_some(autospace)
    }

    /// Typographic character units of the arranged line in visual order; `None` separates
    /// units that do not adjoin.
    fn units(arranged: &Arranged) -> Vec<Option<Unit>> {
        let mut units = Vec::new();
        for (slot, entry) in arranged.order.iter().enumerate() {
            let Slot::Run(index) = *entry else {
                units.push(None);
                continue;
            };
            let run = &arranged.runs[index];
            if run.leader {
                units.push(None);
                continue;
            }
            for (glyph, positioned) in run.glyphs.iter().enumerate() {
                let range = &positioned.text_range;
                if run
                    .glyphs
                    .get(glyph + 1)
                    .is_some_and(|next| next.text_range == *range)
                {
                    continue;
                }
                let text = &run.text[range.clone()];
                units.push(text.chars().next().map(|base| Unit {
                    slot,
                    run: index,
                    glyph,
                    offset: run.source_range.start + range.start,
                    base,
                    span: run.source_span,
                    space: text.len() == SPACE.len_utf8() && base == SPACE,
                }));
            }
        }
        units
    }

    /// The spacing at the boundary of adjoining `left` and `right`, by the innermost element
    /// containing it, which also renders it.
    fn insertion<'u>(
        &self,
        left: &'u Unit,
        right: &Unit,
        removed: Option<&'u Unit>,
    ) -> Result<Option<Insertion<'u>>, TextError> {
        let spans = self.paragraph.spans();
        let (owner, spacing) = if left.span == right.span {
            (spans[left.span].inline_box, &self.spans[left.span])
        } else {
            let owner = common_box(
                self.paragraph,
                spans[left.span].inline_box,
                spans[right.span].inline_box,
            );
            (owner, owner.map_or(&self.root, |index| &self.boxes[index]))
        };
        if removed.is_some() && !spacing.value.replace {
            return Ok(None);
        }
        let (first, second) = if left.offset <= right.offset {
            (left.base, right.base)
        } else {
            (right.base, left.base)
        };
        let Some(gap) = spacing.gap(first, second) else {
            return Ok(None);
        };
        let width = spacing.width(self.context, gap)?;
        Ok((width != 0.0).then_some(Insertion {
            unit: left,
            removed,
            owner,
            width,
        }))
    }

    /// Spaces the adjoining units of `arranged`, before its inline box fragments are formed.
    pub(super) fn apply(&self, arranged: &mut Arranged) -> Result<(), TextError> {
        let units = Self::units(arranged);
        let mut insertions = Vec::new();
        for (index, left) in units.iter().enumerate() {
            let Some(left) = left else {
                continue;
            };
            let (right, removed) = match units.get(index + 1) {
                Some(Some(next)) if next.space => (units.get(index + 2), Some(next)),
                next => (next, None),
            };
            if let Some(Some(right)) = right
                && let Some(insertion) = self.insertion(left, right, removed)?
            {
                insertions.push(insertion);
            }
        }
        // Later slots first, so that inserted slots leave earlier positions in place.
        for insertion in insertions.into_iter().rev() {
            if let Some(space) = insertion.removed {
                let run = &mut arranged.runs[space.run];
                let range = run.glyphs[space.glyph].text_range.clone();
                for glyph in run
                    .glyphs
                    .iter_mut()
                    .filter(|glyph| glyph.text_range == range)
                {
                    run.width -= glyph.advance(run.font_size);
                    glyph.x_advance = 0.0;
                    glyph.x_spacing = 0.0;
                }
                run.removed_spaces.push(range.start);
            }
            let unit = insertion.unit;
            if self.paragraph.spans()[unit.span].inline_box == insertion.owner {
                let run = &mut arranged.runs[unit.run];
                run.glyphs[unit.glyph].x_spacing += insertion.width;
                run.width += insertion.width;
            } else {
                arranged.order.insert(
                    unit.slot + 1,
                    Slot::Space {
                        owner: insertion.owner,
                        width: insertion.width,
                    },
                );
            }
        }
        Ok(())
    }
}
