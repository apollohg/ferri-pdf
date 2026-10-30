//! The boxes of atomic inlines on a block's first formatted line, whose contents inherit
//! through `::first-line` (CSS Pseudo-Elements 4 §2.1.3) and are formatted again from the
//! element's start. A copy is formatted once for each way the line styles its element and
//! shared by every box the line restyles alike, including the boxes formatted alike inside
//! other copies; the copies that a copy requests are formatted after it rather than on its
//! stack, so their nesting is bounded by the formatting depth alone.
use super::{BlockBox, BlockContent, Frame, ImageResources, MarkerScroller, Traversal};
use super::{TraversalKind, block};
use crate::document::{DocumentError, first_line::FirstLineStyles};
use crate::dom::ElementId;
use crate::style::{Style, Stylesheet};
use std::cell::{OnceCell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;

/// A box as its block's first formatted line styles it: staged while the document formats,
/// then resolved with the writing modes of the boxes around it, after which every box that
/// the line restyles alike shares it unchanged.
#[derive(Default)]
pub(super) struct FirstLineBox {
    staged: RefCell<Option<BlockBox>>,
    resolved: OnceCell<BlockBox>,
}
impl FirstLineBox {
    pub(super) fn staged(block: BlockBox) -> Rc<Self> {
        let copy = Rc::new(Self::default());
        copy.stage(block);
        copy
    }
    pub(super) fn get(&self) -> &BlockBox {
        self.resolved
            .get()
            .expect("formatting resolves the first-line boxes of the boxes it keeps")
    }
    fn formatted(&self) -> bool {
        self.staged.borrow().is_some() || self.resolved.get().is_some()
    }
    /// Stages `block`, an atomic inline's box whose first and last formatted lines no ancestor
    /// trims (CSS Inline 3 §6.2).
    fn stage(&self, mut block: BlockBox) {
        block.trim_text_boxes([false; 2]);
        *self.staged.borrow_mut() = Some(block);
    }
    /// Resolves the staged box with `resolve`, unless another box sharing it already has.
    pub(super) fn resolve(&self, resolve: impl FnOnce(&mut BlockBox)) {
        let Some(mut block) = self.staged.borrow_mut().take() else {
            return;
        };
        resolve(&mut block);
        if self.resolved.set(block).is_err() {
            unreachable!("a first-line box is staged until it resolves");
        }
    }
}

/// The block whose lines an atomic inline begins as an initial letter (CSS Inline 3 §7.5),
/// which size and align it.
#[derive(Clone, PartialEq)]
pub(super) struct LetterBlock {
    pub element: ElementId,
    pub style: Style,
}

/// What an atomic inline's first-line copy is formatted from besides the document's state
/// at its element, the same wherever that element is formatted: the style the line gives it
/// and the block whose initial letter it is.
#[derive(PartialEq)]
pub(super) struct CopyKey {
    element: ElementId,
    style: Style,
    letter: Option<LetterBlock>,
}

/// The first-line copy that the box of an atomic inline formatted inside a first-line copy
/// also is.
pub(super) enum OwnCopy {
    /// The box is formatted from what the copy `.0` is formatted from.
    Key(Box<CopyKey>),
    /// The box moved into that copy and stands in for it.
    Moved(Rc<FirstLineBox>),
}

/// The first-line copies of a formatting, by what each is formatted from.
#[derive(Default)]
pub(super) struct FirstLineCopies(HashMap<ElementId, Vec<(CopyKey, Rc<FirstLineBox>)>>);
impl FirstLineCopies {
    fn find(&self, key: &CopyKey) -> Option<&Rc<FirstLineBox>> {
        self.0
            .get(&key.element)?
            .iter()
            .find_map(|(other, copy)| (other == key).then_some(copy))
    }
    fn insert(&mut self, key: CopyKey, copy: Rc<FirstLineBox>) {
        self.0.entry(key.element).or_default().push((key, copy));
    }
    /// Moves the boxes inside `root` that are first-line copies of their own into those
    /// copies, which are then not formatted again, leaving boxes that stand in for them.
    fn share_own_copies(&mut self, root: &mut BlockBox) {
        let mut moved = Vec::new();
        self.move_own_copies(root, &mut moved);
        while let Some((copy, mut block)) = moved.pop() {
            self.move_own_copies(&mut block, &mut moved);
            if !copy.formatted() {
                copy.stage(block);
            }
        }
    }
    /// Replaces the boxes inside `root`, outside those replaced, that are first-line copies of
    /// their own with stand-ins, adding each to `moved` with its copy.
    fn move_own_copies(
        &mut self,
        root: &mut BlockBox,
        moved: &mut Vec<(Rc<FirstLineBox>, BlockBox)>,
    ) {
        let mut pending = vec![root];
        while let Some(block) = pending.pop() {
            let paragraphs = block
                .children
                .iter_mut()
                .filter_map(|child| match child {
                    BlockContent::Block(child) | BlockContent::Positioned(child) => {
                        pending.push(child);
                        None
                    }
                    BlockContent::Inline(paragraph) => Some(paragraph),
                    BlockContent::Events(_) => None,
                })
                .collect::<Vec<_>>();
            if let Some(content) = &mut block.math_content {
                pending.push(content);
            }
            for anchored in paragraphs
                .into_iter()
                .flat_map(crate::document::Paragraph::anchored_blocks_mut)
            {
                let Some(OwnCopy::Key(key)) = anchored
                    .own_copy
                    .take_if(|own| matches!(own, OwnCopy::Key(_)))
                else {
                    pending.push(anchored);
                    continue;
                };
                let copy = if let Some(copy) = self.find(&key) {
                    copy.clone()
                } else {
                    let copy = Rc::new(FirstLineBox::default());
                    self.insert(*key, copy.clone());
                    copy
                };
                let stand_in = anchored.stand_in(copy.clone());
                moved.push((copy, std::mem::replace(anchored, stand_in)));
            }
        }
    }
}

impl BlockBox {
    /// A box standing in for `copy`, which holds this box or one formatted alike.
    fn stand_in(&self, copy: Rc<FirstLineBox>) -> Self {
        let mut stand_in = Self::anonymous(self.style().clone(), Vec::new(), None);
        stand_in.own_copy = Some(OwnCopy::Moved(copy));
        stand_in
    }
}

/// A copy waiting to be formatted from `root`, with the document's state at its start.
pub(super) struct CopyRequest<'a> {
    copy: Rc<FirstLineBox>,
    root: Frame<'a>,
    traversal: Traversal<'a>,
    letter: Option<LetterBlock>,
}

