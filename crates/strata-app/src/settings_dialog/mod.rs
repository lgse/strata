// SPDX-License-Identifier: MIT

//! The Settings dialog (`Ctrl+,` and the header's Settings button), theme catalog, and
//! one-per-streak save notice (`docs/rune/parity/packages/A18.md`).

use rune::{Color, Cursor, El, ScrollSpec, TextEdit, Tx, col, el, icon, row, text};
use strata_components::{
    controls::{
        number_field::{NumberFieldState, Range},
        settings_search::{SearchIndex, SettingsSearchState},
    },
    dialog::DialogShell,
    scroll::{Bar, overlay},
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

mod actions;
mod appearance_page;
mod controls;
mod diagnostics;
mod general;
mod general_more;
mod install_command;
mod integrations;
mod keyboard;
mod message;
mod notice;
mod pages;
mod search_index;
#[cfg(test)]
mod tests;
mod transparency_section;

// Match GTK's responsive settings panel (`src/ui/settings/responsive.rs`, `src/style.css`):
// 1250x800 capped to the window minus a 24 px margin (`look.space.xxl`), 240 px navigation
// rail (216 px min-width + 12 px horizontal padding each side), and 48x24 content padding
// that narrows to 20x18 on compact viewports.
const DIALOG_WIDTH: f32 = 1250.0;
const DIALOG_HEIGHT: f32 = 800.0;
const RAIL_WIDTH: f32 = 240.0;
const COMPACT_CONTENT_WINDOW_WIDTH: i32 = 1000;
const CONTENT_PAD_X: f32 = 48.0;
const CONTENT_PAD_Y: f32 = 24.0;
const CONTENT_BOTTOM_PAD: f32 = 40.0;
const COMPACT_CONTENT_PAD_X: f32 = 20.0;
const COMPACT_CONTENT_PAD_Y: f32 = 18.0;
const COMPACT_CONTENT_BOTTOM_PAD: f32 = 32.0;
const RAIL_SCROLL: &str = "settings-rail-scroll";

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
    transparency: transparency_section::TransparencySliders,
    theme_query: TextEdit,
    theme_filter: appearance_page::ThemeFilter,
    page_bar: Bar,
    rail_bar: Bar,
    theme_bar: Bar,
    choice_bar: Bar,
    notice: notice::NoticeState,
    actions: actions::ActionsState,
    keyboard: keyboard::KeyboardState,
    integrations: integrations::IntegrationsState,
    diagnostics: diagnostics::DiagnosticsState,
}

