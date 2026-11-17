// SPDX-License-Identifier: MIT

//! The Settings dialog (`Ctrl+,` and the header's Settings button), theme catalog, and
//! one-per-streak save notice (`docs/rune/parity/packages/A18.md`).

use rune::{El, ScrollSpec, TextEdit, Tx, col, icon, row, text};
use rune_kit::{Button, ButtonVariant, IconButton, IconButtonVariant};
use strata_components::{
    controls::{
        number_field::{NumberFieldState, Range},
        settings_search::{SearchIndex, SettingsSearchState},
    },
    dialog::DialogShell,
};
use strata_core::i18n::tr;
use strata_desktop::{
    build_info::updater_enabled,
    keys::{Modifiers, bindings::SETTINGS},
    settings::{CornerRoundness, MAX_PREVIEW_VIDEO_START, Preferences, TextSize},
};
use strata_kit::icons::IconName;

use crate::{
    App, buttons,
    commands::{Action, Call, Holds, Keys, Outcome as CommandOutcome, Situation, Stage},
    shell,
    slots::{Frame, Part, Place, Scope, Setup},
};

mod appearance_page;
mod controls;
mod general;
mod general_more;
mod keyboard;
mod notice;
mod pages;
mod search_index;
#[cfg(test)]
mod tests;

const DIALOG_WIDTH: f32 = 920.0;
const DIALOG_HEIGHT: f32 = 620.0;
const RAIL_WIDTH: f32 = 220.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum PageId {
    #[default]
    General,
    Appearance,
    Actions,
    Keyboard,
    Integrations,
    Updates,
    Diagnostics,
    About,
}

struct NavEntry {
    id: PageId,
    key: &'static str,
    label: &'static str,
    subtitle: &'static str,
    accessible: &'static str,
    icon: IconName,
}

const NAVIGATION: [NavEntry; 8] = [
    NavEntry {
        id: PageId::General,
        key: "general",
        label: "General",
        subtitle: "Browsing, search, files",
        accessible: "General",
        icon: IconName::Settings,
    },
    NavEntry {
        id: PageId::Appearance,
        key: "theme",
        label: "Appearance",
        subtitle: "Theme, text, motion",
        accessible: "Appearance settings",
        icon: IconName::Eye,
    },
    NavEntry {
        id: PageId::Actions,
        key: "actions",
        label: "Actions",
        subtitle: "Custom actions",
        accessible: "Actions",
        icon: IconName::Play,
    },
    NavEntry {
        id: PageId::Keyboard,
        key: "keyboard",
        label: "Keyboard",
        subtitle: "Shortcuts for each mode",
        accessible: "Keyboard settings",
        icon: IconName::Keyboard,
    },
    NavEntry {
        id: PageId::Integrations,
        key: "integrations",
        label: "Integrations",
        subtitle: "Apps and services",
        accessible: "Integrations",
        icon: IconName::Properties,
    },
    NavEntry {
        id: PageId::Updates,
        key: "updates",
        label: "Updates",
        subtitle: "Channel, release notes",
        accessible: "Updates",
        icon: IconName::Downloads,
    },
    NavEntry {
        id: PageId::Diagnostics,
        key: "diagnostics",
        label: "Diagnostics",
        subtitle: "Troubleshooting",
        accessible: "Diagnostics",
        icon: IconName::Info,
    },
    NavEntry {
        id: PageId::About,
        key: "about",
        label: "About",
        subtitle: "Version and links",
        accessible: "About",
        icon: IconName::Help,
    },
];

impl PageId {
    pub(super) fn from_key(key: &str) -> Option<Self> {
        NAVIGATION
            .iter()
            .find_map(|entry| (entry.key == key && entry_shown(entry)).then_some(entry.id))
    }

    const fn key(self) -> &'static str {
        match self {
            Self::General => "general",
            Self::Appearance => "theme",
            Self::Actions => "actions",
            Self::Keyboard => "keyboard",
            Self::Integrations => "integrations",
            Self::Updates => "updates",
            Self::Diagnostics => "diagnostics",
            Self::About => "about",
        }
    }

    const fn title(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Appearance => "Appearance",
            Self::Actions => "Actions",
            Self::Keyboard => "Keyboard",
            Self::Integrations => "Integrations",
            Self::Updates => "Updates",
            Self::Diagnostics => "Diagnostics",
            Self::About => "About",
        }
    }

    fn nav_key(self) -> String {
        format!("settings-nav-{}", self.key())
    }
}

fn entry_shown(entry: &NavEntry) -> bool {
    entry.id != PageId::Updates || updater_enabled()
}

