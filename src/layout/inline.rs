//! Inline-level boxes: atomic inline measurement (CSS 2.1 §10.3.9, §10.8.1) and the
//! painted border boxes of inline box fragments.
use crate::document::{BlockBox, InlineBox};
use crate::graphics::{
    BoxPaint, DecorationLine, DecorationPaint, DisplayItem, Point, Rect, TextShadowPaint,
};
use crate::layout::geometry::inline_horizontal;
use crate::layout::measure::Failure;
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

/// An initial letter's margin box against its originating line box (CSS Inline 3 §7.6):
/// that of its last line where its contents wrap (§7.8.4).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct LetterExtent {
    /// Its top and bottom below the line box's top.
    pub top: f32,
    pub bottom: f32,
    /// How far its inline-start edge is inset to kern its glyphs to the line's start
    /// (§7.6.2).
    pub kerning: f32,
    /// Its margin box's width, which its advance on the line and its kerning may not span.
    pub width: f32,
    /// How far above the originating line box lies that of its first line.
    pub lift: f32,
}
impl LetterExtent {
    /// How far the originating line moves down so that the letter stays below the
    /// containing block's block-start content edge (§7.6.1), and the line boxes of its
    /// earlier lines fit above it.
    pub(super) fn rise(&self) -> f32 {
        (-self.top).max(self.lift)
    }
}

/// One line of an inline initial letter's contents, placed as the letter's only line would
/// be (§7.8.4): its border box's top below the top of the line box it originates on, the
/// content box's extent above its line, and the content box's height from there.
struct LetterRow {
    top: f32,
    lead: f32,
    content: f32,
}

/// Measures an atomic inline, which is laid out unbroken.
pub(super) fn measure<'a>(
    measurer: &mut Measurer<'a>,
    block: &'a BlockBox,
    containing_width: f32,
    containing_height: Option<f32>,
    containing_rtl: bool,
) -> Result<MeasuredAtomic<'a>, Failure> {
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
    measurer: &mut Measurer<'a>,
    block: &'a BlockBox,
    containing_width: f32,
    containing_height: Option<f32>,
    containing_rtl: bool,
) -> Result<MeasuredAtomic<'a>, Failure> {
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
    Ok(measured_atomic(block, measured, &geometry))
}

/// Lays a ruby annotation box out again at the `width` its ruby column gives it, which its
/// contents fill by its `ruby-align` (CSS Ruby 1 §4.3): all of its contents, or the `part` of
/// them a line breaking inside its bases holds (§3.4.2).
pub(super) fn measure_annotation<'a>(
    measurer: &mut Measurer<'a>,
    (block, part): (&'a BlockBox, Option<std::ops::Range<usize>>),
    width: f32,
    containing_height: Option<f32>,
    containing_rtl: bool,
) -> Result<MeasuredAtomic<'a>, Failure> {
    let unbroken = std::mem::replace(&mut measurer.unbroken, true);
    measurer.annotation_part = part.and_then(|part| match block.children().first() {
        Some(crate::document::BlockContent::Inline(contents)) => {
            Some((std::ptr::from_ref(contents), part))
        }
        _ => None,
    });
    let geometry = inline_horizontal(
        block.style(),
        0.0,
        width,
        containing_rtl,
        Some(width),
        measurer.box_edges(block, width)?,
    );
    let measured = measurer.measure_format(block, geometry.clone(), false, containing_height, None);
    measurer.unbroken = unbroken;
    measurer.annotation_part = None;
    Ok(measured_atomic(block, measured?, &geometry))
}

