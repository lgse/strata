// SPDX-License-Identifier: MIT

//! The tile size, as GTK's Icons view changes and keeps it (`ThumbnailResize` in
//! `src/ui/browser_modes/thumbnail_size.rs`, `set_folder_icons_size` and
//! `reset_folder_icons_size` in `src/ui/preferences/folder_view_settings.rs`).
//!
//! - **The window's keys** step it: Ctrl+Alt+plus or = larger, Ctrl+Alt+minus smaller, at the
//!   dispatcher's `zoom_shortcut` stage, the default map's chords as GTK's `for_shortcut`
//!   reads them, and Ctrl+Alt+0 resets it. Ctrl+Shift+wheel over the grid steps it too: C3
//!   claims the wheel (`WheelSteps`, the steps of GTK's `WheelZoom`) and hands the step to
//!   [`resized`]. GTK's wheel reaches the size from anywhere in the window but the PDF
//!   preview (`zoom_controller` in `src/ui/window/keyboard/wheel_zoom.rs`); a window-level
//!   wheel is not this module's (`docs/rune/parity/packages/A1.md`, "Open").
//! - **A step** moves along C3's ladder (`Look::tiles`, GTK's `STEPS`), which keeps it between
//!   32 and 256. **A reset** goes back to the size the folder follows: the saved default, or
//!   GTK's 64 while folders' views are not remembered.
//! - **Kept as GTK keeps it,** by the policy both interfaces share
//!   (`strata_desktop::settings::folder_view_policy`): while `remember_folder_views` is off
//!   the size is the saved default, for every folder; while it is on, a remembered folder keeps
//!   its own in `folder-views.toml` ([`crate::folder_views`]) unless it equals the default, and
//!   a folder with no key keeps a changed size while it shows.

use rune::Tx;
use strata_components::icons_grid::{SizeCommand, SizeRequest};
use strata_core::model::Location;
use strata_desktop::{
    keys::{
        Map, Modifiers,
        bindings::{THUMBNAILS_LARGER, THUMBNAILS_RESET, THUMBNAILS_SMALLER},
    },
    settings::{
        Preferences,
        folder_view_policy::{self as policy, IconsSizeKept},
    },
};

use super::view;
use crate::{
    App,
    commands::{Action, Call, Keys, Outcome, Stage},
    folder_views, shell,
};

/// The size commands, read at the dispatcher's `zoom_shortcut` stage (`keyboard.rs:466`),
/// ahead of every owner of input, as GTK reads them.
pub(super) const COMMANDS: &[Action<App>] = &[
    Action::new("thumbnails-larger", larger)
        .command(THUMBNAILS_LARGER)
        .keys(&[Keys::at(
            Stage::ZoomShortcut,
            &[THUMBNAILS_LARGER.in_map(Map::Default)],
        )]),
    Action::new("thumbnails-smaller", smaller)
        .command(THUMBNAILS_SMALLER)
        .keys(&[Keys::at(
            Stage::ZoomShortcut,
            &[THUMBNAILS_SMALLER.in_map(Map::Default)],
        )]),
    Action::new("thumbnails-reset", reset)
        .command(THUMBNAILS_RESET)
        .keys(&[Keys::at(
            Stage::ZoomShortcut,
            &[THUMBNAILS_RESET.ignoring(Modifiers::empty())],
        )]),
];

fn larger(app: &mut App, _: &Call<'_>, tx: &mut Tx) -> Outcome {
    apply(app, SizeCommand::Step(1), tx)
}

fn smaller(app: &mut App, _: &Call<'_>, tx: &mut Tx) -> Outcome {
    apply(app, SizeCommand::Step(-1), tx)
}

fn reset(app: &mut App, _: &Call<'_>, tx: &mut Tx) -> Outcome {
    apply(app, SizeCommand::Reset, tx)
}

/// C3 works the step out and lays the grid out again; [`resized`] keeps the size.
fn apply(app: &mut App, command: SizeCommand, tx: &mut Tx) -> Outcome {
    view::grid().apply_size(app, command, tx);
    Outcome::Ran
}

