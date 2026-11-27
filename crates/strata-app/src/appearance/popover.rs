// SPDX-License-Identifier: MIT

use rune::{Cursor, El, TextStyle, Tx, col, el, icon, row, text};
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
    popover::{Kind, Plain, focus_left},
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
const SIZES: &str = "appearance-column-sizes";
const HIDDEN: &str = "appearance-hidden";

/// The popover's state: whether it is open, and its words.
pub(super) struct State {
    open: bool,
    labels: Labels,
}

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
    sizes: String,
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
                sizes: tr("Show sizes in Columns view"),
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
        tx.focus(SHOW_COLUMNS, tx.by_keys());
    }
    Outcome::Ran
}

fn close(app: &mut App) {
    app.appearance.popover.open = false;
}

pub(crate) fn button_icon(app: &App) -> IconName {
    let Some(view) = shell::shown_view(app) else {
        return IconName::ViewColumns;
    };
    if (view.shows)(ICONS) {
        IconName::ViewIcons
    } else if (view.shows)(LIST) {
        IconName::ViewList
    } else {
        IconName::ViewColumns
    }
}

pub(crate) fn face(app: &App) -> Face {
    let state = &app.appearance.popover;
    Face {
        name: state.labels.appearance.clone(),
        tooltip: None,
        checked: Some(state.open),
    }
}

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

/// Whether the Columns view is shown: "Show sizes in Columns view" is there only.
fn in_columns(app: &App) -> bool {
    shell::shown_view(app).is_some_and(|view| (view.shows)(COLUMNS))
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
    if in_columns(app) {
        stops.push(SIZES);
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
        .px(look.space.md)
        .py(look.space.md)
        .gap(look.space.xxs)
        .child(heading(look, &state.labels.view))
        .children(view_rows(app))
        .child(preview_row(app))
        .child(separator(look))
        .child(heading(look, &state.labels.density))
        .child(density::sliders(app))
        .child(separator(look))
        .child(heading(look, &state.labels.text_size))
        .child(stepper(app))
        .child(separator(look))
        .child(group_row(app))
        .children(lists(app).then(|| customize_row(app)))
        .children(in_columns(app).then(|| sizes_row(app)))
        .child(hidden_row(app));
    let plain = Plain {
        key: SURFACE,
        anchor: BUTTON.into(),
        kind: Kind::Dialog,
        label: state.labels.appearance.clone(),
        stops: stops(app),
        own_arrows: &[FRAME.keys[0], CONTENT.keys[0]],
        listing: shell::view_focus(app),
        viewports: shell::listing_viewports(app),
        close,
        parent: None,
    };
    plain.build(look, body, frame);
}

fn heading(look: &Look, words: &str) -> El {
    let font = TextStyle {
        weight: 700.0,
        ..look.ty.heading
    };
    text(words.to_owned())
        .font(font)
        .color(look.kit.c.text_muted)
        .px(look.space.md)
        .py(look.space.xs)
}

fn separator(look: &Look) -> El {
    el().w_full()
        .h(1.0)
        .my(look.space.sm)
        .fill(look.kit.c.border_muted)
        .a11y_hidden()
}

/// What a row shows: its icon, its name, its key if it has one, and a check while it is on.
struct Row<'a> {
    key: &'static str,
    role: &'static str,
    icon: IconName,
    name: &'a str,
    shown_keys: Option<String>,
    checked: bool,
    tinted_checked: bool,
    sensitive: bool,
    press: fn(&mut App, &mut Tx),
}

fn option(look: &Look, row_of: Row<'_>) -> El {
    let kit = &look.kit;
    let name = row_of.name.to_owned();
    let press = row_of.press;
    let keys_el = row_of.shown_keys.as_ref().map(|shown| {
        text(shown.clone())
            .font(kit.ty.small)
            .color(kit.c.text_muted)
            .a11y_hidden()
    });
    let check_el = row_of.checked.then(|| {
        icon(look.icon(IconName::Check), look.icon_size.md)
            .color(kit.c.icon_accent)
            .a11y_hidden()
    });
    let mut built = row()
        .w_full()
        .min_h(look.row.columns)
        .px(look.space.md)
        .py(look.space.xs)
        .gap(look.space.md)
        .items_center()
        .radius(kit.radius.sm)
        .role(row_of.role)
        .label(name.clone())
        .checked(row_of.checked)
        .child(
            icon(look.icon(row_of.icon), look.icon_size.md)
                .color(kit.c.icon_accent)
                .a11y_hidden(),
        )
        .child(
            text(name)
                .font(kit.ty.ui)
                .color(kit.c.text_base)
                .grow(1.0)
                .a11y_hidden(),
        )
        .children(keys_el)
        .children(check_el);
    if let Some(shown) = &row_of.shown_keys {
        built = built.description(shown.clone());
    }
    built = style_option(kit, built, &row_of, press);
    buttons::on_return(row_of.key, built)
}