pub(crate) struct SettingsDialog {
    open: bool,
    page: PageId,
    opened_page: PageId,
    open_choice: Option<&'static str>,
    search: SettingsSearchState,
    index: SearchIndex,
    indexed_locale: String,
    workers: NumberFieldState,
    video_start: NumberFieldState,
    text_size: NumberFieldState,
    roundness: NumberFieldState,
    theme_query: TextEdit,
    theme_filter: appearance_page::ThemeFilter,
    notice: notice::NoticeState,
    keyboard: keyboard::KeyboardState,
}

impl SettingsDialog {
    fn from_preferences(prefs: &Preferences) -> Self {
        let locale = rust_i18n::locale().to_string();
        let avail = search_index::Availability {
            system: true,
            omarchy: false,
        };
        let index = search_index::build_index(1, avail);
        let workers =
            NumberFieldState::new(prefs.thumbnail_workers as f64, Range::new(1.0, 16.0, 1.0));
        let video_start = NumberFieldState::new(
            f64::from(prefs.preview_video_start),
            Range::new(0.0, f64::from(MAX_PREVIEW_VIDEO_START), 5.0),
        )
        .suffix("%");
        let text_size = NumberFieldState::new(
            f64::from(prefs.text_size.root_font_px()),
            Range::new(f64::from(TextSize::MIN), f64::from(TextSize::MAX), 1.0),
        )
        .suffix(" px");
        let roundness = NumberFieldState::new(
            f64::from(prefs.corner_roundness.percent()),
            Range::new(
                f64::from(CornerRoundness::MIN),
                f64::from(CornerRoundness::MAX),
                f64::from(CornerRoundness::STEP),
            ),
        )
        .suffix("%");
        Self {
            open: false,
            page: PageId::General,
            opened_page: PageId::General,
            open_choice: None,
            search: SettingsSearchState::new(),
            index,
            indexed_locale: locale,
            workers,
            video_start,
            text_size,
            roundness,
            theme_query: TextEdit::default(),
            theme_filter: appearance_page::ThemeFilter::All,
            notice: notice::NoticeState::default(),
            keyboard: keyboard::KeyboardState::default(),
        }
    }

    pub(super) fn is_open(&self) -> bool {
        self.open
    }

    pub(super) fn set_page(&mut self, page: PageId) {
        if self.page != page {
            self.page = page;
            self.open_choice = None;
            self.keyboard.end_capture();
        }
    }

    fn text_field_focused(&self) -> bool {
        self.search.edit().focused
            || self.theme_query.focused
            || self.workers.edit().focused
            || self.video_start.edit().focused
            || self.text_size.edit().focused
            || self.roundness.edit().focused
            || self.keyboard.query.focused
    }

    /// Whether a message over the pages has the keyboard: the save notice or Reset all's
    /// confirmation.
    fn message_open(&self) -> bool {
        self.notice.is_open() || self.keyboard.confirm_open()
    }
}

const OPEN_SETTINGS: Action<App> = Action::new("settings", open_command)
    .command(SETTINGS)
    .keys(&[
        Keys::at(Stage::Unclaimed, &[SETTINGS.ignoring(Modifiers::empty())]).when(no_modal_open),
    ]);

fn no_modal_open(app: &App, _: &Situation) -> bool {
    !shell::modal_open(app)
}

fn open_command(app: &mut App, _: &Call<'_>, tx: &mut Tx) -> CommandOutcome {
    open_settings_tx(app, tx);
    CommandOutcome::Ran
}

pub(crate) fn open_settings_tx(app: &mut App, tx: &mut Tx) {
    if app.settings_dialog.open {
        return;
    }
    let avail = search_index::Availability {
        system: shell::themes(app).system_available(),
        omarchy: shell::themes(app).omarchy_available(),
    };
    let prefs = shell::preferences(app).clone();
    let state = &mut app.settings_dialog;
    controls::sync_search_index_if_needed(&mut state.index, &mut state.indexed_locale, avail);
    general::sync_general_fields(&mut state.workers, &mut state.video_start, &prefs);
    appearance_page::sync_appearance_fields(&mut state.text_size, &mut state.roundness, &prefs);
    state.keyboard = keyboard::KeyboardState::opened(prefs.tenxer_mode);
    state.open = true;
    state.open_choice = None;
    state.opened_page = state.page;
    tx.focus_when_shown(state.page.nav_key(), tx.focus_visible() || tx.by_keys());
}

pub(super) fn close_settings(app: &mut App) {
    let state = &mut app.settings_dialog;
    state.open = false;
    state.open_choice = None;
    state.search = SettingsSearchState::new();
    state.theme_query.replace_text("");
    state.theme_filter = appearance_page::ThemeFilter::All;
    state.keyboard = keyboard::KeyboardState::default();
}

