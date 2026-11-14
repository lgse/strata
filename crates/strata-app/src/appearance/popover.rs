// SPDX-License-Identifier: MIT

//! The Appearance popover (slice decision 21), GTK's `build_appearance_menu` in
//! `src/ui/window/appearance_menu.rs`: a plain popover ([`crate::popover`]) under the header's
//! Appearance button, which is a toggle button pressed while it is open.
//!
//! - **VIEW:** Columns, Icons and List, `radio menu item`s checked for the view shown. A row
//!   shows its view as the keys do ([`super::views::switch`]), closes the popover and gives the
//!   view the keyboard (`focus_switched_view`).
//! - **Preview panel,** a `toggle button` described with its key, drawn unpressed: it runs
//!   `preview-panel`, which the preview drawer's package registers, and is inert until then
//!   (the coordinator's ruling 1 in `docs/rune/parity/packages/A8.md`). It closes the popover.
//! - **DENSITY:** the Frame and Content sliders ([`super::density`]), which save as they move;
//!   the popover stays open.
//! - **TEXT SIZE:** "Aa", then GTK's stepper: Decrease, the size as "N px" (it resets it,
//!   described with its key), Increase, each running the text size's commands
//!   ([`super::text_size`]). The popover stays open.
//! - **Group by file type,** a `check menu item` checked while the setting is on and
//!   sensitive only where the view shown groups (List); it saves the setting, regroups the
//!   view shown and closes the popover.
//! - **Customize columns…,** a `menu item` shown in List alone: it closes the popover and opens
//!   the List table's columns menu under its headings (`customize_columns_option`, then
//!   `open_list_columns_popover`), the menu the List header's Columns button opens.
//! - **Hidden files,** a `check menu item` checked while they show, described with its key; it
//!   toggles them in the browser and closes the popover.
//!
//! On open the keyboard is on the first row, as GTK's popover opens (driven, `A8.md`). The
//! keys, Escape, a press or a wheel outside, and the keyboard leaving it are the plain
//! popover's.
//!
//! Left out until their pull requests: "Show sizes in Columns view" (Columns), and the header
//! button's icon following the view.

use rune::{El, Tx, col, icon, row, text};
use rune_kit::{Button, ButtonVariant, IconButton, IconButtonVariant};
use strata_core::i18n::tr;
use strata_desktop::keys::{
    self, Keys,
    bindings::{HIDDEN_FILES, PREVIEW, TEXT_LARGER, TEXT_RESET, TEXT_SMALLER},
};
use strata_kit::{icons::IconName, look::Look};

use super::{
    density::{self, CONTENT, FRAME},
    views::{COLUMNS, ICONS, LIST},
};
use crate::{
    App, buttons,
    commands::{self, Action, Call, On, Outcome},
    location::labels::with_keys,
    popover::{Plain, focus_left},
    shell::{self, Face},
    slots::Frame,
};

/// The header button the popover hangs from (`shell.rs` keys it `header-` and its name).
const BUTTON: &str = "header-appearance";
const SURFACE: &str = "appearance-popover";

const SHOW_COLUMNS: &str = "appearance-columns";
const SHOW_ICONS: &str = "appearance-icons";
const SHOW_LIST: &str = "appearance-list";
const PREVIEW_PANEL: &str = "appearance-preview";
const SMALLER: &str = "appearance-text-smaller";
const RESET: &str = "appearance-text-reset";
const LARGER: &str = "appearance-text-larger";
const GROUP: &str = "appearance-group";
const CUSTOMIZE: &str = "appearance-customize-columns";
const HIDDEN: &str = "appearance-hidden";

/// The popover's state: whether it is open, and its words.
pub(super) struct State {
    open: bool,
    labels: Labels,
}

/// The words, translated once: the language changes only on restart. What names a key is
/// made at each build, since a binding can change.
struct Labels {
    appearance: String,
    view: String,
    columns: String,
    icons: String,
    list: String,
    preview: String,
    density: String,
    text_size: String,
    group: String,
    group_description: String,
    customize: String,
    hidden: String,
}

