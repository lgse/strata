// SPDX-License-Identifier: MIT

//! The Icons and List panes' header (A5): GTK's `pane_header` in
//! `src/ui/browser_modes/pane_shell.rs`, over the listing of both single-pane views, which
//! [`crate::single::group`] puts there.
//!
//! - **Back, Forward and Parent folder** (`list_navigation` in
//!   `src/ui/browser_modes/pane_header.rs`), each insensitive where the browser cannot go,
//!   named for its first key as bound now ("Back (Alt+Left)"), and running the window's own
//!   commands (`history-back`, `history-forward`, `parent-folder`).
//! - **Back lists the back history** ([`history`]): a right-click, a press held for
//!   [`history::HOLD`] (whose release then does not go back), or the Menu key or Shift+F10 on
//!   the focused button open it under Back, and a chosen entry jumps back to it in one step
//!   (`Browser::back_to`). While it is open the window's commands wait, as under GTK's
//!   popover ([`Holds::Menu`]).
//! - **The heading:** the folder's name, described by it, as GTK's heading label is.
//! - **The header's own actions** ([`actions`]): the Trash's "Empty Trash", "Refresh", and the
//!   Icons view's "Thumbnail size" button and its popover ([`thumbnail_size`]), in GTK's order;
//!   [`actions`] serves a Columns column's header too, and names who brings the rest.
//!
//! The rest of GTK's header is other packages': the spinner and the truncated listing's hint,
//! and the other actions after the heading (leftovers, the sort and the filter). They go in [`bar`]'s `actions` row, in GTK's order (`icons_controls` in
//! `src/ui/browser_modes/icons_controls.rs`).

use rune::{Drag, El, KeyEvent, Phase, PointerEvent, Tx, row, text};
use strata_components::{
    context_menu::{ContextMenu, ContextMenuState, MenuIntent, MenuTarget, menu_chord},
    contract::Point,
    header,
};
use strata_core::i18n::tr;
use strata_desktop::keys::{
    Keys,
    bindings::{CONTEXT_MENU, HISTORY_BACK, HISTORY_FORWARD, PARENT_FOLDER},
};
use strata_kit::icons::IconName;

use crate::{
    App, buttons,
    commands::{self, Holds, On, Situation},
    location::labels::with_keys,
    shell,
    slots::{Frame, Part, Place, Setup},
};

pub(crate) mod actions;
mod empty_trash;
mod history;
mod refresh;
mod thumbnail_size;

#[cfg(test)]
mod tests;

/// The header's state: the back history's menu, and a press held on Back.
pub(crate) struct PaneHeader {
    labels: Labels,
    menu: ContextMenuState,
    heading: String,
    /// When a press on Back began, on the poll's clock, while it is held.
    held: Option<f64>,
    /// The held press opened the history: its release does not go back.
    swallow: bool,
    /// Back's box in window points, where the history opens under it.
    back: [f32; 4],
    thumbnails: thumbnail_size::ThumbnailSize,
}

/// The header's own words.
struct Labels {
    empty_trash: String,
}

/// One of the three buttons: its key's end, its icon, its name, the key that names it, the
/// command it runs, and whether the browser can go there.
struct Button {
    suffix: &'static str,
    icon: IconName,
    message: &'static str,
    keyless: &'static str,
    keys: Keys,
    command: &'static str,
    able: fn(&App) -> bool,
}

/// `list_navigation`'s three, named by each binding's first alternate.
const BUTTONS: [Button; 3] = [
    Button {
        suffix: "back",
        icon: IconName::Back,
        message: "Back (%{keys})",
        keyless: "Back",
        keys: Keys::One(HISTORY_BACK.alternate(0).shown()),
        command: "history-back",
        able: |app| app.browser.can_go_back(),
    },
    Button {
        suffix: "forward",
        icon: IconName::Forward,
        message: "Forward (%{keys})",
        keyless: "Forward",
        keys: Keys::One(HISTORY_FORWARD.alternate(0).shown()),
        command: "history-forward",
        able: |app| app.browser.can_go_forward(),
    },
    Button {
        suffix: "up",
        icon: IconName::Up,
        message: "Parent folder (%{keys})",
        keyless: "Go to the parent folder",
        keys: Keys::One(PARENT_FOLDER.alternate(0).shown()),
        command: "parent-folder",
        able: |app| app.browser.can_go_parent(),
    },
];

/// The header over the listing of the view keyed `view`, for the folder named `title`.
pub(crate) fn bar(app: &App, view: &str, title: &str) -> El {
    let look = &app.look;
    let navigation = BUTTONS
        .iter()
        .fold(row().items_center(), |buttons, button| {
            buttons.child(navigation(app, view, button))
        });
    // Middle-ellipsized, as GTK's heading label is.
    let heading = text(title.to_owned())
        .font(look.kit.ty.ui_medium)
        .color(look.kit.c.text_base)
        .truncate_middle()
        .shrink()
        .min_w(0.0)
        .description(title.to_owned());
    // Other packages' actions follow the heading here, in GTK's order (the module docs).
    // GTK's order ([`actions`]): Empty Trash, Refresh, then the Icons view's thumbnail size.
    let mut actions = row()
        .key(format!("{view}-header-actions"))
        .items_center()
        .children(actions::pane(app, view));
    if view == crate::icons::VIEW {
        actions = actions.child(thumbnail_size::button(app));
    }
    row()
        .key(format!("{view}-header"))
        .w_full()
        .h(look.frame.control + 2.0 * look.space.xs)
        .px(look.space.sm)
        .gap(look.space.sm)
        .items_center()
        .child(navigation)
        .child(row().grow(1.0).min_w(0.0).child(heading))
        .child(actions)
}

