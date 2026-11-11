//! Columns as data: each one's id, title, sizing, alignment, whether it sorts, and what its
//! cells show.

use std::fmt;
use std::rc::Rc;
use std::sync::Arc;

use rune::{Align, Color, El};

use crate::collection::element::RowCx;

/// A column's id: what the layout, the widths, the sort and a sort intent name it by
/// (Strata's `name`, `mode`, `size`, `type`, `modified`). Cheap to clone, and `Send`, so a
/// layout can travel to another window or a worker.
///
/// Used by: C4, hosts.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ColumnId(Arc<str>);

impl ColumnId {
    /// The column named `id`.
    pub fn new(id: &str) -> Self {
        Self(Arc::from(id))
    }

    /// The id as text, as a host saves it.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ColumnId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", &*self.0)
    }
}

impl fmt::Display for ColumnId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<&str> for ColumnId {
    fn from(id: &str) -> Self {
        Self::new(id)
    }
}

/// What a cell shows: text that truncates at its end, or any element.
///
/// Used by: C4, hosts.
pub struct Cell {
    pub(super) content: CellContent,
    pub(super) color: Option<Color>,
    pub(super) mono: bool,
    pub(super) muted: bool,
}

pub(super) enum CellContent {
    Text(String),
    El(Box<El>),
}

impl Cell {
    /// Text.
    pub fn text(t: impl Into<String>) -> Self {
        Self {
            content: CellContent::Text(t.into()),
            color: None,
            mono: false,
            muted: false,
        }
    }

    /// Any element. A double click on the column's edge can't measure it, so it leaves the
    /// width as it is.
    pub fn el(e: El) -> Self {
        Self {
            content: CellContent::El(Box::new(e)),
            color: None,
            mono: false,
            muted: false,
        }
    }

    /// The monospaced face with tabular digits (Mode's permissions).
    pub fn mono(mut self) -> Self {
        self.mono = true;
        self
    }

    /// The muted text colour.
    pub fn muted(mut self) -> Self {
        self.muted = true;
        self
    }

    /// Another colour (a Kit token).
    pub fn color(mut self, c: Color) -> Self {
        self.color = Some(c);
        self
    }

    /// The text, if it is text.
    pub(super) fn as_text(&self) -> Option<&str> {
        match &self.content {
            CellContent::Text(t) => Some(t),
            CellContent::El(_) => None,
        }
    }
}

/// The widest a column without a max is drawn, in base points.
///
/// Used by: C4, hosts (to bound a saved width).
pub const WIDTH_CEILING: f32 = 4096.0;

pub(super) type CellFn<S> = Rc<dyn Fn(&S, &RowCx<'_>) -> Cell>;

/// One column: its id, its heading's title, its sizing, alignment, whether a heading click
/// sorts by it, and its cells.
///
/// Sizing follows rune-kit's `TableColumn` (`crates/rune-kit/src/table.rs` at the pinned rev):
/// a width in points or a share of the free width, bounded by a min and a max, with
/// [`Column::clamp`] as `TableColumn::clamp`. rune-kit keeps those fields private (Rune draft
/// R-K1), so the rules are copied here, not the type reused. Widths are in base units, at
/// Strata's default text size; the table scales them with the text size
/// ([`super::text_scale`]).
///
/// The primary column ([`Column::primary`], Strata's Name) holds the row's icon and label,
/// fills the free width until it is resized by hand ([`Column::fill`]), and can't be hidden.
///
/// Used by: C4, hosts.
pub struct Column<S> {
    pub(super) id: ColumnId,
    pub(super) title: String,
    pub(super) width: f32,
    pub(super) min: f32,
    pub(super) max: f32,
    pub(super) fill: bool,
    pub(super) align: Align,
    pub(super) sortable: bool,
    pub(super) cell: Option<CellFn<S>>,
}

impl<S> Clone for Column<S> {
    fn clone(&self) -> Self {
        Self {
            id: self.id.clone(),
            title: self.title.clone(),
            width: self.width,
            min: self.min,
            max: self.max,
            fill: self.fill,
            align: self.align,
            sortable: self.sortable,
            cell: self.cell.clone(),
        }
    }
}

impl<S: 'static> Column<S> {
    /// The primary column: the row's icon and label (C1e's slots, and the rename editor),
    /// filling the free width until resized. It can't be hidden.
    pub fn primary(id: &str, title: impl Into<String>) -> Self {
        Self {
            id: ColumnId::new(id),
            title: title.into(),
            width: 160.0,
            min: 48.0,
            max: f32::INFINITY,
            fill: true,
            align: Align::Start,
            sortable: false,
            cell: None,
        }
    }

    /// A column whose cells `cell` draws from the row: text or an element.
    pub fn new(
        id: &str,
        title: impl Into<String>,
        cell: impl Fn(&S, &RowCx<'_>) -> Cell + 'static,
    ) -> Self {
        Self {
            id: ColumnId::new(id),
            title: title.into(),
            width: 120.0,
            min: 48.0,
            max: f32::INFINITY,
            fill: false,
            align: Align::Start,
            sortable: false,
            cell: Some(Rc::new(cell)),
        }
    }
}

impl<S> Column<S> {
    /// Its default width, in base points.
    pub fn width(mut self, w: f32) -> Self {
        self.width = w.max(0.0);
        self
    }

    /// The narrowest it gets, by layout or by dragging.
    pub fn min(mut self, w: f32) -> Self {
        self.min = w.max(0.0);
        self
    }

    /// The widest it gets.
    pub fn max(mut self, w: f32) -> Self {
        self.max = w.max(0.0);
        self
    }

    /// It takes the width the other columns leave, at least its width, until the host's
    /// widths hold one set by hand.
    pub fn fill(mut self) -> Self {
        self.fill = true;
        self
    }

    /// Where its cells and title sit.
    pub fn align(mut self, a: Align) -> Self {
        self.align = a;
        self
    }

    /// A heading click asks the host to sort by it.
    pub fn sortable(mut self) -> Self {
        self.sortable = true;
        self
    }

    /// The column's id.
    pub fn id(&self) -> &ColumnId {
        &self.id
    }

    /// The heading's title.
    pub fn title(&self) -> &str {
        &self.title
    }

    /// Whether it is the primary column.
    pub fn is_primary(&self) -> bool {
        self.cell.is_none()
    }

    /// `w` within the column's min and max, as `TableColumn::clamp`: a max under the min yields
    /// to the min.
    pub fn clamp(&self, w: f32) -> f32 {
        w.max(self.min).min(self.max.max(self.min))
    }

    /// `w` as the table draws it, in base points: within the column's bounds, at most
    /// [`WIDTH_CEILING`] when the column has no max, and the default width for a width that
    /// isn't a finite number. A saved width can be anything a hand-edited file holds.
    pub fn bounded(&self, w: f32) -> f32 {
        let w = if w.is_finite() { w } else { self.width };
        let w = self.clamp(w);
        if w.is_finite() {
            w.min(WIDTH_CEILING.max(self.min))
        } else {
            WIDTH_CEILING.max(self.min)
        }
    }
}