impl State {
    pub(super) fn translated() -> Self {
        Self {
            open: false,
            labels: Labels {
                appearance: tr("Appearance"),
                view: tr("VIEW"),
                columns: tr("Columns"),
                icons: tr("Icons"),
                list: tr("List"),
                preview: tr("Preview panel"),
                density: tr("DENSITY"),
                text_size: tr("TEXT SIZE"),
                group: tr("Group by file type"),
                group_description: tr("Group List entries under file-type headings"),
                customize: tr("Customize columns…"),
                hidden: tr("Hidden files"),
            },
        }
    }

    pub(super) fn is_open(&self) -> bool {
        self.open
    }
}

/// The header button's command: the button opens the popover, or closes it.
pub(super) const TOGGLE: Action<App> = Action::new("appearance", toggle);

fn toggle(app: &mut App, _: &Call<'_>, tx: &mut Tx) -> Outcome {
    let state = &mut app.appearance.popover;
    state.open = !state.open;
    if state.open {
        FRAME.follow_saved(app);
        CONTENT.follow_saved(app);
        tx.focus(SHOW_COLUMNS, true);
    }
    Outcome::Ran
}

fn close(app: &mut App) {
    app.appearance.popover.open = false;
}

/// The header button as GTK names it: "Appearance", a toggle pressed while the popover is
/// open (`MenuShell` in `src/ui/window/appearance_menu.rs`).
pub(crate) fn face(app: &App) -> Face {
    let state = &app.appearance.popover;
    Face {
        name: state.labels.appearance.clone(),
        tooltip: None,
        checked: Some(state.open),
    }
}

/// GTK's popover hides when the keyboard leaves it. True when it closed.
pub(super) fn follow(app: &mut App, tx: &Tx) -> bool {
    if !app.appearance.popover.open || !focus_left(tx, BUTTON, &stops(app), &[]) {
        return false;
    }
    close(app);
    true
}

/// Whether the List view is shown: "Customize columns…" is there only.
fn lists(app: &App) -> bool {
    shell::shown_view(app).is_some_and(|view| (view.shows)(LIST))
}

/// Whether the view shown groups by file type: the Group row is sensitive only there.
fn groups(app: &App) -> bool {
    shell::shown_view(app).is_some_and(|view| view.supports_type_grouping)
}

/// The popover's controls, in order: the disabled Group row is none.
fn stops(app: &App) -> Vec<&'static str> {
    let mut stops = vec![SHOW_COLUMNS, SHOW_ICONS, SHOW_LIST, PREVIEW_PANEL];
    stops.extend(FRAME.keys);
    stops.extend(CONTENT.keys);
    stops.extend([SMALLER, RESET, LARGER]);
    if groups(app) {
        stops.push(GROUP);
    }
    if lists(app) {
        stops.push(CUSTOMIZE);
    }
    stops.push(HIDDEN);
    stops
}

/// The surface, in the layer while the popover is open.
pub(super) fn build(app: &App, frame: &mut Frame<'_, App>) {
    let state = &app.appearance.popover;
    if !state.open {
        return;
    }
    let look = &app.look;
    let body = col()
        .w(260.0)
        .padding(look.space.sm)
        .gap(look.space.xxs)
        .child(heading(look, &state.labels.view))
        .children(view_rows(app))
        .child(preview_row(app))
        .child(heading(look, &state.labels.density))
        .child(density::sliders(app))
        .child(heading(look, &state.labels.text_size))
        .child(stepper(app))
        .child(group_row(app))
        .children(lists(app).then(|| customize_row(app)))
        .child(hidden_row(app));
    let plain = Plain {
        key: SURFACE,
        anchor: BUTTON,
        label: state.labels.appearance.clone(),
        stops: stops(app),
        own_arrows: &[FRAME.keys[0], CONTENT.keys[0]],
        listing: shell::view_focus(app),
        close,
        parent: None,
    };
    plain.build(look, body, frame);
}