/// The first-line box of the atomic inline that tops `frames`, in the block `frames[block]`,
/// and the copy its own box is; see `first_line_box`. Kept out of `block`, whose frame nested
/// navigables' formatting stacks.
#[inline(never)]
pub(super) fn atomic_first_line<'a>(
    frames: &mut [Frame<'a>],
    block: usize,
    sheet: &'a Stylesheet,
    traversal: &mut Traversal<'a>,
    resources: &mut ImageResources<'a>,
) -> Result<(Option<Rc<FirstLineBox>>, Option<OwnCopy>), DocumentError> {
    let Some(origins) = frames[block]
        .block
        .as_ref()
        .and_then(|open| open.first_line.origins.clone())
    else {
        return Ok((None, None));
    };
    let index = frames.len() - 1;
    let style = &frames[index].style;
    let letter = if style.initial_letter.is_some() && !style.display.is_table_part() {
        let (builder, _) = super::active_builder(frames, block, sheet, traversal, resources);
        // The atomic inline is content of the ruby part it lies in, as it is when placed.
        builder.ruby.content(builder.inline)?;
        builder.initial_letter_allowed().then(|| LetterBlock {
            element: builder.element,
            style: builder.style.clone(),
        })
    } else {
        None
    };
    let scroller = super::marker_scroller(&frames[..index], traversal);
    first_line_box(
        (&frames[index], scroller),
        (&frames[block].style, &origins),
        letter,
        sheet,
        traversal,
        resources,
    )
}

/// The first-line box of the atomic inline `frame`, nested in the scroll container
/// `scroller`, on the first formatted line of `origins` (outermost first, ending with its
/// block, styled `style`), and, in a first-line copy, the copy that the box formatted for it
/// there also is; `None` for the box where no `::first-line` rule restyles it, and for both in
/// a scroll marker prepass, which keeps only the markers. `letter` is the block whose initial
/// letter it is. A box whose styles the line leaves alone is its own first-line box.
fn first_line_box<'a>(
    (frame, scroller): (&Frame<'a>, Option<MarkerScroller>),
    (style, origins): (&Style, &[ElementId]),
    letter: Option<LetterBlock>,
    sheet: &'a Stylesheet,
    traversal: &mut Traversal<'a>,
    resources: &mut ImageResources<'a>,
) -> Result<(Option<Rc<FirstLineBox>>, Option<OwnCopy>), DocumentError> {
    if traversal.marker_prepass {
        return Ok((None, None));
    }
    let Some(mut styles) =
        FirstLineStyles::of(style, origins, sheet, traversal.document, resources)?
    else {
        return Ok((None, None));
    };
    let element = frame.element.id();
    let restyled = styles.style(&frame.style)?;
    let own = (traversal.kind == TraversalKind::FirstLineCopy).then(|| {
        OwnCopy::Key(Box::new(CopyKey {
            element,
            style: (*frame.style).clone(),
            letter: letter.clone(),
        }))
    });
    if restyled == *frame.style {
        return Ok((None, own));
    }
    let key = CopyKey {
        element,
        style: restyled,
        letter,
    };
    if let Some(copy) = traversal.first_lines.borrow().find(&key) {
        return Ok((Some(copy.clone()), own));
    }
    let mut state = traversal.subtree_copy(TraversalKind::FirstLineCopy);
    state.marker_scroller = scroller;
    let request = CopyRequest {
        copy: Rc::default(),
        root: frame.copy_root(Box::new(key.style.clone())),
        traversal: state,
        letter: key.letter.clone(),
    };
    let copy = request.copy.clone();
    traversal.first_lines.borrow_mut().insert(key, copy.clone());
    if traversal.kind == TraversalKind::FirstLineCopy {
        traversal.requests.push(request);
    } else {
        format_copies(request, sheet, resources)?;
    }
    Ok((Some(copy), own))
}

/// Formats the copy `first` requests and, in turn, the copies each copy requests.
fn format_copies<'a>(
    first: CopyRequest<'a>,
    sheet: &'a Stylesheet,
    resources: &mut ImageResources<'a>,
) -> Result<(), DocumentError> {
    let mut requests = VecDeque::from([first]);
    while let Some(CopyRequest {
        copy,
        root,
        mut traversal,
        letter,
    }) = requests.pop_front()
    {
        if copy.formatted() {
            continue;
        }
        let mut formatted = block(root, sheet, &mut traversal, resources)?;
        if let Some(LetterBlock { element, style }) = letter {
            super::letter_atomic(sheet, resources.resolver, (element, &style), &mut formatted)?;
        }
        traversal
            .first_lines
            .borrow_mut()
            .share_own_copies(&mut formatted);
        copy.stage(formatted);
        requests.extend(traversal.requests);
    }
    Ok(())
}