/// Sets `metrics` to those of `paragraph`'s measured blocks (`Paragraph::measured_blocks`)
/// from its inline blocks as measured, `atomics`: those inside an annotation box as laying the
/// box out measured them. A block already placed keeps its own and its contents' metrics.
pub(super) fn paragraph_metrics(
    paragraph: &crate::document::Paragraph,
    atomics: &[Option<MeasuredAtomic<'_>>],
    metrics: &mut Vec<AtomicMetrics>,
) {
    metrics.resize(paragraph.measured_blocks().len(), AtomicMetrics::default());
    for (index, atomic) in atomics.iter().enumerate() {
        let Some(atomic) = atomic else {
            continue;
        };
        metrics[index] = atomic.metrics;
        let Some(annotation) = index.checked_sub(paragraph.atomics().len()) else {
            continue;
        };
        let nested = paragraph.annotation_metrics(annotation);
        if nested.is_empty() {
            continue;
        }
        let Some(contents) = atomic.block.items.iter().find_map(|item| match item {
            FlowItem::Paragraph(contents) => Some(contents),
            _ => None,
        }) else {
            unreachable!("an annotation box with measured blocks lays out its contents")
        };
        let mut inner = metrics[nested.clone()].to_vec();
        paragraph_metrics(contents.paragraph, &contents.atomics, &mut inner);
        metrics[nested].copy_from_slice(&inner);
    }
}

/// Sets `metrics` to those of `paragraph`'s measured blocks on its first formatted line:
/// `base`, those of its measured blocks, with those of its atomic inlines' `::first-line`
/// boxes as measured, `first_line`, in their place (CSS Pseudo-Elements 4 §2.1.3). A box
/// already placed keeps its metrics.
pub(super) fn first_line_metrics(
    paragraph: &crate::document::Paragraph,
    base: &[AtomicMetrics],
    first_line: &[Option<MeasuredAtomic<'_>>],
    metrics: &mut Vec<AtomicMetrics>,
) {
    metrics.resize(base.len(), AtomicMetrics::default());
    for (index, value) in metrics.iter_mut().enumerate() {
        if let Some(Some(atomic)) = first_line.get(index) {
            *value = atomic.metrics;
        } else if !has_first_line_box(paragraph, index) {
            *value = base[index];
        }
    }
}

/// Whether inline block `index` of `paragraph`, an atomic inline or ruby annotation box, has
/// a `::first-line` box, which its first formatted line lays out in its place.
pub(super) fn has_first_line_box(paragraph: &crate::document::Paragraph, index: usize) -> bool {
    paragraph
        .inline_blocks()
        .nth(index)
        .is_some_and(|block| block.block().first_line_box().is_some())
}

/// An atomic inline's metrics from its layout `measured` with horizontal `geometry`.
fn measured_atomic<'a>(
    block: &'a BlockBox,
    measured: MeasuredBlock<'a>,
    geometry: &crate::layout::geometry::HorizontalBox,
) -> MeasuredAtomic<'a> {
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
    MeasuredAtomic {
        metrics: AtomicMetrics {
            width,
            height,
            baseline: content.map_or(height, |baseline| margin[0] + baseline),
            synthesized_baseline: content.is_none(),
        },
        block: measured,
        margin_top: margin[0],
        letter: None,
    }
}