fn navigation(app: &App, view: &str, button: &Button) -> El {
    let look = &app.look;
    let name = with_keys(button.message, Some(button.keyless), button.keys);
    let command = button.command;
    let built = header::action(look, button.icon, &name)
        .disabled(!(button.able)(app))
        .on_press_tx(move |app: &mut App, tx: &mut Tx| pressed(app, command, tx))
        .build(&look.kit);
    let key = format!("{view}-{}", button.suffix);
    if button.suffix != "back" {
        return buttons::on_return(key, built);
    }
    let described = with_keys(
        "Right-click, hold, or press %{keys} to list the back history",
        None,
        Keys::One(CONTEXT_MENU.shown()),
    );
    let built = built.description(described);
    if !app.browser.can_go_back() {
        return buttons::on_return(key, built);
    }
    buttons::taking_return(key, built)
        .on_layout_tx(|app: &mut App, rect, _: &mut Tx| app.pane_header.back = rect)
        .on_drag(held)
        .on_secondary_press_local(|app: &mut App, _: PointerEvent, _: &mut Tx| open(app))
        .on_key(back_key)
}

/// A press on one of the buttons: its command, unless it is Back's, and the release of a
/// press held on Back that listed the history instead.
fn pressed(app: &mut App, command: &str, tx: &mut Tx) {
    if command == "history-back" && std::mem::take(&mut app.pane_header.swallow) {
        return;
    }
    commands::run(app, command, On::Focus, tx);
}

/// The history closed, chosen from or not: a held press's release is no longer pending, as
/// Back lies inert behind the menu and its release may reach the menu's scrim instead.
fn closed(app: &mut App) {
    app.pane_header.swallow = false;
}

/// A press held on Back: its start, until it is let go.
fn held(app: &mut App, drag: &mut Drag, tx: &mut Tx) {
    let header = &mut app.pane_header;
    match drag.phase {
        Phase::Down => {
            header.held = Some(tx.clock());
            header.swallow = false;
            tx.wake_after(std::time::Duration::from_secs_f64(history::HOLD));
        }
        Phase::Move => {}
        Phase::Up => header.held = None,
    }
}

/// The Menu key or Shift+F10 on Back lists the history, read as the catalog's
/// `context_menu`; with anything else held they open nothing, as on GTK.
fn back_key(app: &mut App, event: &KeyEvent, tx: &mut Tx) -> bool {
    let at = Situation::of(app, tx);
    if commands::read(app, event, &at, tx).is(CONTEXT_MENU) {
        if !tx.key_repeat() {
            open(app);
        }
        return true;
    }
    menu_chord(event).is_some() || buttons::pressed(event, tx)
}

/// Opens the back history under Back.
fn open(app: &mut App) {
    let back = app.browser.back_locations();
    if back.is_empty() {
        return;
    }
    let home = glib::home_dir();
    let model = history::model(&app.pane_header.heading, &back, &home);
    let [x, y, _, h] = app.pane_header.back;
    app.pane_header
        .menu
        .open(MenuTarget::Background, Point::new(x, y + h), model);
}

/// A history entry was chosen: the browser goes back to it in one step.
fn chosen(app: &mut App, intent: MenuIntent, tx: &mut Tx) {
    closed(app);
    let MenuIntent::Activate { action, .. } = intent;
    let Some(index) = history::index_of(&action) else {
        return;
    };
    shell::publish(app);
    app.browser.back_to(index + 1);
    shell::settle(app, tx);
}

/// The history's surfaces, in the layer while it is open.
fn build(app: &App, frame: &mut Frame<'_, App>) -> Option<El> {
    let menu = ContextMenu::new(
        "pane-history",
        |app: &App| &app.pane_header.menu,
        |app: &mut App| &mut app.pane_header.menu,
    )
    .on_intent(chosen)
    .on_dismiss(|app: &mut App, _| closed(app))
    .build(app, &app.look, frame.layer);
    thumbnail_size::build(app, frame);
    Some(menu)
}

impl Part for PaneHeader {
    const PLACE: Place<App> = Place::Overlays(build);

    fn new(_: &Setup<'_>) -> Self {
        Self {
            menu: ContextMenuState::default(),
            labels: Labels {
                empty_trash: tr("Empty Trash"),
            },
            heading: tr("BACK TO"),
            held: None,
            swallow: false,
            back: [0.0; 4],
            thumbnails: thumbnail_size::ThumbnailSize::new(),
        }
    }

    /// A press on Back held long enough lists the history, and its release goes nowhere; the
    /// thumbnail size's slider follows the size.
    fn poll(app: &mut App, clock: f64, tx: &mut Tx) -> bool {
        let followed = thumbnail_size::follow(app, tx);
        let Some(since) = app.pane_header.held else {
            return followed;
        };
        let left = history::HOLD - (clock - since);
        if left > 1e-6 {
            tx.wake_after(std::time::Duration::from_secs_f64(left));
            return followed;
        }
        app.pane_header.held = None;
        app.pane_header.swallow = true;
        open(app);
        true
    }

    fn keyboard(app: &App) -> Option<Holds> {
        if app.pane_header.menu.is_open() {
            return Some(Holds::Menu);
        }
        app.pane_header
            .thumbnails
            .is_open()
            .then_some(Holds::Popover)
    }
}