fn heading(look: &Look, words: &str) -> El {
    text(words.to_owned())
        .font(look.ty.heading)
        .color(look.kit.c.text_muted)
        .px(look.space.sm)
        .py(look.space.xs)
}

/// What a row shows: its icon, its name, its key if it has one, and a check while it is on.
struct Row<'a> {
    key: &'static str,
    role: &'static str,
    icon: IconName,
    name: &'a str,
    shown_keys: Option<String>,
    checked: bool,
    sensitive: bool,
    press: fn(&mut App, &mut Tx),
}

/// A row as GTK's `appearance_option` makes one: a frameless button with the row's role,
/// named by its label and, when it shows a key, described by it (`describe_menu_item`).
fn option(look: &Look, row_of: Row<'_>) -> El {
    let kit = &look.kit;
    let name = row_of.name.to_owned();
    let keys_shown = row_of.shown_keys.clone();
    let checked = row_of.checked;
    let check = look.icon(IconName::Check);
    let press = row_of.press;
    let built = Button::new(name.clone())
        .variant(ButtonVariant::Ghost)
        .icon(look.icon(row_of.icon))
        .disabled(!row_of.sensitive)
        .label_with(move |font, ink| {
            let keys_text = keys_shown.map(|shown| text(shown).font(font).color(ink));
            row()
                .grow(1.0)
                .gap(8.0)
                .child(text(name).font(font).color(ink).grow(1.0))
                .children(keys_text)
                .children(checked.then(|| icon(check, 16.0).color(ink)))
        })
        .on_press_tx(move |app: &mut App, tx: &mut Tx| press(app, tx))
        .build(kit)
        .w_full()
        .role(row_of.role)
        .checked(checked);
    let built = match &row_of.shown_keys {
        Some(shown) => built.description(shown.clone()),
        None => built,
    };
    buttons::on_return(row_of.key, built)
}

fn view_rows(app: &App) -> Vec<El> {
    let labels = &app.appearance.popover.labels;
    let shown = |mode| shell::shown_view(app).is_some_and(|view| (view.shows)(mode));
    let row = |key, icon, name, mode, press| {
        let checked = shown(mode);
        let role = "menuitemradio";
        let row = Row {
            key,
            role,
            icon,
            name,
            shown_keys: None,
            checked,
            sensitive: true,
            press,
        };
        option(&app.look, row)
    };
    vec![
        row(
            SHOW_COLUMNS,
            IconName::ViewColumns,
            &labels.columns,
            COLUMNS,
            |app, tx| {
                chosen_view(app, COLUMNS, tx);
            },
        ),
        row(
            SHOW_ICONS,
            IconName::ViewIcons,
            &labels.icons,
            ICONS,
            |app, tx| {
                chosen_view(app, ICONS, tx);
            },
        ),
        row(
            SHOW_LIST,
            IconName::ViewList,
            &labels.list,
            LIST,
            |app, tx| {
                chosen_view(app, LIST, tx);
            },
        ),
    ]
}

/// A view row: the view shows as the keys show it, the popover closes, and the view has the
/// keyboard (`connect_mode_buttons`, then `focus_switched_view`).
fn chosen_view(app: &mut App, mode: &'static str, tx: &mut Tx) {
    super::views::switch(app, mode, tx);
    close(app);
    if let Some(view) = shell::view_focus(app) {
        tx.focus(view, true);
    }
}

fn preview_row(app: &App) -> El {
    let labels = &app.appearance.popover.labels;
    let shown = keys::label(&Keys::One(PREVIEW.shown()));
    let described = if shown.is_empty() {
        tr("Toggle preview panel while browsing")
    } else {
        rust_i18n::t!(
            "Toggle preview panel while browsing (%{text})",
            text = shown
        )
        .into_owned()
    };
    let row = option(
        &app.look,
        Row {
            key: PREVIEW_PANEL,
            role: "button",
            icon: IconName::Eye,
            name: &labels.preview,
            shown_keys: (!shown.is_empty()).then_some(shown),
            checked: false,
            sensitive: true,
            press: |app, tx| {
                commands::run(app, "preview-panel", On::Focus, tx);
                close(app);
            },
        },
    );
    row.description(described)
}