/// Measures an initial letter box (CSS Inline 3 §§7.5.5–7.6.2). Its alignment points sit at
/// the surrounding text's: the edges of an atomic inline's content box, or else those its
/// trimmed line box holds, or with `border-box` its border edges. An automatically sized
/// inline letter's content box fits its sink and its glyph outlines; a definite one aligns
/// its line by `align-content`.
fn measure_initial_letter<'a>(
    measurer: &mut Measurer<'a>,
    block: &'a BlockBox,
    letter: &crate::document::InitialLetterBox,
    containing_width: f32,
    containing_height: Option<f32>,
    containing_rtl: bool,
) -> Result<MeasuredAtomic<'a>, Failure> {
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
    // Where the line box of each line of an inline letter's contents lies below the first's.
    let mut origins: Vec<f32> = Vec::new();
    // The border box top below the line box top.
    let top = if letter.atomic {
        if border_box {
            surrounding.over_position(initial, measured.height)
        } else {
            surrounding.over_position(initial, measured.height - start - end) - start
        }
    } else {
        trim_letter_lines(measurer.text, &mut measured)?;
        let Some(FlowItem::Paragraph(paragraph)) = measured
            .items
            .iter()
            .find(|item| matches!(item, FlowItem::Paragraph(_)))
        else {
            unreachable!("an inline initial letter holds its contents' paragraph")
        };
        let automatic = *style.height.basis() == crate::style::Size::Auto;
        let sizing = LetterSizing {
            border_box,
            automatic,
            content: measured.height - start - end,
            edges: [start, end],
        };
        let mut rows = Vec::with_capacity(paragraph.lines.len());
        for (index, line) in paragraph.lines.iter().enumerate() {
            let outline = measurer.text.outline_bounds(paragraph.paragraph, line)?;
            rows.push(letter_row(letter, &sizing, style, line, outline));
            if index == 0
                && *style.width.basis() == crate::style::Size::Auto
                && (0..4).all(|side| {
                    style.padding[side].is_zero()
                        && style.decoration.borders[side].used_width() == 0.0
                })
                && let Some(rect) = outline
            {
                kerning = if style.used_rtl() {
                    line.width - (rect.x + rect.width)
                } else {
                    rect.x
                };
            }
        }
        // Each line's line box follows the margin box of the line before, moved down for
        // its letter line to stay below it (§§7.6.1, 7.8.4).
        let bottom = |row: &LetterRow| row.top + start + row.lead + row.content + end + margin[2];
        for (index, row) in rows.iter().enumerate() {
            let origin = match index.checked_sub(1) {
                Some(previous) => {
                    origins[previous] + bottom(&rows[previous]) + (margin[0] - row.top).max(0.0)
                }
                None => 0.0,
            };
            origins.push(origin);
        }
        let first = &rows[0];
        let last = &rows[rows.len() - 1];
        // The lines' tops below the first's.
        let line_tops: Vec<f32> = rows
            .iter()
            .zip(&origins)
            .map(|(row, origin)| origin + row.top + row.lead - first.top - first.lead)
            .collect();
        if automatic || first.lead != 0.0 || rows.len() > 1 {
            geometry.padding[0] += first.lead;
            measured = measurer.measure_format(
                block,
                geometry.clone(),
                false,
                containing_height,
                Some(line_tops[line_tops.len() - 1] + last.content),
            )?;
        }
        if rows.len() > 1 {
            trim_letter_lines(measurer.text, &mut measured)?;
            // Later fragments of a sliced letter lack its inline-start edges and kerning; cloned
            // ones keep its edges (CSS Fragmentation 3 §5.4).
            let indent = if style.clone_decoration {
                0.0
            } else {
                kerning - inline_start_edges(&geometry, style)
            };
            space_letter_lines(&mut measured, &line_tops, indent);
            let extents = row_extents(style, &geometry, letter_lines(&mut measured));
            let fragments: Vec<RowFragment> = rows
                .iter()
                .zip(&origins)
                .zip(&extents)
                .map(|((row, origin), &extent)| RowFragment {
                    top: origin + row.top - first.top,
                    height: start + row.lead + row.content + end,
                    lead: row.lead - first.lead,
                    extent,
                })
                .collect();
            if let Some(paint) = measured.paint.take() {
                let (carrier, pieces) =
                    letter_fragments(paint, &geometry, style, &fragments, indent);
                measured.paint = Some(carrier);
                measured.line_fragments = pieces;
            }
        }
        first.top - origins[origins.len() - 1]
    };
    let width = margin[3]
        + geometry.border[3]
        + geometry.padding[3]
        + geometry.width
        + geometry.padding[1]
        + geometry.border[1]
        + margin[1];
    // Normal text follows the last line, which starts at the line's start (§7.8.4).
    let advance = if origins.len() > 1 {
        let end_side = if style.used_rtl() { LEFT } else { RIGHT };
        let extent = row_extents(style, &geometry, letter_lines(&mut measured))
            .last()
            .copied()
            .unwrap_or(0.0);
        // A cloned last fragment keeps the letter's inline-start edges.
        let start = if style.clone_decoration {
            inline_start_edges(&geometry, style) - kerning
        } else {
            0.0
        };
        start + extent + geometry.padding[end_side] + geometry.border[end_side] + margin[end_side]
    } else {
        width - kerning
    };
    Ok(MeasuredAtomic {
        metrics: AtomicMetrics {
            width: advance,
            height: 0.0,
            baseline: 0.0,
            synthesized_baseline: false,
        },
        letter: Some(LetterExtent {
            top: top - margin[0],
            bottom: top + measured.height + margin[2],
            kerning,
            width,
            lift: origins.last().copied().unwrap_or(0.0),
        }),
        block: measured,
        margin_top: margin[0],
    })
}

/// How an inline initial letter's lines size its content box: whether its alignment is
/// `border-box`, whether its block size is automatic, its content box height otherwise, and
/// its block-start and block-end padding and borders.
struct LetterSizing {
    border_box: bool,
    automatic: bool,
    content: f32,
    edges: [f32; 2],
}