impl SettingsDialog {
    fn from_preferences(prefs: &Preferences, later: crate::slots::Later<App>) -> Self {
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
        );
        let text_size = NumberFieldState::new(
            f64::from(prefs.text_size.root_font_px()),
            Range::new(f64::from(TextSize::MIN), f64::from(TextSize::MAX), 1.0),
        );
        let roundness = NumberFieldState::new(
            f64::from(prefs.corner_roundness.percent()),
            Range::new(
                f64::from(CornerRoundness::MIN),
                f64::from(CornerRoundness::MAX),
                f64::from(CornerRoundness::STEP),
            ),
        );
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
            transparency: transparency_section::TransparencySliders::new(),
            theme_query: TextEdit::default(),
            theme_filter: appearance_page::ThemeFilter::All,
            page_bar: controls::settings_bar(),
            rail_bar: controls::settings_bar(),
            theme_bar: controls::settings_bar(),
            choice_bar: controls::settings_bar(),
            notice: notice::NoticeState::default(),
            actions: actions::ActionsState::new(later.clone()),
            keyboard: keyboard::KeyboardState::default(),
            integrations: integrations::IntegrationsState::new(later.clone()),
            diagnostics: diagnostics::DiagnosticsState::new(later),
        }
    }

    pub(crate) fn poll_editor_highlight(&mut self) -> bool {
        self.actions.poll_editor_highlight()
    }

    pub(super) fn is_open(&self) -> bool {
        self.open
    }

    /// Shows `page`. The Keyboard page is made the first time it shows, as GTK builds it on its
    /// first selection (`src/ui/settings.rs`), so it opens on the table of the mode in force then.
    /// Returns `true` when the active page changed.
    pub(super) fn select_page(&mut self, page: PageId, tenxer_mode: bool) -> bool {
        let changed = self.page != page;
        self.set_page(page);
        if page == PageId::Keyboard {
            self.keyboard.build_once(tenxer_mode);
        }
        if changed && page == PageId::Integrations {
            self.integrations.mark_needs_scan();
        }
        changed
    }

    pub(super) fn set_page(&mut self, page: PageId) {
        if self.page != page {
            self.page = page;
            self.open_choice = None;
            self.page_bar.release();
            self.theme_bar.release();
            self.choice_bar.release();
            self.keyboard.end_capture();
        }
    }

    fn release_bars(&self) {
        self.page_bar.release();
        self.rail_bar.release();
        self.theme_bar.release();
        self.choice_bar.release();
    }

    fn text_field_focused(&self) -> bool {
        self.search.edit().focused
            || self.theme_query.focused
            || self.workers.edit().focused
            || self.video_start.edit().focused
            || self.text_size.edit().focused
            || self.roundness.edit().focused
            || self.keyboard.query.focused
            || self.actions.has_focused_field()
    }

    /// Whether one of the pages' choice menus is open over Settings, with no message over it.
    fn choice_menu_open(&self) -> bool {
        self.open && !self.message_open() && self.open_choice.is_some()
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
    let transparency = shell::transparency(app);
    let state = &mut app.settings_dialog;
    controls::sync_search_index_if_needed(&mut state.index, &mut state.indexed_locale, avail);
    general::sync_general_fields(&mut state.workers, &mut state.video_start, &prefs);
    appearance_page::sync_appearance_fields(&mut state.text_size, &mut state.roundness, &prefs);
    state.transparency.sync(transparency);
    state.open = true;
    state.open_choice = None;
    state.opened_page = state.page;
    let nav_key = state.page.nav_key();
    let on_actions = state.page == PageId::Actions;
    let on_integrations = state.page == PageId::Integrations;
    if on_integrations {
        state.integrations.mark_needs_scan();
    }
    if on_actions {
        actions::ensure_observing(app);
    }
    if on_integrations {
        integrations::ensure_scanned(app);
    }
    tx.focus_when_shown(nav_key, tx.focus_visible() || tx.by_keys());
}

