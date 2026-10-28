use crate::document::{BlockBox, BlockContent};
use ferri_pdf_style::{CaptionSide, Display, Style};

#[derive(Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
enum TableOrder {
    TopCaption,
    Columns,
    Header,
    Body,
    Footer,
    BottomCaption,
}

pub(super) fn order<T: Content>(children: &mut [&T]) {
    let first_group = |role| {
        children
            .iter()
            .copied()
            .find(|child| child.display() == Some(&role))
    };
    let header = first_group(Display::TableHeaderGroup);
    let footer = first_group(Display::TableFooterGroup);
    children.sort_by_key(|child| match child.display() {
        Some(Display::TableCaption) => match child.caption_side() {
            CaptionSide::Top => TableOrder::TopCaption,
            CaptionSide::Bottom => TableOrder::BottomCaption,
        },
        Some(Display::TableColumn | Display::TableColumnGroup) => TableOrder::Columns,
        _ if header.is_some_and(|header| std::ptr::eq(*child, header)) => TableOrder::Header,
        _ if footer.is_some_and(|footer| std::ptr::eq(*child, footer)) => TableOrder::Footer,
        _ => TableOrder::Body,
    });
}

pub(super) trait Content {
    fn display(&self) -> Option<&Display>;
    fn semantic(&self) -> bool {
        false
    }
    fn whitespace(&self) -> bool;
    fn caption_side(&self) -> CaptionSide;
}

impl Content for BlockContent {
    fn semantic(&self) -> bool {
        matches!(self, Self::Events(_))
    }
    fn display(&self) -> Option<&Display> {
        match self {
            Self::Block(block) => Some(&block.style().display),
            Self::Inline(_) | Self::Positioned(_) | Self::Events(_) => None,
        }
    }
    fn whitespace(&self) -> bool {
        matches!(self, Self::Inline(paragraph) if paragraph.text().chars().all(crate::document::inline::is_css_whitespace))
    }
    fn caption_side(&self) -> CaptionSide {
        match self {
            Self::Block(block) => block.style().table.caption_side,
            _ => CaptionSide::default(),
        }
    }
}

fn proper_descendant(parent: &Display, child: Option<&Display>) -> bool {
    child.is_none_or(|child| match parent {
        table if table.is_table() => child.is_table_internal() || *child == Display::TableCaption,
        Display::TableRow => *child == Display::TableCell,
        role if role.is_row_group() => matches!(child, Display::TableRow | Display::TableCell),
        _ => false,
    })
}

fn wrap_runs<T: Content>(
    children: Vec<T>,
    role: &Display,
    anonymous: &impl Fn(Display, Vec<T>) -> T,
    wraps: impl Fn(&T) -> bool,
) -> Vec<T> {
    let mut output = Vec::new();
    let mut pending = Vec::new();
    let flush = |pending: &mut Vec<T>, output: &mut Vec<T>| {
        if !pending.is_empty() {
            output.push(anonymous(role.clone(), std::mem::take(pending)));
        }
    };
    for child in children {
        if child.semantic() {
            if pending.is_empty() {
                output.push(child);
            } else {
                pending.push(child);
            }
        } else if wraps(&child) {
            pending.push(child);
        } else {
            flush(&mut pending, &mut output);
            output.push(child);
        }
    }
    flush(&mut pending, &mut output);
    output
}

pub(super) fn normalize(
    style: &Style,
    children: Vec<BlockContent>,
    page_scope: Option<&std::sync::Arc<str>>,
) -> Vec<BlockContent> {
    normalize_with(&style.display, children, &|role, children| {
        BlockContent::Block(BlockBox::anonymous(
            style.anonymous(role),
            children,
            page_scope.cloned(),
        ))
    })
}

pub(super) fn normalize_with<T: Content>(
    parent: &Display,
    children: Vec<T>,
    anonymous: &impl Fn(Display, Vec<T>) -> T,
) -> Vec<T> {
    if *parent == Display::TableColumn {
        return Vec::new();
    }
    if *parent == Display::TableColumnGroup {
        return children
            .into_iter()
            .filter(|child| child.semantic() || child.display() == Some(&Display::TableColumn))
            .collect();
    }
    let tabular = parent.is_table() || *parent == Display::TableRow || parent.is_row_group();
    let children = children
        .iter()
        .enumerate()
        .map(|(index, child)| {
            let before = children[..index]
                .iter()
                .rev()
                .find(|child| !child.semantic())
                .and_then(Content::display);
            let after = children[index + 1..]
                .iter()
                .find(|child| !child.semantic())
                .and_then(Content::display);
            !(child.whitespace()
                && ((tabular
                    && proper_descendant(parent, before)
                    && proper_descendant(parent, after))
                    || before
                        .is_some_and(|d| d.is_table_internal() || *d == Display::TableCaption)
                        && after
                            .is_some_and(|d| d.is_table_internal() || *d == Display::TableCaption)))
        })
        .collect::<Vec<_>>()
        .into_iter()
        .zip(children)
        .filter_map(|(keep, child)| keep.then_some(child))
        .collect();
    match parent {
        table if table.is_table() => wrap_runs(children, &Display::TableRow, anonymous, |child| {
            !child.display().is_some_and(Display::is_table_child)
        }),
        Display::TableRow => wrap_runs(children, &Display::TableCell, anonymous, |child| {
            child.display() != Some(&Display::TableCell)
        }),
        role if role.is_row_group() => {
            wrap_runs(children, &Display::TableRow, anonymous, |child| {
                child.display() != Some(&Display::TableRow)
            })
        }
        _ => {
            let children = wrap_runs(children, &Display::TableRow, anonymous, |child| {
                child.display() == Some(&Display::TableCell)
            });
            wrap_runs(children, &Display::Table, anonymous, |child| {
                child.display().is_some_and(Display::is_table_child)
            })
        }
    }
}