/// Places one line of an inline letter's contents, whose glyph outlines are `outline`, as the
/// letter's only line would be (§§7.5.5–7.6.1).
fn letter_row(
    letter: &crate::document::InitialLetterBox,
    sizing: &LetterSizing,
    style: &crate::style::Style,
    line: &crate::text::Line,
    outline: Option<Rect>,
) -> LetterRow {
    let initial = style
        .initial_letter
        .expect("an initial letter has a size and sink");
    let surrounding = letter.surrounding;
    let [start, end] = sizing.edges;
    // The outlines' extent below the line box top, which is the over alignment point.
    let [ink_top, ink_bottom] = outline.map_or([0.0, line.height], |rect| {
        [rect.y + line.ascent, rect.y + rect.height + line.ascent]
    });
    let ink_bottom = ink_bottom.max(line.height);
    // The content box's extent above the line box top: glyphs above the over point,
    // which only padding or borders bring into it, or `align-content`'s free space.
    let lead = if sizing.automatic {
        if start > 0.0 {
            (-ink_top).max(0.0)
        } else {
            0.0
        }
    } else {
        crate::flex::spacing(
            super::flex::alignment(style.flex.align_content, false),
            style.flex.align_content_overflow,
            sizing.content - line.height,
            1,
        )
        .0
    };
    // The content box's extent below the line box top that reaches the sink's under
    // point from a line box top at `line_top`, and covers the glyphs.
    let fitted = |line_top: f32| (surrounding.under_of(initial.sink) - line_top).max(ink_bottom);
    let (top, below) = if sizing.border_box {
        let natural = start + lead + ink_bottom + end;
        let top = surrounding.over_position(initial, natural);
        (top, fitted(top + start + lead + end))
    } else {
        let line_top = surrounding.over_position(initial, line.height);
        (line_top - lead - start, fitted(line_top))
    };
    LetterRow {
        top,
        lead,
        content: if sizing.automatic {
            below
        } else {
            sizing.content - lead
        },
    }
}

/// The inline-start margin, border and padding of a box with horizontal `geometry`.
fn inline_start_edges(
    geometry: &crate::layout::geometry::HorizontalBox,
    style: &crate::style::Style,
) -> f32 {
    let side = if style.used_rtl() { RIGHT } else { LEFT };
    geometry.margin[side].unwrap_or(0.0) + geometry.border[side] + geometry.padding[side]
}

/// The lines of an inline letter's `measured` contents.
fn letter_lines<'m>(measured: &'m mut MeasuredBlock<'_>) -> &'m mut Vec<crate::text::Line> {
    measured
        .items
        .iter_mut()
        .find_map(|item| match item {
            FlowItem::Paragraph(paragraph) => Some(&mut paragraph.lines),
            _ => None,
        })
        .expect("an inline initial letter holds its contents' paragraph")
}

/// Trims each line of an inline letter's `measured` contents at the ends its paragraph trims
/// only its first and last lines at, as the letter's only line would be (§7.8.4).
fn trim_letter_lines(
    text: &crate::text::TextContext<'_>,
    measured: &mut MeasuredBlock<'_>,
) -> Result<(), Failure> {
    let Some(FlowItem::Paragraph(paragraph)) = measured
        .items
        .iter_mut()
        .find(|item| matches!(item, FlowItem::Paragraph(_)))
    else {
        unreachable!("an inline initial letter holds its contents' paragraph")
    };
    let trim = paragraph.paragraph.text_box_trim;
    let last = paragraph.lines.len().saturating_sub(1);
    for (index, line) in paragraph.lines.iter_mut().enumerate() {
        crate::text::trim_line(
            text,
            paragraph.paragraph.style(),
            line,
            [trim[0] && index > 0, trim[1] && index < last],
        )?;
    }
    Ok(())
}

/// Spaces the lines of an inline letter's `measured` contents to their tops `line_tops`, and
/// moves those after the first `indent` from their start: as fragments of the letter after
/// its first, they lack its inline-start edges and kerning.
fn space_letter_lines(measured: &mut MeasuredBlock<'_>, line_tops: &[f32], indent: f32) {
    let lines = letter_lines(measured);
    for (index, line) in lines.iter_mut().enumerate() {
        if let Some(next) = line_tops.get(index + 1) {
            line.height = next - line_tops[index];
        }
        if index > 0 {
            line.indent = indent;
        }
    }
    let height = lines.iter().map(|line| line.height).sum();
    if let Some(FlowItem::Paragraph(paragraph)) = measured
        .items
        .iter_mut()
        .find(|item| matches!(item, FlowItem::Paragraph(_)))
    {
        paragraph.height = height;
    }
}

/// The content extent of each line fragment of an inline letter with `lines`: its line's
/// width, or under a definite inline size the content box's (§7.5.5).
fn row_extents(
    style: &crate::style::Style,
    geometry: &crate::layout::geometry::HorizontalBox,
    lines: &[crate::text::Line],
) -> Vec<f32> {
    let automatic = *style.width.basis() == crate::style::Size::Auto;
    lines
        .iter()
        .map(|line| {
            if automatic {
                line.width
            } else {
                geometry.width
            }
        })
        .collect()
}

/// One line fragment of an inline letter, in its whole box's coordinates.
struct RowFragment {
    /// Its border box's top below the whole box's.
    top: f32,
    height: f32,
    /// How much more of its content box than of the first fragment's lies above its line.
    lead: f32,
    extent: f32,
}

