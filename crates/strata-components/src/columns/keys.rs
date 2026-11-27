// SPDX-License-Identifier: MIT

/// The key of the collection of the column whose serial is `serial`.
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

/// The key of the blank space after the last column: also the key its drop zone reaches the binder by.
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

/// The key of an active column header's top focus bar.
pub fn indicator(column: &str) -> String {
    format!("{column}-indicator")
}

/// The key of a column header's loading spinner.
pub fn spinner(column: &str) -> String {
    format!("{column}-spinner")
}

/// The key of the static vertical divider line along a column's right edge.
pub fn edge_line(column: &str) -> String {
    format!("{column}-edge-line")
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

/// The key of a partially clipped column's reveal target (`"Reveal <Folder> column"`).
pub fn reveal(column: &str) -> String {
    format!("{column}-reveal")
}

/// The key of a row's size badge, under the row's key (`row-<id>/badge`).
pub const BADGE: &str = "badge";

/// The key of the box around a row's detail line, under the row's key (`row-<id>/detail`).
pub const DETAIL: &str = "detail";
