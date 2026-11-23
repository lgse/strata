// SPDX-License-Identifier: MIT

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
    crate::tenxer::cancel_chord(app);
    view::grid().apply_size(app, command, tx);
    Outcome::Ran
}

pub(crate) fn step(app: &mut App, steps: i32, tx: &mut Tx) {
    let _ = apply(app, SizeCommand::Step(steps), tx);
}

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

/// The folder the Icons view shows, for the pane header's popover, which closes when it changes.
pub(crate) fn shown_location(app: &App) -> Option<&Location> {
    shown_folder(app)
}

/// The shown size, for the pane header's slider (`pane_header::thumbnail_size`).
pub(crate) fn shown(app: &App) -> i32 {
    stored(app.icons.size)
}

pub(crate) fn set(app: &mut App, size: i32, tx: &mut Tx) {
    let shown = shown_folder(app).cloned();
    keep(app, shown.as_ref(), size, tx);
    app.icons.size = tile_size(size);
}

pub(crate) fn make_default(app: &mut App, size: i32, tx: &mut Tx) {
    let mut prune = None;
    shell::saved(app).change(|settings| prune = policy::set_default_icons_size(settings, size));
    if let Some(prune) = prune {
        folder_views::change(app, |views| prune.apply(views), tx);
    }
}

pub(crate) fn reset_folder(app: &mut App, tx: &mut Tx) {
    let Some(key) = remembered_shown(app) else {
        return;
    };
    folder_views::change(
        app,
        |views| views.update(&key, |view| view.icons_size = None),
        tx,
    );
}

pub(crate) fn differs_from_default(app: &App) -> bool {
    remembered_shown(app).is_some() && shown(app) != shell::preferences(app).icons_thumbnail_size
}

fn remembered_shown(app: &App) -> Option<strata_desktop::settings::folder_views::FolderKey> {
    let shown = shown_folder(app)?;
    policy::remembered_key(shell::preferences(app), shown)
}

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
    let size = Preferences::default().icons_thumbnail_size;
    keep(app, location, size, tx);
    tile_size(size)
}

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

fn folder_size(app: &App, location: Option<&Location>) -> f32 {
    let views = folder_views::views(app);
    let size = policy::icons_size_for(shell::preferences(app), location, |key| {
        views.view(key).icons_size
    });
    tile_size(size)
}

pub(super) fn tile_size(saved: i32) -> f32 {
    f32::from(u16::try_from(saved).unwrap_or(64))
}

fn stored(size: f32) -> i32 {
    size.round() as i32
}

#[cfg(test)]
mod tests;