/// Splits the paint of an inline letter whose contents wrap into one per line fragment
/// (§7.8.4), sliced at the line breaks or cloned as `box-decoration-break` says (CSS
/// Fragmentation 3 §5.4); fragments after the first are moved `indent` from the content
/// box's start. The whole box keeps an inkless paint, which carries its masks, clips and
/// the effects of its stacking context.
fn letter_fragments(
    paint: BoxPaint,
    geometry: &crate::layout::geometry::HorizontalBox,
    style: &crate::style::Style,
    rows: &[RowFragment],
    indent: f32,
) -> (BoxPaint, Vec<BoxPaint>) {
    let rtl = style.used_rtl();
    let last = rows.len() - 1;
    let edge = |side: usize| geometry.padding[side] + geometry.border[side];
    let mut decoration = paint.decoration.clone();
    decoration.masks = std::sync::Arc::default();
    let fragments = rows
        .iter()
        .enumerate()
        .map(|(index, row)| {
            let start = index == 0 || style.clone_decoration;
            let end = index == last || style.clone_decoration;
            let inline_edges = if rtl { [end, start] } else { [start, end] };
            let offset = if index == 0 { 0.0 } else { indent };
            let left = if rtl {
                geometry.x + geometry.width - offset - row.extent
            } else {
                geometry.x + offset
            };
            let x = left - if inline_edges[0] { edge(LEFT) } else { 0.0 };
            let right = left + row.extent + if inline_edges[1] { edge(RIGHT) } else { 0.0 };
            let mut padding = paint.padding;
            padding[TOP] += row.lead;
            let mut margins = paint.margins;
            for (side, kept) in [LEFT, RIGHT].into_iter().zip(inline_edges) {
                if !kept {
                    margins[side] = 0.0;
                }
            }
            BoxPaint {
                rect: Rect {
                    x,
                    y: row.top,
                    width: right - x,
                    height: row.height,
                },
                decoration: decoration.clone(),
                padding,
                margins,
                inline_edges,
                ..paint.clone()
            }
        })
        .collect();
    let borders = paint
        .decoration
        .borders
        .map(|border| crate::graphics::Border {
            color: crate::graphics::Color::TRANSPARENT,
            ..border
        });
    let carrier = BoxPaint {
        decoration: crate::graphics::BoxDecoration {
            borders,
            radii: paint.decoration.radii.clone(),
            corner_shapes: paint.decoration.corner_shapes.clone(),
            masks: paint.decoration.masks.clone(),
            ..crate::graphics::BoxDecoration::default()
        },
        ..paint
    };
    (carrier, fragments)
}

/// Shared shrink-to-fit geometry for atomic inlines and floating boxes.
pub(super) fn measure_shrink_to_fit<'a>(
    measurer: &mut Measurer<'a>,
    block: &'a BlockBox,
    containing_width: f32,
    containing_height: Option<f32>,
    containing_rtl: bool,
) -> Result<(MeasuredBlock<'a>, crate::layout::geometry::HorizontalBox), Failure> {
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
            measurer.measure_format(block, geometry.clone(), false, containing_height, None)?;
        (measured, geometry)
    };
    Ok(result)
}

pub(super) fn shrink_to_fit_geometry<'a>(
    measurer: &Measurer<'a>,
    block: &'a BlockBox,
    containing_width: f32,
    containing_height: Option<f32>,
    containing_rtl: bool,
) -> Result<crate::layout::geometry::HorizontalBox, Failure> {
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
        measurer.box_edges(block, containing_width)?,
    );
    Ok(geometry)
}

/// `width: auto` resolves to min(max(min-content, available), max-content), or through a
/// preferred aspect ratio from a definite height (CSS Sizing 4 §4.2).
pub(super) fn shrink_to_fit<'a>(
    measurer: &Measurer<'a>,
    block: &'a BlockBox,
    containing_width: f32,
    containing_height: Option<f32>,
) -> Result<f32, Failure> {
    let style = block.style();
    let edges = measurer.box_edges(block, containing_width)?;
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
        constraints.preferred(sizes.max.min(sizes.min.max(available)))
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
    if style.is_scroll_container() || style.containment().layout {
        return None;
    }
    if block.image().is_some() {
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
    containing_width: f32,
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
    // `auto` margins of inline boxes are zero; inline-axis ones belong to the edge fragments.
    let margins = std::array::from_fn(|side| {
        let edge = match side {
            LEFT => fragment.edges[0],
            RIGHT => fragment.edges[1],
            _ => true,
        };
        if edge {
            inline_box.style.margin[side]
                .resolve(containing_width)
                .unwrap_or(0.0)
        } else {
            0.0
        }
    });
    Some(BoxPaint {
        padding,
        margins,
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