/// C3's request: a step's size, or the one a reset goes back to, kept as GTK keeps it.
pub(super) fn resized(app: &mut App, request: SizeRequest, tx: &mut Tx) {
    let shown = shown_folder(app).cloned();
    let size = match request {
        SizeRequest::Step { to, .. } => {
            keep(app, shown.as_ref(), stored(to), tx);
            to
        }
        SizeRequest::Reset => reset_to(app, shown.as_ref(), tx),
    };
    app.icons.size = size;
}

/// Keeps a changed size where the shared policy says: the default, or the folder's own
/// (`set_folder_icons_size`, which GTK's slider calls on every move).
fn keep(app: &mut App, location: Option<&Location>, size: i32, tx: &mut Tx) {
    match policy::keep_icons_size(shell::preferences(app), location, size) {
        IconsSizeKept::Default(size) => {
            shell::saved(app).change(|settings| settings.icons_thumbnail_size = size);
        }
        IconsSizeKept::Folder(key, size) => folder_views::change(
            app,
            |views| views.update(&key, |view| view.icons_size = size),
            tx,
        ),
        IconsSizeKept::Shown => {}
    }
}

/// The size a reset goes back to, with what it keeps (`ThumbnailResize::Reset`): a remembered
/// folder forgets its own size and follows the default; while folders' views are remembered
/// any other folder takes the default as it is; while they are not, the default goes back to
/// GTK's shipped 64.
fn reset_to(app: &mut App, location: Option<&Location>, tx: &mut Tx) -> f32 {
    let settings = shell::preferences(app);
    let remembered = location.and_then(|location| policy::remembered_key(settings, location));
    if let Some(key) = remembered {
        folder_views::change(
            app,
            |views| views.update(&key, |view| view.icons_size = None),
            tx,
        );
        return folder_size(app, location);
    }
    if settings.remember_folder_views {
        return tile_size(settings.icons_thumbnail_size);
    }
    // GTK's `DEFAULT_ICONS_THUMBNAIL_SIZE`, the shipped default.
    let size = Preferences::default().icons_thumbnail_size;
    keep(app, location, size, tx);
    tile_size(size)
}

/// Another folder shows at the size it follows, and a shown folder follows a change of the
/// values (another process's, say) only when the size it resolves to changes, as GTK's
/// `apply_resolved_icons_size` does: a size kept only while its folder shows stays. Call it
/// from the part's poll; C3's poll then lays the grid out at it.
pub(super) fn follow_folder(app: &mut App) {
    let revision = folder_views::revision(app);
    let icons = &app.icons;
    let shown = shown_folder(app);
    let moved = icons.sized_for.as_ref() != shown;
    if !moved && icons.sized_at == revision {
        return;
    }
    let shown = shown.cloned();
    let size = folder_size(app, shown.as_ref());
    let icons = &mut app.icons;
    let resolved_anew = moved || icons.resolved != size;
    icons.sized_for = shown;
    icons.sized_at = revision;
    icons.resolved = size;
    if resolved_anew {
        icons.size = size;
    }
}

fn shown_folder(app: &App) -> Option<&Location> {
    app.icons
        .single
        .pane
        .listing()
        .map(|listing| listing.location())
}

/// The size `location` shows at: its own while it is remembered, else the default
/// (`icons_size_for`).
fn folder_size(app: &App, location: Option<&Location>) -> f32 {
    let views = folder_views::views(app);
    let size = policy::icons_size_for(shell::preferences(app), location, |key| {
        views.view(key).icons_size
    });
    tile_size(size)
}

/// A saved size as the grid takes it: loading holds it between 32 and 256 (`stored.rs` in
/// `strata_desktop::settings`).
pub(super) fn tile_size(saved: i32) -> f32 {
    f32::from(u16::try_from(saved).unwrap_or(64))
}

/// A size of C3's ladder as `settings.toml` holds it.
fn stored(size: f32) -> i32 {
    size.round() as i32
}

#[cfg(test)]
mod tests;
