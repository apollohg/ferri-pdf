use crate::document::{BlockBox, BoxNavigation};
use crate::layout::measure::Failure;
use crate::layout::measure::{FlowItem, MeasuredBlock, Measurer};
use crate::layout::{breaks, geometry, intrinsic, margin::MarginStrut};
use crate::style::{BreakBetween, CaptionSide};

pub(crate) fn principal<'m, 'a>(table: &'m mut MeasuredBlock<'a>) -> &'m mut MeasuredBlock<'a> {
    if table.table.is_some() {
        return table;
    }
    let source = table.source;
    table
        .items
        .iter_mut()
        .find_map(|item| match item {
            FlowItem::Block(child)
                if std::ptr::eq(child.source, source) && child.table.is_some() =>
            {
                Some(child.as_mut())
            }
            _ => None,
        })
        .expect("a caption wrapper retains its principal table")
}

pub(super) fn minimum<'a>(
    captions: &[&'a BlockBox],
    measurer: &Measurer<'a>,
    available: f32,
) -> Result<f32, Failure> {
    let mut minimum: f32 = 0.0;
    for caption in captions {
        minimum = minimum.max(intrinsic::measure(*caption, measurer, available)?.min);
    }
    Ok(minimum)
}

pub(super) fn wrap<'a>(
    measurer: &mut Measurer<'a>,
    mut table: MeasuredBlock<'a>,
    captions: &[&'a BlockBox],
    geometry: &geometry::HorizontalBox,
) -> Result<MeasuredBlock<'a>, Failure> {
    if captions.is_empty() {
        return Ok(table);
    }
    let style = table.style;
    let source = table.source;
    let before = table.before;
    let after = table.after;
    table.before = MarginStrut::default();
    table.after = MarginStrut::default();
    table.break_before = BreakBetween::Auto;
    table.break_after = BreakBetween::Auto;
    table.has_forced = false;
    let x = geometry.x - geometry.padding[3] - geometry.border[3];
    let width = geometry.width
        + geometry.padding[1]
        + geometry.padding[3]
        + geometry.border[1]
        + geometry.border[3];
    let mut top = Vec::new();
    let mut bottom = Vec::new();
    for &source in captions {
        let caption_geometry = crate::layout::measure::block_geometry(
            measurer,
            source,
            x,
            width,
            style.used_rtl(),
            None,
        )?;
        let caption = measurer.measure_format(source, caption_geometry, true, None, None)?;
        match source.style().table.caption_side {
            CaptionSide::Top => top.push(caption),
            CaptionSide::Bottom => bottom.push(caption),
        }
    }
    let mut items = Vec::new();
    let mut pending = MarginStrut::default();
    let mut has_forced = false;
    let mut previous_break_after = BreakBetween::Auto;
    for child in top.into_iter().chain([table]).chain(bottom) {
        has_forced |= child.has_forced;
        pending.merge(
            if previous_break_after.is_forced() || child.break_before.is_forced() {
                child.before.clone().after_forced_break()
            } else {
                child.before.clone()
            },
        );
        items.push(FlowItem::Margin(pending));
        pending = child.after.clone();
        previous_break_after = child.break_after;
        let following = source.following_events(child.source);
        items.push(FlowItem::Block(Box::new(child)));
        items.extend(following.map(|event| FlowItem::Events(std::slice::from_ref(event))));
    }
    items.push(FlowItem::Margin(pending));
    let height = items.iter().map(FlowItem::height).sum::<f32>().max(0.0);
    let overflow_height = crate::layout::measure::overflow_height(&items).max(height);
    let edges = measurer.fragmentainer.edges(style);
    let first_fragment = if measurer.fragmentainer.keeps(style)
        && !has_forced
        && overflow_height <= measurer.page_height
    {
        overflow_height
    } else {
        breaks::first_fragment(&items).min(height)
    };
    let (break_before, break_after) = breaks::boundaries(style, &items, edges);
    Ok(MeasuredBlock {
        frame_key: None,
        frame_owner: None,
        grid: None,
        clearance_before: None,
        clearance_target: measurer
            .clearance_targets
            .get(&std::ptr::from_ref::<BlockBox>(source))
            .copied(),
        source,
        geometry: geometry::HorizontalBox {
            x,
            width,
            padding: [0.0; 4],
            border: [0.0; 4],
            ..geometry.clone()
        },
        containing_height: None,
        offset: [0.0; 2],
        style,
        canvas_source: false,
        before,
        after,
        through: false,
        height,
        sizing: None,
        constrained: false,
        independent: true,
        overflow_height,
        first_fragment,
        has_forced: has_forced || edges.iter().any(|edge| edge.is_forced()),
        break_before,
        break_after,
        items,
        paint: None,
        // The table box inside carries the element's navigation.
        navigation: BoxNavigation::default(),
        end_space: 0.0,
        start_space: 0.0,
        repeat_space: 0.0,
        marker: None,
        table: None,
        continues: false,
        spanner_fragment: None,
    })
}
