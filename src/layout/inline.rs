//! Inline-level boxes: atomic inline measurement (CSS 2.1 §10.3.9, §10.8.1) and the
//! painted border boxes of inline box fragments.
use crate::document::{BlockBox, InlineBox};
use crate::graphics::{
    BoxPaint, DecorationLine, DecorationPaint, DisplayItem, Point, Rect, TextShadowPaint,
};
use crate::layout::LayoutError;
use crate::layout::geometry::{Edges, inline_horizontal};
use crate::layout::measure::{FlowItem, MeasuredBlock, Measurer};
use crate::style::BaselineSource;
use crate::text::{AtomicMetrics, BoxFragment, GlyphRun};

/// An atomic inline measured in its containing block, waiting for its line position.
#[derive(Clone)]
pub(super) struct MeasuredAtomic<'a> {
    /// Laid out with its left margin edge at x = 0.
    pub block: MeasuredBlock<'a>,
    pub metrics: AtomicMetrics,
    pub margin_top: f32,
    /// An initial letter's place against its originating line, which it does not grow.
    pub letter: Option<LetterExtent>,
}

/// An initial letter's margin box against its originating line box (CSS Inline 3 §7.6).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct LetterExtent {
    /// Its top and bottom below the line box's top.
    pub top: f32,
    pub bottom: f32,
    /// How far its inline-start edge is inset to kern its glyphs to the line's start
    /// (§7.6.2); the advance it takes on the line is its width less this.
    pub kerning: f32,
}
impl LetterExtent {
    /// How far the originating line moves down so that the letter stays below the
    /// containing block's block-start content edge (§7.6.1).
    pub(super) fn rise(&self) -> f32 {
        (-self.top).max(0.0)
    }
}

/// Measures an atomic inline, which is laid out unbroken.
pub(super) fn measure<'a>(
    measurer: &mut Measurer<'_>,
    block: &'a BlockBox,
    containing_width: f32,
    containing_height: Option<f32>,
    containing_rtl: bool,
) -> Result<MeasuredAtomic<'a>, LayoutError> {
    let unbroken = std::mem::replace(&mut measurer.unbroken, true);
    let measured = measure_atomic(
        measurer,
        block,
        containing_width,
        containing_height,
        containing_rtl,
    );
    measurer.unbroken = unbroken;
    measured
}

fn measure_atomic<'a>(
    measurer: &mut Measurer<'_>,
    block: &'a BlockBox,
    containing_width: f32,
    containing_height: Option<f32>,
    containing_rtl: bool,
) -> Result<MeasuredAtomic<'a>, LayoutError> {
    if let Some(letter) = block.initial_letter() {
        return measure_initial_letter(
            measurer,
            block,
            letter,
            containing_width,
            containing_height,
            containing_rtl,
        );
    }
    let (measured, geometry) = measure_shrink_to_fit(
        measurer,
        block,
        containing_width,
        containing_height,
        containing_rtl,
    )?;
    let margin = geometry.margin.map(|margin| margin.unwrap_or(0.0));
    let width = margin[3]
        + geometry.border[3]
        + geometry.padding[3]
        + geometry.width
        + geometry.padding[1]
        + geometry.border[1]
        + margin[1];
    let height = margin[0] + measured.height + margin[2];
    let content = baseline(block, &measured);
    Ok(MeasuredAtomic {
        metrics: AtomicMetrics {
            width,
            height,
            baseline: content.map_or(height, |baseline| margin[0] + baseline),
            synthesized_baseline: content.is_none(),
        },
        block: measured,
        margin_top: margin[0],
        letter: None,
    })
}