fn dismiss_settings_or_notice(app: &mut App, tx: &mut Tx) {
    if app.settings_dialog.notice.is_open() {
        notice::dismiss_tx(app, tx);
        return;
    }
    if app.settings_dialog.keyboard.confirm_open() {
        keyboard::reset_all::dismiss(app, tx);
        return;
    }
    close_settings(app);
}

impl Part for SettingsDialog {
    const SCOPE: Scope = Scope::Window;
    const PLACE: Place<App> = Place::Dialogs(build);
    const COMMANDS: &'static [Action<App>] = &[OPEN_SETTINGS];

    fn new(setup: &Setup<'_>) -> Self {
        Self::from_preferences(setup.settings)
    }

    fn keyboard(app: &App) -> Option<Holds> {
        let dialog = &app.settings_dialog;
        if !dialog.open && !dialog.notice.is_open() {
            return None;
        }
        Some(Holds::Modal {
            field: dialog.open && !dialog.message_open() && dialog.text_field_focused(),
        })
    }

    fn poll(app: &mut App, clock: f64, tx: &mut Tx) -> bool {
        let mut changed = notice::poll(app, tx);
        if !app.settings_dialog.open {
            return changed;
        }
        let avail = search_index::Availability {
            system: shell::themes(app).system_available(),
            omarchy: shell::themes(app).omarchy_available(),
        };
        {
            let state = &mut app.settings_dialog;
            controls::sync_search_index_if_needed(
                &mut state.index,
                &mut state.indexed_locale,
                avail,
            );
        }
        changed |= controls::search_handle().tick(app, clock, tx);
        changed |= general_more::workers_field().poll(app, tx);
        changed |= general::video_start_field().poll(app, tx);
        changed |= appearance_page::text_size_field().poll(app, tx);
        changed |= appearance_page::roundness_field().poll(app, tx);
        changed |= keyboard::poll(app, tx);
        let prefs = shell::preferences(app).clone();
        let state = &mut app.settings_dialog;
        general::sync_general_fields(&mut state.workers, &mut state.video_start, &prefs);
        appearance_page::sync_appearance_fields(&mut state.text_size, &mut state.roundness, &prefs);
        changed
    }
}

fn build(app: &App, frame: &mut Frame<'_, App>) -> Option<El> {
    if !app.settings_dialog.open {
        notice::standalone_dialog(app, frame);
        return None;
    }
    let search = controls::search_handle();
    let mut menu_layer = Vec::new();
    let rail = build_rail(app, &search);
    let main = build_main_column(app, &search, &mut menu_layer);
    let notice_open = app.settings_dialog.message_open();
    let mut body = row()
        .key("settings-body")
        .w_full()
        .h(DIALOG_HEIGHT)
        .min_h(0.0)
        .child(rail)
        .child(main);
    if notice_open {
        body = body.inert();
    }
    let mut stack = col().key("settings-stack").w_full().min_h(0.0).child(body);
    if let Some(overlay) = notice::inline_overlay(app).or_else(|| keyboard::reset_all_overlay(app))
    {
        stack = stack.child(overlay);
    }
    let mut shell = DialogShell::new("settings", tr("Settings"), close_settings)
        .on_dismiss_tx(dismiss_settings_or_notice)
        .open(true)
        .fade_only()
        .reduced_motion(frame.cx.is_reduced_motion())
        .width(DIALOG_WIDTH)
        .child(stack);
    if let Some(view) = shell::view_focus(app) {
        shell = shell.return_focus(view);
    }
    shell.build(&app.look, frame.layer);
    frame.layer.extend(menu_layer);
    None
}

fn build_rail(
    app: &App,
    search: &strata_components::controls::settings_search::SettingsSearch<App>,
) -> El {
    let look = &app.look;
    let c = &look.kit.c;
    let nav_items: Vec<El> = NAVIGATION
        .iter()
        .filter(|entry| entry_shown(entry) && search.shows_page(app, entry.key))
        .map(|entry| nav_button(app, entry))
        .collect();
    let nav_list = col()
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .gap(look.space.xxs)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .children(nav_items);
    col()
        .key("settings-rail")
        .w(RAIL_WIDTH)
        .h_full()
        .min_h(0.0)
        .padding(look.space.md)
        .gap(look.space.md)
        .fill(c.layer_02)
        .border_sides([0.0, 1.0, 0.0, 0.0], c.border_muted)
        .child(
            text(tr("SETTINGS"))
                .font(look.ty.heading)
                .color(c.text_muted)
                .px(look.space.xs),
        )
        .child(search.field(app, look))
        .child(nav_list)
}

