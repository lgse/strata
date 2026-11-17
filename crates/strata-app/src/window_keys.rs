// SPDX-License-Identifier: MIT

//! The keys the GTK window's dispatcher reads after a view's or a field's own have left them
//! (`src/ui/window/keyboard/items.rs` and `commands.rs`): Escape at three stages, and Enter
//! at the items. Each reads the press through the keymap's aliases (`commands::read`), so a
//! structural alias such as `o` for Enter or Ctrl+[ for Escape acts as the key it stands for,
//! where the view's and the editor's own keys read the key alone.
//!
//! - **`inline_editing`**, for an Escape while a rename is open: the rename ends
//!   (`cancel_rename`). The editor takes a plain Escape itself; an alias reaches the stage.
//! - **`dismissal`**, for an Escape anywhere outside text: an open preview closes and returns
//!   the keyboard to the shown view, else the peek closes, else the active column's selection
//!   clears (`dismiss_preview_or_selection`). With no preview open, an Escape on a header
//!   button clears the selection and leaves the keyboard on the button.
//! - **`item_navigation`**, for an Escape that reaches the listing with nothing to clear: the
//!   browser's `escape()` (`directory_enter_or_escape`), which closes the deepest column. In
//!   Columns that is the column the keyboard was in; in Icons and List it is one the view does
//!   not show, which the next folder opened in place no longer finds behind it.
//! - **`item_navigation`**, for an Enter that reaches the listing: the cursor's entry opens
//!   (`activate_focused`). The view takes a plain Return itself; an alias reaches the stage.
//!
//! The view's own Escape clears a selection while it has the keyboard, and leaves an Escape
//! with nothing selected to the stages after it; the `dismissal` stage is asked ahead of it,
//! as on GTK, and clears the same selection through the browser.
//!
//! The commands are registered by the menus' part (`menus.rs`), this package's: nothing of
//! them is drawn or kept.

use rune::Tx;
use strata_desktop::keys::bindings::{
    ENTER, ENTER_ANY_SHIFT, ESCAPE, ESCAPE_ANY, ESCAPE_ANY_SHIFT,
};

use crate::{
    App,
    commands::{Action, Call, Keys, Outcome, Situation, Stage},
    rename, shell,
};

/// `cancel_rename`: an open rename ends without renaming.
fn rename_cancelled(app: &mut App, _: &Call<'_>, tx: &mut Tx) -> Outcome {
    rename::cancel(app, tx);
    Outcome::Ran
}

/// `dismiss_preview_or_selection`: the preview drawer, the peek, else the active column's
/// selection.
fn dismissed(app: &mut App, _: &Call<'_>, tx: &mut Tx) -> Outcome {
    if crate::preview::dismiss_escape(app, tx) {
        return Outcome::Ran;
    }
    let browser = &app.browser;
    if browser.close_peek() || browser.clear_active_selection() {
        Outcome::Ran
    } else {
        Outcome::Declined
    }
}

/// `directory_enter_or_escape`'s Escape: the browser closes its deepest column.
fn escaped(app: &mut App, _: &Call<'_>, _: &mut Tx) -> Outcome {
    app.browser.clone().escape();
    Outcome::Ran
}

/// `directory_enter_or_escape`'s Enter: the cursor's entry opens, in a child column in
/// Columns and in place in a view that shows one folder (`BrowserView::activate_focused`).
fn activated(app: &mut App, _: &Call<'_>, tx: &mut Tx) -> Outcome {
    if app.browser.is_chooser_mode()
        && let Some((depth, position, _)) = app.browser.focused_item()
        && crate::chooser::confirm_key_activation(
            app,
            (depth, position),
            strata_components::collection::element::Activation::Key,
            tx,
        )
    {
        return Outcome::Ran;
    }
    let browser = app.browser.clone();
    if shows_columns(app) {
        browser.activate_focused();
    } else {
        browser.activate_focused_in_place();
    }
    Outcome::Ran
}

/// The Columns view is shown: the one the saved `browser_mode` names `columns`.
fn shows_columns(app: &App) -> bool {
    shell::shown_view(app).is_some_and(|view| (view.shows)("columns"))
}

/// `items_unfocused_for`: an Escape reaches the items only from the shown view's listing.
fn from_the_listing(_: &App, at: &Situation) -> bool {
    at.in_view()
}

pub(crate) const COMMANDS: &[Action<App>] = &[
    Action::new("rename-cancel", rename_cancelled)
        .command(ESCAPE)
        .keys(&[
            Keys::at(Stage::InlineEditing, &[ESCAPE_ANY]).when(|app, _| app.rename.state.is_open())
        ]),
    // `!event.text_has_focus()`.
    Action::new("dismiss", dismissed)
        .command(ESCAPE)
        .keys(&[Keys::at(Stage::Dismissal, &[ESCAPE_ANY_SHIFT])
            .when(|app, at| !app.browser.is_chooser_mode() && !at.text())]),
    Action::new("escape", escaped)
        .command(ESCAPE)
        .keys(&[
            Keys::at(Stage::ItemNavigation, &[ESCAPE_ANY_SHIFT]).when(|app, at| {
                !app.browser.is_chooser_mode()
                    && from_the_listing(app, at)
                    && !shell::preferences(app).tenxer_mode
            }),
        ]),
    Action::new("activate", activated)
        .command(ENTER)
        .keys(&[Keys::at(Stage::ItemNavigation, &[ENTER_ANY_SHIFT]).when(from_the_listing)]),
];

#[cfg(test)]
mod tests;