/// Measures an initial letter box (CSS Inline 3 §§7.5.5–7.6.2). Its alignment points sit at
/// the surrounding text's: the edges of an atomic inline's content box, or else those its
/// trimmed line box holds, or with `border-box` its border edges. An automatically sized
/// inline letter's content box fits its sink and its glyph outlines; a definite one aligns
/// its line by `align-content`.
fn measure_initial_letter<'a>(
    measurer: &mut Measurer<'_>,
    block: &'a BlockBox,
    letter: &crate::document::InitialLetterBox,
    containing_width: f32,
    containing_height: Option<f32>,
    containing_rtl: bool,
) -> Result<MeasuredAtomic<'a>, LayoutError> {
    let style = block.style();
    let initial = style
        .initial_letter
        .expect("an initial letter has a size and sink");
    let border_box = style.initial_letter_align.border_box;
    let surrounding = letter.surrounding;
    let (mut measured, mut geometry) = measure_shrink_to_fit(
        measurer,
        block,
        containing_width,
        containing_height,
        containing_rtl,
    )?;
    let margin = geometry.margin.map(|margin| margin.unwrap_or(0.0));
    let start = geometry.padding[0] + geometry.border[0];
    let end = geometry.padding[2] + geometry.border[2];
    let mut kerning = 0.0;
    // The border box top below the line box top.
    let top = if letter.atomic {
        if border_box {
            surrounding.over_position(initial, measured.height)
        } else {
            surrounding.over_position(initial, measured.height - start - end) - start
        }
    } else {
        let Some(FlowItem::Paragraph(paragraph)) = measured
            .items
            .iter()
            .find(|item| matches!(item, FlowItem::Paragraph(_)))
        else {
            unreachable!("an inline initial letter holds its contents' paragraph")
        };
        let [line] = paragraph.lines.as_slice() else {
            return Err(LayoutError::UnsupportedInitialLetter {
                feature: "contents wrapping onto several lines",
            });
        };
        let outline = measurer.text.outline_bounds(paragraph.paragraph, line)?;
        // The outlines' extent below the line box top, which is the over alignment point.
        let [ink_top, ink_bottom] = outline.map_or([0.0, line.height], |rect| {
            [rect.y + line.ascent, rect.y + rect.height + line.ascent]
        });
        let ink_bottom = ink_bottom.max(line.height);
        let automatic = style.height == crate::style::Size::Auto;
        // The content box's extent above the line box top: glyphs above the over point,
        // which only padding or borders bring into it, or `align-content`'s free space.
        let lead = if automatic {
            if start > 0.0 {
                (-ink_top).max(0.0)
            } else {
                0.0
            }
        } else {
            let content = measured.height - start - end;
            crate::flex::spacing(
                super::flex::alignment(style.flex.align_content, false),
                content - line.height,
                1,
            )
            .0
        };
        // The content box's extent below the line box top that reaches the sink's under
        // point from a line box top at `line_top`, and covers the glyphs.
        let fitted =
            |line_top: f32| (surrounding.under_of(initial.sink) - line_top).max(ink_bottom);
        let (top, below) = if border_box {
            let natural = start + lead + ink_bottom + end;
            let top = surrounding.over_position(initial, natural);
            (top, fitted(top + start + lead + end))
        } else {
            let line_top = surrounding.over_position(initial, line.height);
            (line_top - lead - start, fitted(line_top))
        };
        if style.width == crate::style::Size::Auto
            && (0..4).all(|side| {
                style.padding[side].is_zero() && style.decoration.borders[side].used_width() == 0.0
            })
            && let Some(rect) = outline
        {
            kerning = if style.rtl {
                line.width - (rect.x + rect.width)
            } else {
                rect.x
            };
        }
        if automatic || lead != 0.0 {
            let content = if automatic {
                below
            } else {
                measured.height - start - end - lead
            };
            geometry.padding[0] += lead;
            measured = measurer.format_block(
                block,
                geometry.clone(),
                false,
                containing_height,
                Some(content),
            )?;
        }
        top
    };
    let width = margin[3]
        + geometry.border[3]
        + geometry.padding[3]
        + geometry.width
        + geometry.padding[1]
        + geometry.border[1]
        + margin[1];
    Ok(MeasuredAtomic {
        metrics: AtomicMetrics {
            width: width - kerning,
            height: 0.0,
            baseline: 0.0,
            synthesized_baseline: false,
        },
        letter: Some(LetterExtent {
            top: top - margin[0],
            bottom: top + measured.height + margin[2],
            kerning,
        }),
        block: measured,
        margin_top: margin[0],
    })
}

/// Shared shrink-to-fit geometry for atomic inlines and floating boxes.
pub(super) fn measure_shrink_to_fit<'a>(
    measurer: &mut Measurer<'_>,
    block: &'a BlockBox,
    containing_width: f32,
    containing_height: Option<f32>,
    containing_rtl: bool,
) -> Result<(MeasuredBlock<'a>, crate::layout::geometry::HorizontalBox), LayoutError> {
    let result = if block.style().display.is_table() {
        crate::layout::tables::measure_placed(
            measurer,
            block,
            0.0,
            containing_width,
            containing_rtl,
            containing_height,
            inline_horizontal,
        )?
    } else {
        let geometry = shrink_to_fit_geometry(
            measurer,
            block,
            containing_width,
            containing_height,
            containing_rtl,
        )?;
        let measured =
            measurer.format_block(block, geometry.clone(), false, containing_height, None)?;
        (measured, geometry)
    };
    Ok(result)
}

