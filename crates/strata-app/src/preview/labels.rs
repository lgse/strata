// SPDX-License-Identifier: MIT

//! Translated labels and dynamic shortcut wording for C9's [`DrawerLabels`], C10b's
//! [`CodeLabels`], C11's [`DocumentLabels`], and C11b's [`SheetLabels`].

use strata_components::{
    code_view::CodeLabels, document_view::DocumentLabels, preview_drawer::DrawerLabels,
    sheet_grid::SheetLabels,
};
use strata_core::i18n::{integer, tr};
use strata_desktop::keys::{
    Keys,
    bindings::{FIND_IN_PREVIEW, PREVIEW},
    label,
};

use crate::location::labels::with_keys;

pub(super) fn drawer() -> DrawerLabels {
    let mut labels = DrawerLabels::default();
    labels.region = tr("Preview");
    labels.close = with_keys(
        "Close preview (%{keys})",
        Some("Close preview"),
        Keys::One(PREVIEW.shown()),
    );
    labels.open = tr("Open in default application");
    labels.wrap = tr("Toggle word wrap");
    labels.view_source = tr("View source");
    labels.view_rendered = tr("View rendered");
    labels.find = find_button_label();
    labels.find_field = tr("Find in preview");
    labels.find_previous = tr("Previous match");
    labels.find_next = tr("Next match");
    labels.find_close = tr("Close find");
    labels.no_matches = tr("No matches");
    labels.match_of = match_of;
    labels.divider = tr("Preview panel minimum width");
    labels.detail_descriptions = [tr("Size"), tr("Modified"), tr("Type")];
    labels
}

fn find_button_label() -> String {
    let shortcut = label(&Keys::One(FIND_IN_PREVIEW.shown()));
    if shortcut.is_empty() {
        return tr("Find in preview");
    }
    rust_i18n::t!("Find in preview (%{shortcut})", shortcut = shortcut).into_owned()
}

fn match_of(current: usize, total: usize) -> String {
    rust_i18n::t!(
        "Match %{current} of %{total}",
        current = integer(current as u64),
        total = integer(total as u64),
    )
    .into_owned()
}

pub(super) fn code() -> CodeLabels {
    let mut labels = CodeLabels::default();
    labels.document = tr("Source");
    labels.position = tr("Document position");
    labels.line_of = tr("Line %{line} of %{lines}")
        .replace("%{line}", "{line}")
        .replace("%{lines}", "{lines}");
    labels.blank = tr("Blank");
    labels
}

pub(super) fn document() -> DocumentLabels {
    let mut labels = DocumentLabels::default();
    labels.document = tr("Document");
    labels.copy_code = tr("Copy code");
    labels.code_copied = tr("Code copied");
    labels.copy_diagram = tr("Copy diagram source");
    labels.copy_equation = tr("Copy equation source");
    labels.loading_image = tr("Loading preview…");
    labels.diagram = tr("Mermaid diagram");
    labels.equation = tr("LaTeX equation");
    labels
}

pub(super) fn sheet() -> SheetLabels {
    SheetLabels {
        sheets: tr("Sheets"),
        table: tr("Table"),
        row: tr("Row"),
        column: tr("Column %{value1}").replace("%{value1}", "{}"),
        resize: tr("Resize %{column}").replace("%{column}", "{}"),
        sort_key: tr("Sort key %{index}").replace("%{index}", "{}"),
        ascending: tr("ascending"),
        descending: tr("descending"),
        limited: tr(
            "Table preview reached its text, column, or value budget; open the file to see all data.",
        ),
        shortened: tr(
            "Cell shortened for responsive preview; copying the table keeps the complete text.",
        ),
        loading: tr("Loading sheet…"),
        failed: tr("Preview unavailable"),
        retry: tr("Try again"),
        empty: tr("This sheet is empty"),
    }
}
