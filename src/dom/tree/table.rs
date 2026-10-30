//! Table columns from document-language semantics alone: the HTML table model (HTML
//! §4.9.12.1) and `MathML` Core tables, which form slots with the same algorithm (§3.5.3).
use super::{Element, ElementId, TreeRelation};
use crate::dom::{HTML_NAMESPACE, MATHML_NAMESPACE, non_negative_integer};
use ego_tree::NodeId;
use std::{collections::HashMap, ops::Range, sync::Arc};

const MAX_COLUMN_SPAN: usize = 1000;
const MAX_ROW_SPAN: usize = 65534;
const DEFAULT_SPAN: usize = 1;
const HTML_COLUMN_SPAN: &str = "colspan";
const MATHML_COLUMN_SPAN: &str = "columnspan";
const ROW_SPAN: &str = "rowspan";
const SPAN: &str = "span";

#[derive(Clone, Copy, PartialEq)]
enum Part {
    ColumnGroup,
    Column,
    RowGroup,
    Footer,
    Row,
    Cell,
}

/// The role an element plays in the table model of its namespace.
fn part(element: Element<'_>) -> Option<Part> {
    match (element.namespace(), element.name()) {
        (HTML_NAMESPACE, "colgroup") => Some(Part::ColumnGroup),
        (HTML_NAMESPACE, "col") => Some(Part::Column),
        (HTML_NAMESPACE, "thead" | "tbody") => Some(Part::RowGroup),
        (HTML_NAMESPACE, "tfoot") => Some(Part::Footer),
        (HTML_NAMESPACE, "tr") | (MATHML_NAMESPACE, "mtr") => Some(Part::Row),
        (HTML_NAMESPACE, "td" | "th") | (MATHML_NAMESPACE, "mtd") => Some(Part::Cell),
        _ => None,
    }
}

fn is_table(element: Element<'_>) -> bool {
    matches!(
        (element.namespace(), element.name()),
        (HTML_NAMESPACE, "table") | (MATHML_NAMESPACE, "mtable")
    )
}

/// A cell's or `col`/`colgroup` element's span: an error, zero or absence counts as one.
pub(crate) fn column_span(element: Element<'_>) -> usize {
    let attribute = match (element.namespace(), part(element)) {
        (MATHML_NAMESPACE, _) => MATHML_COLUMN_SPAN,
        (_, Some(Part::Cell)) => HTML_COLUMN_SPAN,
        _ => SPAN,
    };
    element
        .attr(attribute)
        .and_then(non_negative_integer)
        .map_or(DEFAULT_SPAN, |span| span as usize)
        .clamp(DEFAULT_SPAN, MAX_COLUMN_SPAN)
}

/// A cell's `rowspan`, where zero extends the cell to the end of its row group.
pub(crate) fn row_span(element: Element<'_>) -> usize {
    element
        .attr(ROW_SPAN)
        .and_then(non_negative_integer)
        .map_or(DEFAULT_SPAN, |span| span as usize)
        .min(MAX_ROW_SPAN)
}

/// The columns a cell or column element covers, and the number in its table.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Columns {
    pub range: Range<usize>,
    pub count: usize,
}

#[derive(Debug, Default)]
pub(crate) struct TableModel {
    cells: HashMap<NodeId, Range<usize>>,
    /// The `col` and `colgroup` elements, each with the columns it corresponds to.
    columns: Vec<(NodeId, Range<usize>)>,
    width: usize,
}

/// A cell covering rows before `end` of the current row group; a downward-growing cell
/// never ends inside it.
struct Spanning {
    columns: Range<usize>,
    end: usize,
}

#[derive(Default)]
struct Former {
    model: TableModel,
    height: usize,
    current: usize,
    spanning: Vec<Spanning>,
}
impl Former {
    fn form(table: Element<'_>) -> TableModel {
        let mut former = Self::default();
        let mut rows_started = false;
        let mut footers = Vec::new();
        for child in table.children(TreeRelation::Selector) {
            match part(child) {
                Some(Part::ColumnGroup) if !rows_started => former.column_group(child),
                Some(Part::Row) => {
                    rows_started = true;
                    former.row(child);
                }
                Some(Part::Footer) => {
                    rows_started = true;
                    former.end_row_group();
                    footers.push(child);
                }
                Some(Part::RowGroup) => {
                    rows_started = true;
                    former.end_row_group();
                    former.row_group(child);
                }
                _ => {}
            }
        }
        for footer in footers {
            former.row_group(footer);
        }
        former.model
    }