pub(super) fn shrink_to_fit_geometry(
    measurer: &Measurer<'_>,
    block: &BlockBox,
    containing_width: f32,
    containing_height: Option<f32>,
    containing_rtl: bool,
) -> Result<crate::layout::geometry::HorizontalBox, LayoutError> {
    let style = block.style();
    let width = if block.image().is_some() {
        crate::layout::images::size(
            block,
            containing_width,
            containing_width,
            containing_height,
            None,
            None,
        )[0]
    } else {
        shrink_to_fit(measurer, block, containing_width, containing_height)?
    };
    let geometry = inline_horizontal(
        style,
        0.0,
        containing_width,
        containing_rtl,
        Some(width),
        Edges::of(style, containing_width),
    );
    Ok(geometry)
}

/// `width: auto` resolves to min(max(min-content, available), max-content), or through a
/// preferred aspect ratio from a definite height (CSS Sizing 4 §4.2).
fn shrink_to_fit(
    measurer: &Measurer<'_>,
    block: &BlockBox,
    containing_width: f32,
    containing_height: Option<f32>,
) -> Result<f32, LayoutError> {
    let style = block.style();
    let edges = Edges::of(style, containing_width);
    if let Some(width) = crate::layout::ratio::width_from_height(
        measurer,
        block,
        containing_width,
        containing_height,
        &edges,
    )? {
        return Ok(width);
    }
    let box_edges = edges;
    let provisional = inline_horizontal(style, 0.0, containing_width, false, None, edges);
    let edges = edges.horizontal();
    let available = (containing_width
        - edges
        - provisional.margin[1].unwrap_or(0.0)
        - provisional.margin[3].unwrap_or(0.0))
    .max(0.0);
    let mut constraints = crate::layout::intrinsic::width_constraints(
        block,
        measurer,
        Some(containing_width),
        edges,
        Some(available),
    )?;
    crate::layout::ratio::transfer_to_width(style, &mut constraints, containing_height, &box_edges);
    let width = if let Some(width) = constraints.specified {
        width
    } else {
        let sizes = crate::layout::intrinsic::content(block, measurer, containing_width)?;
        sizes.max.min(sizes.min.max(available))
    };
    Ok(constraints.constrain(width))
}

/// Baseline below the border-box top; `None` uses the bottom margin edge. `baseline-source`
/// chooses the first or last baseline set; `auto` is the last line box for inline blocks and
/// the first baseline for other atomic containers (CSS Inline 3 §4.2.1); replaced elements,
/// scroll containers (CSS Align 3 §9.1) and layout containment boxes (CSS Containment 2
/// §3.2) have none.
fn baseline(block: &BlockBox, measured: &MeasuredBlock<'_>) -> Option<f32> {
    let style = block.style();
    if block.image().is_some() || style.is_scroll_container() || style.containment().layout {
        return None;
    }
    match style.baseline.source {
        BaselineSource::Last => measured.last_baseline(),
        BaselineSource::First => measured.first_baseline(),
        BaselineSource::Auto if style.display.is_inline_block() => measured.last_baseline(),
        BaselineSource::Auto => measured.first_baseline(),
    }
}

const TOP: usize = 0;
const RIGHT: usize = 1;
const BOTTOM: usize = 2;
const LEFT: usize = 3;

/// Border box of an inline box fragment relative to the line's left edge and baseline, with
/// its resolved padding. Vertical padding and borders surround the content area without
/// affecting the line height.
pub(super) fn fragment_border_box(
    inline_box: &InlineBox,
    fragment: &BoxFragment,
    containing_width: f32,
) -> (Rect, [f32; 4]) {
    let style = &inline_box.style;
    let padding = style
        .padding
        .each_ref()
        .map(|padding| padding.resolve(containing_width));
    let outside = |side: usize| padding[side] + style.decoration.borders[side].used_width();
    let top = fragment.baseline - fragment.ascent - outside(TOP);
    let bottom = fragment.baseline + fragment.descent + outside(BOTTOM);
    let rect = Rect {
        x: fragment.x,
        y: top,
        width: fragment.width,
        height: bottom - top,
    };
    (rect, padding)
}

