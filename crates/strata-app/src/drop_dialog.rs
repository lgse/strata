// SPDX-License-Identifier: MIT

//! Cross-volume file drop confirmation on C15's [`StrataDialog`], matching GTK's
//! `confirm_cross_volume_drop` in `src/ui/browser/transfer/drop.rs`.
//!
//! - **Footer order:** `[Cancel, Move, Copy]`, with `Copy` as the accent primary button and no
//!   default button so Return activates only the focused control.
//! - **Initial focus:** `Copy` (`Focus::Primary`), so `Shift+Tab` steps to `Move` and `Cancel`.
//! - **Dismissal (`Cancel`, `Escape`, `Close dialog`, backdrop):** abandons the drop without
//!   starting a transfer and returns keyboard focus to the listing.

use rune::{El, Tx};
use strata_components::dialog::{Focus, MESSAGE_WIDTH, StrataDialog, Tone};
use strata_core::{
    i18n::{count, tr},
    model::Location,
};
use strata_fs::services::VolumeRelation;
use strata_kit::icons::IconName;

use crate::{
    App, conflict_dialog,
    commands::Holds,
    shell,
    slots::{Frame, Later, Part, Place, Scope, Setup},
    transfer,
};

#[cfg(test)]
mod tests;

pub(crate) fn cross_volume_drop_description(volume: VolumeRelation) -> &'static str {
    match volume {
        VolumeRelation::Different => "The destination is on a different device.",
        VolumeRelation::Same | VolumeRelation::Unknown => {
            "Strata could not determine whether the destination is on the same device."
        }
    }
}

struct Shown {
    destination: Location,
    sources: Vec<Location>,
    volume: VolumeRelation,
    generation: u64,
    pending: bool,
}

pub(crate) struct DropDialog {
    later: Later<App>,
    shown: Option<Shown>,
    open: bool,
    opened: u64,
}

impl Part for DropDialog {
    const PLACE: Place<App> = Place::Dialogs(build);
    const SCOPE: Scope = Scope::Tab;

    fn new(setup: &Setup<'_>) -> Self {
        Self {
            later: setup.later.clone(),
            shown: None,
            open: false,
            opened: 0,
        }
    }

    fn keyboard(app: &App) -> Option<Holds> {
        app.drop_dialog
            .open
            .then_some(Holds::Modal { field: false })
    }
}

/// Opens the `"Copy or move?"` prompt for `sources` dropped onto `destination`.
pub(crate) fn prompt(
    app: &mut App,
    destination: Location,
    sources: Vec<Location>,
    volume: VolumeRelation,
) {
    if sources.is_empty() {
        return;
    }
    let dialog = &mut app.drop_dialog;
    dialog.opened += 1;
    let generation = dialog.opened;
    dialog.shown = Some(Shown {
        destination,
        sources,
        volume,
        generation,
        pending: true,
    });
    dialog.open = true;
}

#[cfg(test)]
pub(crate) fn is_open(app: &App) -> bool {
    app.drop_dialog.open
}

fn restore_listing_focus(app: &mut App, tx: &mut Tx) {
    if let Some(view) = shell::view_focus(app)
        && tx.focused() != Some(view)
    {
        tx.focus(view, false);
    }
    app.browser.focus_active();
}

fn dismiss(app: &mut App) {
    if !app.drop_dialog.open {
        return;
    }
    app.drop_dialog.open = false;
    if let Some(shown) = app.drop_dialog.shown.as_mut() {
        shown.pending = false;
    }
    let later = app.drop_dialog.later.clone();
    later.then(|app, tx| {
        restore_listing_focus(app, tx);
        true
    });
}

fn choose(app: &mut App, move_sources: bool) {
    if !app.drop_dialog.open {
        return;
    }
    app.drop_dialog.open = false;
    let Some(shown) = app.drop_dialog.shown.as_mut().filter(|s| s.pending) else {
        return;
    };
    shown.pending = false;
    let destination = shown.destination.clone();
    let sources = shown.sources.clone();
    let later = app.drop_dialog.later.clone();
    later.then(move |app, tx| {
        transfer::start_drop_transfer(app, destination, sources, move_sources);
        if !shell::modal_open(app) {
            restore_listing_focus(app, tx);
        }
        true
    });
}

fn build(app: &App, frame: &mut Frame<'_, App>) -> Option<El> {
    let state = &app.drop_dialog;
    let shown = state.shown.as_ref()?;
    let key = format!("drop-confirm-{}", shown.generation);
    let subtitle = rust_i18n::t!(
        "%{items} to %{destination}",
        items = count("items", shown.sources.len()),
        destination = conflict_dialog::compact_display_path(&shown.destination)
    )
    .into_owned();
    let mut dialog = StrataDialog::new(key, tr("Copy or move?"), dismiss)
        .open(state.open)
        .subtitle(subtitle)
        .icon(IconName::Copy)
        .tone(Tone::Accent)
        .message(tr(cross_volume_drop_description(shown.volume)))
        .action("cancel", tr("Cancel"), dismiss)
        .action("move", tr("Move"), |app| choose(app, true))
        .primary(tr("Copy"), |app| choose(app, false))
        .initial_focus(Focus::Primary)
        .default_button(false)
        .close_label(tr("Close dialog"))
        .width(MESSAGE_WIDTH);
    if let Some(view) = shell::view_focus(app) {
        dialog = dialog.return_focus(view);
    }
    dialog.build(&app.look, frame.layer);
    None
}