    fn column_group(&mut self, group: Element<'_>) {
        let start = self.model.width;
        let mut columns = group
            .children(TreeRelation::Selector)
            .filter(|child| part(*child) == Some(Part::Column))
            .peekable();
        if columns.peek().is_none() {
            self.model.width += column_span(group);
        }
        for column in columns {
            let first = self.model.width;
            self.model.width += column_span(column);
            self.model
                .columns
                .push((column.raw.id(), first..self.model.width));
        }
        self.model
            .columns
            .push((group.raw.id(), start..self.model.width));
    }

    fn row_group(&mut self, group: Element<'_>) {
        for row in group
            .children(TreeRelation::Selector)
            .filter(|child| part(*child) == Some(Part::Row))
        {
            self.row(row);
        }
        self.end_row_group();
    }

    fn end_row_group(&mut self) {
        self.current = self.height;
        self.spanning.clear();
    }

    fn row(&mut self, row: Element<'_>) {
        if self.height == self.current {
            self.height += 1;
        }
        let current = self.current;
        self.spanning.retain(|cell| cell.end > current);
        let mut occupied: Vec<Range<usize>> = self
            .spanning
            .iter()
            .map(|cell| cell.columns.clone())
            .collect();
        occupied.sort_unstable_by_key(|columns| columns.start);
        let mut occupied = occupied.into_iter().peekable();
        let mut x = 0;
        for cell in row
            .children(TreeRelation::Selector)
            .filter(|child| part(*child) == Some(Part::Cell))
        {
            while let Some(columns) = occupied.peek() {
                if columns.end <= x {
                    occupied.next();
                } else if columns.start <= x {
                    x = columns.end;
                    occupied.next();
                } else {
                    break;
                }
            }
            let columns = x..x + column_span(cell);
            let rows = row_span(cell);
            self.model.width = self.model.width.max(columns.end);
            self.height = self.height.max(current + rows.max(DEFAULT_SPAN));
            self.spanning.push(Spanning {
                columns: columns.clone(),
                end: if rows == 0 {
                    usize::MAX
                } else {
                    current + rows
                },
            });
            x = columns.end;
            self.model.cells.insert(cell.raw.id(), columns);
        }
        self.current += 1;
    }
}

impl Element<'_> {
    /// The table whose model gives this cell its slots: that of its row's parent or of the
    /// row group holding its row.
    fn cell_table(self) -> Option<Self> {
        if part(self)? != Part::Cell {
            return None;
        }
        let row = self.parent(TreeRelation::Selector)?;
        if part(row)? != Part::Row {
            return None;
        }
        let parent = row.parent(TreeRelation::Selector)?;
        if is_table(parent) {
            return Some(parent);
        }
        if !matches!(part(parent)?, Part::RowGroup | Part::Footer) {
            return None;
        }
        parent
            .parent(TreeRelation::Selector)
            .filter(|table| is_table(*table))
    }

    fn table_model(self) -> Arc<TableModel> {
        let key = self.raw.id();
        if let Some(model) = self.tree.tables.borrow().get(&key) {
            return model.clone();
        }
        let model = Arc::new(Former::form(self));
        self.tree.tables.borrow_mut().insert(key, model.clone());
        model
    }

    /// The columns of the table cell this element is (Selectors 5 §8.2).
    pub(crate) fn cell_columns(self) -> Option<Columns> {
        let model = self.cell_table()?.table_model();
        let range = model.cells.get(&self.raw.id())?.clone();
        Some(Columns {
            range,
            count: model.width,
        })
    }

    /// The `col` and `colgroup` elements representing a column this cell belongs to
    /// (Selectors 5 §8.1).
    pub(crate) fn column_elements(self) -> Vec<Self> {
        let Some(table) = self.cell_table() else {
            return Vec::new();
        };
        let model = table.table_model();
        let Some(cell) = model.cells.get(&self.raw.id()) else {
            return Vec::new();
        };
        model
            .columns
            .iter()
            .filter(|(_, columns)| columns.start < cell.end && cell.start < columns.end)
            .filter_map(|(node, _)| {
                self.tree.element(ElementId {
                    document: self.tree.identity,
                    node: *node,
                })
            })
            .collect()
    }
}