fn nav_button(app: &App, entry: &NavEntry) -> El {
    let look = &app.look;
    let kit = &look.kit;
    let active = app.settings_dialog.page == entry.id;
    let glyph = look.icon(entry.icon);
    let label = tr(entry.label);
    let subtitle = tr(entry.subtitle);
    let a11y_label = tr(entry.accessible);
    let page = entry.id;
    let muted = kit.c.text_muted;
    let small = kit.ty.small;
    let variant = if active {
        ButtonVariant::Neutral
    } else {
        ButtonVariant::Ghost
    };
    let mut btn = Button::new(label.clone())
        .variant(variant)
        .label_with(move |font, ink| {
            row()
                .grow(1.0)
                .gap(10.0)
                .items_center()
                .child(icon(glyph, 16.0).color(ink).a11y_hidden())
                .child(
                    col()
                        .grow(1.0)
                        .gap(1.0)
                        .child(text(label).font(font).color(ink).a11y_hidden())
                        .child(text(subtitle).font(small).color(muted).a11y_hidden()),
                )
        })
        .on_press_tx(move |app: &mut App, _: &mut Tx| {
            app.settings_dialog.set_page(page);
        })
        .build(kit)
        .w_full()
        .label(a11y_label);
    if entry.id == app.settings_dialog.opened_page {
        btn = btn.autofocus();
    }
    buttons::on_return(entry.id.nav_key(), btn)
}

fn build_main_column(
    app: &App,
    search: &strata_components::controls::settings_search::SettingsSearch<App>,
    menu_layer: &mut Vec<El>,
) -> El {
    let look = &app.look;
    let c = &look.kit.c;
    let page = app.settings_dialog.page;
    let heading = if app.settings_dialog.search.no_results() {
        search.results_title().to_owned()
    } else {
        tr(page.title())
    };
    let close = buttons::on_return(
        "settings-close",
        IconButton::new(look.icon(IconName::Close))
            .variant(IconButtonVariant::Ghost)
            .label(tr("Close settings"))
            .on_press_tx(|app: &mut App, _: &mut Tx| close_settings(app))
            .build(&look.kit),
    );
    let topbar = row()
        .w_full()
        .px(look.space.xl)
        .py(look.space.md)
        .gap(look.space.md)
        .items_center()
        .border_sides([0.0, 0.0, 1.0, 0.0], c.border_muted)
        .child(
            text(heading)
                .font(look.ty.title)
                .color(c.text_base)
                .grow(1.0),
        )
        .child(close);
    let body = if app.settings_dialog.search.no_results() {
        col()
            .w_full()
            .grow(1.0)
            .px(look.space.xl)
            .py(look.space.xl)
            .child(search.empty(look))
    } else {
        let content = build_page_content(app, page, search, menu_layer);
        col()
            .key(controls::PAGE_SCROLL)
            .w_full()
            .grow(1.0)
            .min_h(0.0)
            .px(look.space.xl)
            .py(look.space.lg)
            .gap(look.space.sm)
            .scroll(ScrollSpec {
                stick_bottom: false,
            })
            .children(content)
    };
    col()
        .w_full()
        .grow(1.0)
        .h_full()
        .min_w(0.0)
        .min_h(0.0)
        .child(topbar)
        .child(body)
}

fn build_page_content(
    app: &App,
    page: PageId,
    search: &strata_components::controls::settings_search::SettingsSearch<App>,
    menu_layer: &mut Vec<El>,
) -> Vec<El> {
    match page {
        PageId::General => general::build(app, search, menu_layer),
        PageId::Appearance => appearance_page::build(app, search, menu_layer),
        PageId::Actions => pages::actions_page(app, search),
        PageId::Keyboard => keyboard::build(app, search),
        PageId::Integrations => pages::integrations_page(app, search),
        PageId::Updates => pages::updates_page(app, search, menu_layer),
        PageId::Diagnostics => pages::diagnostics_page(app, search),
        PageId::About => pages::about_page(app, search),
    }
}

pub(super) fn restart_process(app: &mut App, tx: &mut Tx) {
    #[cfg(not(test))]
    spawn_relaunch_after_exit();
    shell::close_window(app, tx);
}

#[cfg(not(test))]
fn spawn_relaunch_after_exit() {
    use std::{
        os::unix::process::CommandExt,
        process::{Command, Stdio},
    };
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let pid = std::process::id().to_string();
    let _ = Command::new("/bin/sh")
        .args([
            "-c",
            "while kill -0 \"$1\" 2>/dev/null; do sleep 0.1; done; exec \"$2\"",
            "strata-rune-restart",
        ])
        .arg(pid)
        .arg(exe)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn();
}
