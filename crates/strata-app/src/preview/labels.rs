// SPDX-License-Identifier: MIT

//! Translated labels and dynamic shortcut wording for C9's [`DrawerLabels`] and C10b's
//! [`CodeLabels`], read at each run of the view so keymap changes show at once.

use strata_components::{code_view::CodeLabels, preview_drawer::DrawerLabels};
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
