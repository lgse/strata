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
//! - **Kept as GTK keeps it.** While `remember_folder_views` is off, the size is the saved
//!   default, for every folder. While it is on, GTK keeps the size each remembered folder
//!   chose in `folder-views.toml`; this interface does not hold that store yet, so a changed
//!   size lasts while its folder shows, as GTK keeps it for a folder it does not remember,
//!   and another folder shows the default.

use rune::Tx;
use strata_components::icons_grid::{SizeCommand, SizeRequest};
use strata_desktop::{
    keys::{
        Map, Modifiers,
        bindings::{THUMBNAILS_LARGER, THUMBNAILS_RESET, THUMBNAILS_SMALLER},
    },
    settings::Preferences,
};

use super::view;
use crate::{
    App,
    commands::{Action, Call, Keys, Outcome, Stage},
    shell,
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
pub(super) fn resized(app: &mut App, request: SizeRequest, _: &mut Tx) {
    let remembers = remembers_folder_views(app);
    let size = match request {
        SizeRequest::Step { to, .. } => to,
        SizeRequest::Reset if remembers => default_size(app),
        // GTK's `DEFAULT_ICONS_THUMBNAIL_SIZE`, the shipped default.
        SizeRequest::Reset => tile_size(Preferences::default().icons_thumbnail_size),
    };
    app.icons.size = size;
    if !remembers {
        let saved = stored(size);
        shell::saved(app).change(|settings| settings.icons_thumbnail_size = saved);
    }
}

/// Another folder shows at the size it follows: the saved default, while this interface keeps
/// no folder's own. Call it from the part's poll; C3's poll then lays the grid out at it.
pub(super) fn follow_folder(app: &mut App) {
    let icons = &app.icons;
    let shown = icons
        .single
        .pane
        .listing()
        .map(|listing| listing.location());
    if icons.sized_for.as_ref() == shown {
        return;
    }
    let shown = shown.cloned();
    let size = default_size(app);
    let icons = &mut app.icons;
    icons.sized_for = shown;
    icons.size = size;
}

fn remembers_folder_views(app: &mut App) -> bool {
    shell::saved(app).preferences().remember_folder_views
}

fn default_size(app: &mut App) -> f32 {
    tile_size(shell::saved(app).preferences().icons_thumbnail_size)
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