/// "Aa" and GTK's stepper: Decrease, the size (a reset), Increase (`text_size_stepper`).
fn stepper(app: &App) -> El {
    let look = &app.look;
    let kit = &look.kit;
    let size = shell::preferences(app).text_size.root_font_px();
    let step = |key: &'static str, icon: IconName, name: String, command: &'static str| {
        let button = IconButton::new(look.icon(icon))
            .variant(IconButtonVariant::Ghost)
            .label(name)
            .on_press_tx(move |app: &mut App, tx: &mut Tx| {
                commands::run(app, command, On::Focus, tx);
            })
            .build(kit);
        buttons::on_return(key, button)
    };
    let smaller = with_keys(
        "Decrease text size (%{keys})",
        Some("Decrease text size"),
        Keys::One(TEXT_SMALLER.shown()),
    );
    let larger = with_keys(
        "Increase text size (%{keys})",
        Some("Increase text size"),
        Keys::One(TEXT_LARGER.alternate(0).shown()),
    );
    let reset_described = with_keys(
        "Reset text size (%{keys})",
        Some("Reset text size"),
        Keys::One(TEXT_RESET.shown()),
    );
    let reset = Button::new(format!("{size} px"))
        .variant(ButtonVariant::Ghost)
        .on_press_tx(|app: &mut App, tx: &mut Tx| {
            commands::run(app, "text-reset", On::Focus, tx);
        })
        .build(kit)
        .description(reset_described);
    row()
        .w_full()
        .gap(look.space.sm)
        .px(look.space.sm)
        .items_center()
        .child(text("Aa").font(kit.ty.ui).color(kit.c.text_base).grow(1.0))
        .child(step(SMALLER, IconName::Minus, smaller, "text-smaller"))
        .child(buttons::on_return(RESET, reset))
        .child(step(LARGER, IconName::Plus, larger, "text-larger"))
}

fn group_row(app: &App) -> El {
    let labels = &app.appearance.popover.labels;
    let row = option(
        &app.look,
        Row {
            key: GROUP,
            role: "menuitemcheckbox",
            icon: IconName::ListChecks,
            name: &labels.group,
            shown_keys: None,
            checked: shell::preferences(app).group_by_type,
            sensitive: groups(app),
            press: |app, tx| {
                let on = !shell::preferences(app).group_by_type;
                shell::saved(app).change(|settings| settings.group_by_type = on);
                shell::regroup(app, tx);
                close(app);
            },
        },
    );
    row.description(labels.group_description.clone())
}

fn customize_row(app: &App) -> El {
    let labels = &app.appearance.popover.labels;
    option(
        &app.look,
        Row {
            key: CUSTOMIZE,
            role: "menuitem",
            icon: IconName::ViewColumns,
            name: &labels.customize,
            shown_keys: None,
            checked: false,
            sensitive: true,
            press: |app, tx| {
                close(app);
                crate::list::customize_columns(app, tx);
            },
        },
    )
}

fn hidden_row(app: &App) -> El {
    let labels = &app.appearance.popover.labels;
    let shown_hidden = app.browser.preferences().show_hidden;
    let icon = if shown_hidden {
        IconName::Eye
    } else {
        IconName::EyeOff
    };
    let shown = keys::label(&Keys::One(HIDDEN_FILES.alternate(0).shown()));
    option(
        &app.look,
        Row {
            key: HIDDEN,
            role: "menuitemcheckbox",
            icon,
            name: &labels.hidden,
            shown_keys: (!shown.is_empty()).then_some(shown),
            checked: shown_hidden,
            sensitive: true,
            press: |app, tx| {
                shell::publish(app);
                app.browser.toggle_hidden();
                close(app);
                shell::settle(app, tx);
            },
        },
    )
}