pub(super) fn close_settings(app: &mut App) {
    let state = &mut app.settings_dialog;
    state.open = false;
    state.open_choice = None;
    state.release_bars();
    state.search = SettingsSearchState::new();
    state.theme_query.replace_text("");
    state.theme_filter = appearance_page::ThemeFilter::All;
    // GTK hides its Settings layer and keeps it: the keybind search and the table shown stay as
    // they were, and only a capture or a confirmation ends with it.
    state.actions.closed();
    state.keyboard.closed();
    state.integrations.closed();
    state.diagnostics.closed();
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
        Self::from_preferences(setup.settings, setup.later.clone())
    }

    fn keyboard(app: &App) -> Option<Holds> {
        let dialog = &app.settings_dialog;
        if !dialog.open && !dialog.notice.is_open() {
            return None;
        }
        if dialog.choice_menu_open() || (dialog.open && dialog.actions.popover_open()) {
            // A choice menu or editor popover is a second modal, which leaves only the popover
            // in the tree. Its keys stay in it, as GTK's popover in the Settings window keeps
            // them from the browser.
            return Some(Holds::Popover);
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
        if app.settings_dialog.page == PageId::Actions
            && !app.settings_dialog.actions.is_observing()
        {
            actions::ensure_observing(app);
        }
        if app.settings_dialog.page == PageId::Integrations {
            integrations::ensure_scanned(app);
        }
        changed |= controls::search_handle().tick(app, clock, tx);
        changed |= general_more::workers_field().poll(app, tx);
        changed |= general::video_start_field().poll(app, tx);
        changed |= appearance_page::text_size_field().poll(app, tx);
        changed |= appearance_page::roundness_field().poll(app, tx);
        changed |= actions::poll(app, tx);
        changed |= keyboard::poll(app, tx);
        changed |= integrations::poll(app, clock, tx);
        changed |= diagnostics::poll(app, clock, tx);
        let prefs = shell::preferences(app).clone();
        let transparency = shell::transparency(app);
        let state = &mut app.settings_dialog;
        general::sync_general_fields(&mut state.workers, &mut state.video_start, &prefs);
        appearance_page::sync_appearance_fields(&mut state.text_size, &mut state.roundness, &prefs);
        state.transparency.sync(transparency);
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
        .grow(1.0)
        .min_h(0.0)
        .child(rail)
        .child(main);
    if notice_open {
        body = body.inert();
    }
    let mut stack = col()
        .key("settings-stack")
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .child(body);
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
        .height(DIALOG_HEIGHT)
        .margin(app.look.space.xxl)
        .fill(app.look.transparency.overlay_color(app.look.kit.c.bg_base))
        .panel_radius(app.look.kit.radius.xl)
        .child(stack);
    if let Some(view) = shell::view_focus(app) {
        shell = shell.return_focus(view);
    }
    shell.build(&app.look, frame.layer);
    frame.layer.extend(menu_layer);
    actions::build_overlays(app, frame.layer);
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
    let rail_bar = &app.settings_dialog.rail_bar;
    let inner = col()
        .w_full()
        .min_w(0.0)
        .px(look.space.lg)
        .pb(look.space.xl)
        .gap(look.space.xs)
        .children(nav_items);
    let viewport = col()
        .key(RAIL_SCROLL)
        .w_full()
        .h_full()
        .min_h(0.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .on_scroll_offset(rail_bar.watch())
        .child(inner);
    let bar_el = rail_bar
        .build::<App>(
            "settings-rail-scrollbar",
            RAIL_SCROLL,
            controls::settings_bar_look(
                look,
                controls::SCROLLBAR_EDGE_INSET,
                controls::SCROLLBAR_END_INSET,
            ),
        )
        .wheel_scrolls(RAIL_SCROLL);
    let nav_list = col()
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .child(rail_bar.area(overlay(viewport, [bar_el])));
    col()
        .key("settings-rail")
        .w(RAIL_WIDTH)
        .h_full()
        .min_h(0.0)
        .pt(look.space.xl)
        .gap(look.space.lg)
        .border_sides([0.0, 1.0, 0.0, 0.0], controls::panel_border(look))
        .child(
            text(tr("SETTINGS"))
                .font(look.ty.heading)
                .color(c.text_muted)
                .px(look.space.lg + 10.0)
                .pt(2.0),
        )
        .child(
            col()
                .w_full()
                .px(look.space.lg)
                .pb(2.0)
                .child(search.field(app, look)),
        )
        .child(nav_list)
}

struct NavColors {
    fill: Color,
    hover: Color,
    icon: Color,
    title: Color,
    subtitle: Color,
}

fn nav_colors(look: &strata_kit::look::Look, active: bool) -> NavColors {
    let kit = &look.kit;
    let c = &kit.c;
    if active {
        NavColors {
            fill: kit.alpha(c.bg_accent, 0.14),
            hover: kit.alpha(c.bg_accent, 0.18),
            icon: c.icon_accent,
            title: c.text_base,
            subtitle: kit.alpha(c.text_base, 0.78),
        }
    } else {
        NavColors {
            fill: Color::TRANSPARENT,
            hover: c.hover,
            icon: kit.alpha(c.icon_accent, 0.65),
            title: c.text_muted,
            subtitle: c.text_muted,
        }
    }
}

fn nav_button(app: &App, entry: &NavEntry) -> El {
    let look = &app.look;
    let kit = &look.kit;
    let colors = nav_colors(look, app.settings_dialog.page == entry.id);
    let page = entry.id;
    let mut btn = row()
        .role("button")
        .label(tr(entry.accessible))
        .w_full()
        .min_h(46.0)
        .px(11.0)
        .py(7.0)
        .gap(10.0)
        .items_center()
        .radius(look.round(8.0))
        .fill(colors.fill)
        .hover(colors.hover)
        .press(kit.c.pressed)
        .focusable(true)
        .focus_ring(2.0, -2.0, kit.c.border_focus)
        .cursor(Cursor::Pointer)
        .child(
            icon(look.icon(entry.icon), 18.0)
                .color(colors.icon)
                .a11y_hidden(),
        )
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(2.0)
                .child(
                    text(tr(entry.label))
                        .font(kit.ty.ui_medium)
                        .color(colors.title)
                        .truncate()
                        .a11y_hidden(),
                )
                .child(
                    text(tr(entry.subtitle))
                        .font(kit.ty.small)
                        .color(colors.subtitle)
                        .truncate()
                        .a11y_hidden(),
                ),
        )
        .on_press_tx(move |app: &mut App, tx: &mut Tx| {
            let tenxer_mode = shell::preferences(app).tenxer_mode;
            if app.settings_dialog.select_page(page, tenxer_mode) {
                tx.scroll_to_instant(controls::PAGE_SCROLL, 0.0, 0.0);
            }
            if page == PageId::Actions {
                actions::ensure_observing(app);
            }
            if page == PageId::Integrations {
                integrations::ensure_scanned(app);
            }
        });
    if entry.id == app.settings_dialog.opened_page {
        btn = btn.autofocus();
    }
    buttons::on_return(entry.id.nav_key(), btn)
}

fn close_button(look: &strata_kit::look::Look) -> El {
    let kit = &look.kit;
    let c = &kit.c;
    buttons::on_return(
        "settings-close",
        el().role("button")
            .label(tr("Close settings"))
            .size(28.0, 28.0)
            .center()
            .radius(kit.radius.sm)
            .fill(Color::TRANSPARENT)
            .hover(c.hover)
            .press(c.pressed)
            .focusable(true)
            .focus_ring(2.0, -2.0, c.border_focus)
            .cursor(Cursor::Pointer)
            .child(
                icon(look.icon(IconName::Close), 18.0)
                    .color(kit.alpha(c.icon_accent, 0.65))
                    .a11y_hidden(),
            )
            .on_press_tx(|app: &mut App, _: &mut Tx| close_settings(app)),
    )
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
    let topbar = row()
        .w_full()
        .pl(24.0)
        .pr(12.0)
        .py(12.0)
        .gap(look.space.md)
        .items_center()
        .border_sides([0.0, 0.0, 1.0, 0.0], controls::panel_border(look))
        .child(
            text(heading)
                .font(look.ty.title)
                .color(c.text_base)
                .grow(1.0),
        )
        .child(close_button(look));
    let (pad_x, pad_y, bottom_pad) = content_padding(app);
    let body = if app.settings_dialog.search.no_results() {
        col()
            .w_full()
            .grow(1.0)
            .px(pad_x)
            .pt(pad_y)
            .pb(bottom_pad)
            .child(search.empty(look))
    } else {
        let content = build_page_content(app, page, search, menu_layer);
        build_page_scroll(app, content, (pad_x, pad_y, bottom_pad))
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

fn build_page_scroll(
    app: &App,
    content: Vec<El>,
    (pad_x, pad_y, bottom_pad): (f32, f32, f32),
) -> El {
    let look = &app.look;
    let page_bar = &app.settings_dialog.page_bar;
    let inner = col()
        .w_full()
        .min_w(0.0)
        .px(pad_x)
        .pt(pad_y)
        .pb(bottom_pad)
        .children(content);
    let viewport = col()
        .key(controls::PAGE_SCROLL)
        .w_full()
        .h_full()
        .min_h(0.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .on_scroll_offset(page_bar.watch())
        .child(inner);
    let bar_el = page_bar
        .build::<App>(
            "settings-page-scrollbar",
            controls::PAGE_SCROLL,
            controls::settings_bar_look(
                look,
                controls::SCROLLBAR_EDGE_INSET,
                controls::SCROLLBAR_END_INSET,
            ),
        )
        .wheel_scrolls(controls::PAGE_SCROLL);
    col()
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .child(page_bar.area(overlay(viewport, [bar_el])))
}

fn content_padding(app: &App) -> (f32, f32, f32) {
    if app
        .window_size()
        .is_some_and(|(w, _)| w < COMPACT_CONTENT_WINDOW_WIDTH)
    {
        (
            COMPACT_CONTENT_PAD_X,
            COMPACT_CONTENT_PAD_Y,
            COMPACT_CONTENT_BOTTOM_PAD,
        )
    } else {
        (CONTENT_PAD_X, CONTENT_PAD_Y, CONTENT_BOTTOM_PAD)
    }
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
        PageId::Actions => actions::build(app, search),
        PageId::Keyboard => keyboard::build(app, search),
        PageId::Integrations => integrations::build(app, search),
        PageId::Updates => pages::updates_page(app, search, menu_layer),
        PageId::Diagnostics => diagnostics::build(app, search),
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
