//! The keys the view gives its parts, so hosts, scripts and scroll requests can name them.
//! Every key starts with the view's key, so two strips (two windows' states in one process,
//! the chooser) never collide.

/// The key of the collection of the column whose serial is `serial`: its rows' key paths start
/// there (`viewport_path`, `row_path`). A column gets a new serial for each folder it shows.
pub fn column(view: &str, serial: u64) -> String {
    format!("{view}-col-{serial}")
}

/// The key of a column's shell: kept by a sibling that takes the column's place.
pub(super) fn slot(view: &str, slot: u64) -> String {
    format!("{view}-slot-{slot}")
}

/// The key of the strip's sideways scroll viewport, for scroll requests.
pub fn strip(view: &str) -> String {
    format!("{view}-strip")
}

/// The key of the blank space after the last column: also the key its drop zone reaches the
/// binder by (W1's `Dnd::drop_zone_keyed`). Each column's rows and background reach it as C1e's
/// do, under the column's own key.
pub fn blank(view: &str) -> String {
    format!("{view}-blank")
}

/// The key of a column's header, under its collection: `{column}-header`.
pub fn header(column: &str) -> String {
    format!("{column}-header")
}

/// The key of a column header's title.
pub fn title(column: &str) -> String {
    format!("{column}-title")
}

/// The key of a column header's "Close this pane" button.
pub fn close(column: &str) -> String {
    format!("{column}-close")
}

/// The key of a column header's loading spinner.
pub fn spinner(column: &str) -> String {
    format!("{column}-spinner")
}

/// The key of the resize edge on a column's right.
pub fn edge(column: &str) -> String {
    format!("{column}-edge")
}

/// The key of the accent line a column's resize edge lights under the pointer.
pub fn edge_accent(column: &str) -> String {
    format!("{column}-edge-accent")
}

/// The key of the "Column width" caption over a column's edge.
pub fn caption(column: &str) -> String {
    format!("{column}-width-caption")
}

/// The key of the wash over the column a held file drag would drop into.
pub fn wash(column: &str) -> String {
    format!("{column}-drop")
}

/// The key of a row's size badge, under the row's key (`row-<id>/badge`).
pub const BADGE: &str = "badge";