fn style_option(
    kit: &rune_kit::Kit,
    built: El,
    row_of: &Row<'_>,
    press: fn(&mut App, &mut Tx),
) -> El {
    if !row_of.sensitive {
        return built.disabled(true).layer_opacity(0.42);
    }
    let interactive = built
        .focusable(true)
        .cursor(Cursor::Pointer)
        .focus_ring(2.0, -2.0, kit.alpha(kit.c.border_focus, 0.6))
        .press(kit.c.pressed)
        .on_press_tx(move |app: &mut App, tx: &mut Tx| press(app, tx));
    if row_of.tinted_checked && row_of.checked {
        interactive
            .fill(kit.alpha(kit.c.bg_accent, 0.10))
            .hover(kit.alpha(kit.c.bg_accent, 0.18))
    } else {
        interactive.hover(kit.c.hover)
    }
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
            tinted_checked: false,
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
            checked: crate::preview::panel_pressed(app),
            tinted_checked: true,
            sensitive: true,
            press: |app, tx| {
                commands::run(app, "preview-panel", On::Focus, tx);
                close(app);
            },
        },
    );
    row.description(described)
}

fn stepper(app: &App) -> El {
    let look = &app.look;
    let kit = &look.kit;
    let size = shell::preferences(app).text_size.root_font_px();
    let step_h = look.row.columns.max(26.0);
    let inner_r = kit.radius.sm;
    let sample_font = TextStyle {
        weight: 700.0,
        ..kit.ty.ui
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
    let reset = reset_button(look, size, step_h);
    let segment = row()
        .items_center()
        .fill(kit.alpha(kit.c.text_base, 0.04))
        .border(1.0, kit.c.border_base)
        .border_layout()
        .radius(kit.radius.md)
        .child(step_button(
            look,
            StepSpec {
                key: SMALLER,
                icon: IconName::Minus,
                name: smaller,
                command: "text-smaller",
                height: step_h,
                corners: [inner_r, 0.0, 0.0, inner_r],
            },
        ))
        .child(reset)
        .child(step_button(
            look,
            StepSpec {
                key: LARGER,
                icon: IconName::Plus,
                name: larger,
                command: "text-larger",
                height: step_h,
                corners: [0.0, inner_r, inner_r, 0.0],
            },
        ));
    row()
        .w_full()
        .gap(look.space.md)
        .px(look.space.md)
        .py(look.space.xxs)
        .items_center()
        .child(
            text("Aa")
                .font(sample_font)
                .color(kit.c.text_accent)
                .grow(1.0)
                .a11y_hidden(),
        )
        .child(segment)
}

fn reset_button(look: &Look, size: u32, height: f32) -> El {
    let kit = &look.kit;
    let reset_described = with_keys(
        "Reset text size (%{keys})",
        Some("Reset text size"),
        Keys::One(TEXT_RESET.shown()),
    );
    let value_label = format!("{size} px");
    let reset = row()
        .h(height)
        .px(7.0)
        .center()
        .role("button")
        .label(value_label.clone())
        .description(reset_described)
        .border_sides([0.0, 1.0, 0.0, 1.0], kit.c.border_base)
        .focusable(true)
        .cursor(Cursor::Pointer)
        .hover(kit.c.hover)
        .press(kit.c.pressed)
        .focus_ring(2.0, -2.0, kit.c.border_focus)
        .on_press_tx(|app: &mut App, tx: &mut Tx| {
            commands::run(app, "text-reset", On::Focus, tx);
        })
        .child(
            text(value_label)
                .font(kit.ty.ui)
                .color(kit.c.text_base)
                .a11y_hidden(),
        );
    buttons::on_return(RESET, reset)
}

struct StepSpec {
    key: &'static str,
    icon: IconName,
    name: String,
    command: &'static str,
    height: f32,
    corners: [f32; 4],
}

fn step_button(look: &Look, spec: StepSpec) -> El {
    let kit = &look.kit;
    let command = spec.command;
    let button = row()
        .min_w(spec.height)
        .h(spec.height)
        .center()
        .corners(spec.corners)
        .role("button")
        .label(spec.name.clone())
        .help(spec.name)
        .focusable(true)
        .cursor(Cursor::Pointer)
        .hover(kit.c.hover)
        .press(kit.c.pressed)
        .focus_ring(2.0, -2.0, kit.c.border_focus)
        .on_press_tx(move |app: &mut App, tx: &mut Tx| {
            commands::run(app, command, On::Focus, tx);
        })
        .child(
            icon(look.icon(spec.icon), look.icon_size.md)
                .color(kit.c.icon_accent)
                .a11y_hidden(),
        );
    buttons::on_return(spec.key, button)
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
            tinted_checked: false,
            sensitive: groups(app),
            press: |app, tx| {
                let on = !shell::preferences(app).group_by_type;
                shell::saved(app).change(|settings| settings.group_by_type = on);
                crate::filter::close_for_regroup(app, tx);
                shell::regroup(app, tx);
                close(app);
                tx.focus(BUTTON, true);
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
            tinted_checked: false,
            sensitive: true,
            press: |app, tx| {
                close(app);
                crate::list::customize_columns(app, tx);
            },
        },
    )
}

fn sizes_row(app: &App) -> El {
    let labels = &app.appearance.popover.labels;
    option(
        &app.look,
        Row {
            key: SIZES,
            role: "menuitemcheckbox",
            icon: IconName::ViewColumns,
            name: &labels.sizes,
            shown_keys: None,
            checked: shell::preferences(app).columns_show_sizes,
            tinted_checked: false,
            sensitive: true,
            press: |app, _| {
                let on = !shell::preferences(app).columns_show_sizes;
                shell::saved(app).change(|settings| settings.columns_show_sizes = on);
                app.columns.show_sizes(on);
                close(app);
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
            tinted_checked: false,
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