/// Content box of an inline box fragment with border box `border` and resolved `padding`:
/// the inline-axis borders and padding belong only to the fragments holding the box's edges.
pub(super) fn fragment_content_box(
    inline_box: &InlineBox,
    fragment: &BoxFragment,
    (border, padding): (Rect, [f32; 4]),
) -> Rect {
    let borders = &inline_box.style.decoration.borders;
    let inset = |side: usize| padding[side] + borders[side].used_width();
    let left = if fragment.edges[0] { inset(LEFT) } else { 0.0 };
    let right = if fragment.edges[1] { inset(RIGHT) } else { 0.0 };
    Rect {
        x: border.x + left,
        y: border.y + inset(TOP),
        width: (border.width - left - right).max(0.0),
        height: (border.height - inset(TOP) - inset(BOTTOM)).max(0.0),
    }
}

/// Paint of an inline box fragment with border box `rect`, when it paints or carries
/// navigation.
pub(super) fn fragment_paint(
    inline_box: &InlineBox,
    fragment: &BoxFragment,
    (rect, padding): (Rect, [f32; 4]),
) -> Option<BoxPaint> {
    let mut decoration = inline_box.decoration.clone();
    if inline_box.canvas_source {
        decoration.clear_background();
    }
    // Masks and referenced filters measure the fragment even when it paints nothing.
    if !decoration.is_visible()
        && inline_box.navigation.is_empty()
        && !decoration.masks.is_masked()
        && !inline_box.style.is_filtered()
    {
        return None;
    }
    Some(BoxPaint {
        padding,
        inline_edges: fragment.edges,
        ..BoxPaint::whole(rect, decoration)
    })
}

/// Runs this close together share one shadow.
const SHADOW_JOIN_TOLERANCE_PT: f32 = 1.0e-3;

/// `text-shadow` layers of a line at its left edge and baseline `origin`, bottom-most
/// first: one per layer of each stretch of adjacent runs with the same shadows, covering
/// them and the decorations that `decorates` draws over them (CSS Text Decoration 3 §4).
pub(super) fn shadow_items(
    runs: &[GlyphRun],
    decorations: &[DecorationPaint],
    origin: Point,
    decorates: &dyn Fn(&GlyphRun, &DecorationPaint) -> bool,
) -> Vec<DisplayItem> {
    let mut items = Vec::new();
    let mut start = 0;
    while start < runs.len() {
        let shadows = &runs[start].shadows;
        let mut end = start + 1;
        while end < runs.len()
            && runs[end].shadows == *shadows
            && (runs[end].x - runs[end - 1].x - runs[end - 1].width).abs()
                <= SHADOW_JOIN_TOLERANCE_PT
        {
            end += 1;
        }
        let group = &runs[start..end];
        start = end;
        if shadows.is_empty() {
            continue;
        }
        let extent = [
            group[0].x,
            group[group.len() - 1].x + group[group.len() - 1].width,
        ];
        let decorations: Vec<DecorationPaint> = decorations
            .iter()
            .filter(|decoration| {
                decoration.rect.x < extent[1]
                    && decoration.rect.x + decoration.rect.width > extent[0]
                    && group.iter().any(|run| decorates(run, decoration))
            })
            .map(|decoration| DecorationPaint {
                rect: Rect {
                    x: decoration.rect.x + origin[0],
                    y: decoration.rect.y + origin[1],
                    ..decoration.rect
                },
                ..*decoration
            })
            .collect();
        for shadow in shadows.iter().rev() {
            if shadow.color.used.alpha <= 0.0 {
                continue;
            }
            items.push(DisplayItem::TextShadow(TextShadowPaint {
                origin,
                runs: group.to_vec(),
                decorations: decorations.clone(),
                extent: extent.map(|x| x + origin[0]),
                offset: shadow.offset,
                blur: shadow.blur,
                color: shadow.color.used,
            }));
        }
    }
    items
}

/// A line's decorations at its left edge `x` and `baseline`: those painted below its text,
/// and the line-throughs painted over it (CSS 2.1 Appendix E.2).
pub(super) fn decoration_items(
    decorations: &[DecorationPaint],
    x: f32,
    baseline: f32,
) -> [Vec<DisplayItem>; 2] {
    let (above, below): (Vec<_>, Vec<_>) = decorations
        .iter()
        .map(|decoration| DecorationPaint {
            rect: Rect {
                x: decoration.rect.x + x,
                y: decoration.rect.y + baseline,
                ..decoration.rect
            },
            ..*decoration
        })
        .partition(|decoration| decoration.line == DecorationLine::LineThrough);
    [below, above].map(|items| items.into_iter().map(DisplayItem::Decoration).collect())
}
