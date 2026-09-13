// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    env,
    path::PathBuf,
    rc::Rc,
    time::{Duration, Instant},
};

use gtk::{gio, glib, prelude::*};

use crate::{
    adapters::{LocalFileSource, LocalOperationProvider, RevealRequest, location_for_file},
    app::{Browser, BrowserEvent},
    model::Location,
    services::{BuildKind, ReleaseMetadata, sanitize_uri_credentials},
};

use super::{
    browser::{
        BrowserView, PeekBehavior, PinStatus, PreparedFileDrop, WeakBrowserView, file_drop_action,
        file_drop_commit, locations_from_file_list_value, prepare_file_drop_target,
        show_error_dialog,
    },
    browser_modes::{BrowserDensity, BrowserMode},
    motion::{animations_enabled, emphasized_deceleration},
    theme::ThemeManager,
};

mod composition;
mod devices;
mod keyboard;
mod open_argument;
mod sidebar;

pub use open_argument::present_open;

use sidebar::PlaceNavigation;
pub(super) use sidebar::build_sidebar;

pub(super) const SIDEBAR_WIDTH: i32 = 208;
pub(super) const MIN_SIDEBAR_WIDTH: i32 = 176;
const SIDEBAR_TRANSITION: Duration = Duration::from_millis(300);
const PINNED_DRAG_PREFIX: &str = "pinned:";
const STANDARD_PLACE_IDS: &[&str] = &["desktop", "documents", "downloads", "pictures", "videos"];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MouseHistoryAction {
    Back,
    Forward,
}

#[derive(Clone)]
struct TypeToSearch {
    view: BrowserView,
    preferences: Rc<ThemeManager>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TypeToSearchQuery {
    Empty,
    Character(char),
}

impl TypeToSearch {
    fn show(&self, query: TypeToSearchQuery) -> bool {
        self.preferences.type_to_search()
            && match query {
                TypeToSearchQuery::Empty => self.view.show_filter(),
                TypeToSearchQuery::Character(character) => {
                    self.view.show_filter_with_query(&character.to_string())
                }
            }
    }
}

fn mouse_history_action(button: u32) -> Option<MouseHistoryAction> {
    match button {
        8 => Some(MouseHistoryAction::Back),
        9 => Some(MouseHistoryAction::Forward),
        _ => None,
    }
}

pub fn present(application: &gtk::Application) {
    present_target(application, None, Vec::new(), false, true);
}

/// Opens the requested directory with the named items selected.
pub fn present_reveal(application: &gtk::Application, request: RevealRequest) {
    present_target(
        application,
        Some(request.directory),
        request.selection,
        request.properties,
        true,
    );
}

fn bind_update_notice_preferences(
    anchor: &impl IsA<gtk::Widget>,
    manager: &ThemeManager,
    notice: &super::settings::UpdateNoticeHandler,
) {
    let initial = Cell::new(true);
    let notice = Rc::downgrade(notice);
    manager.bind_preference(
        anchor,
        |manager| (manager.checks_for_updates(), manager.release_channel()),
        move |_, _| {
            if !initial.replace(false)
                && let Some(notice) = notice.upgrade()
            {
                notice(None);
            }
        },
    );
}

fn browser_for_window() -> BrowserView {
    let browser = BrowserView::new(Rc::new(LocalFileSource), PeekBehavior::default());
    browser.set_operation_provider(Rc::new(LocalOperationProvider));
    browser
}

pub(super) fn present_target(
    application: &gtk::Application,
    location: Option<Location>,
    selection: Vec<String>,
    properties: bool,
    auto_navigate: bool,
) -> BrowserView {
    let present_started = std::time::Instant::now();
    crate::assets::register_icon_theme();
    let theme_manager = super::theme::ThemeManager::shared();
    load_styles();
    tracing::debug!(
        elapsed_ms = present_started.elapsed().as_millis() as u64,
        "present theme and styles ready"
    );

    let window = gtk::ApplicationWindow::builder()
        .application(application)
        .title("Strata")
        .default_width(1200)
        .default_height(760)
        .build();

    let content = composition::WindowContent::new(&window, &theme_manager);
    let update_notice = content.bind(&window, &theme_manager);
    let browser = content.browser.clone();
    browser.connect_navigation_cleanup(window.upcast_ref());
    schedule_after_first_paint(&window, &content.sidebar);
    content.connect_cleanup(&window);
    window.present();
    crate::metrics::mark_window_presented();
    if auto_navigate {
        let pending_location = location.unwrap_or_else(|| startup_location(&theme_manager));
        if !selection.is_empty() {
            browser.select_after_load(selection, properties);
        }
        let idle_browser = browser.clone();
        glib::idle_add_local_once(move || {
            let started = std::time::Instant::now();
            idle_browser.navigate_location(pending_location);
            tracing::debug!(
                elapsed_ms = started.elapsed().as_millis() as u64,
                "present navigation started"
            );
        });
    }
    super::portal_preferences::schedule_offer(&window);
    schedule_due_update_check(&theme_manager, &update_notice);
    browser
}

fn schedule_after_first_paint(window: &gtk::ApplicationWindow, sidebar: &SidebarView) {
    let state = sidebar.state.clone();
    let armed = Cell::new(false);
    window.connect_map(move |window| {
        if armed.get() {
            return;
        }
        let Some(clock) = window.frame_clock() else {
            return;
        };
        armed.set(true);
        let handler = Rc::new(RefCell::new(None));
        let handler_for_paint = handler.clone();
        let state = state.clone();
        let id = clock.connect_after_paint(move |clock| {
            if let Some(id) = handler_for_paint.borrow_mut().take() {
                clock.disconnect(id);
            }
            crate::metrics::mark_first_themed_frame();
            let state = state.clone();
            glib::idle_add_local_once(move || state.rebuild());
        });
        handler.replace(Some(id));
    });
}

fn schedule_due_update_check(
    manager: &Rc<ThemeManager>,
    notice: &super::settings::UpdateNoticeHandler,
) {
    let manager = manager.clone();
    let notice = notice.clone();
    glib::timeout_add_local_once(std::time::Duration::from_secs(8), move || {
        glib::idle_add_local_once(move || {
            super::settings::maybe_run_due_update_check(&manager, &notice);
        });
    });
}
pub(super) fn bind_sidebar_text_size(paned: &gtk::Paned) {
    ThemeManager::shared().bind_interface_scale(paned, |widget, scale| {
        let paned = widget.downcast_ref::<gtk::Paned>().expect("sidebar split");
        if paned.position() > 0 {
            paned.set_position(scaled_sidebar_width(paned, scale));
        }
    });
}

fn scaled_sidebar_width(paned: &gtk::Paned, scale: f64) -> i32 {
    let preferred = (f64::from(SIDEBAR_WIDTH) * scale).round() as i32;
    let available = if paned.width() > 0 {
        paned.width() / 2
    } else {
        preferred
    };
    preferred.min(available).max(MIN_SIDEBAR_WIDTH)
}

fn animate_sidebar(
    paned: &gtk::Paned,
    sidebar: &gtk::Widget,
    generation: &Rc<Cell<u64>>,
    animating: &Rc<Cell<bool>>,
    expanded: bool,
) {
    let animation_id = generation.get().saturating_add(1);
    generation.set(animation_id);
    animating.set(true);
    paned.set_shrink_start_child(true);
    let target = if expanded {
        scaled_sidebar_width(paned, ThemeManager::shared().interface_scale())
    } else {
        0
    };
    let start = paned.position();
    if expanded {
        sidebar.set_visible(true);
    }

    if !animations_enabled() || start == target {
        paned.set_position(target);
        sidebar.set_visible(expanded);
        paned.set_shrink_start_child(!expanded);
        animating.set(false);
        return;
    }

    let started = Instant::now();
    let paned = paned.clone();
    let sidebar = sidebar.clone();
    let generation = generation.clone();
    let animating = animating.clone();
    let _tick = paned.clone().add_tick_callback(move |_, _| {
        if generation.get() != animation_id {
            return glib::ControlFlow::Break;
        }

        let progress =
            (started.elapsed().as_secs_f64() / SIDEBAR_TRANSITION.as_secs_f64()).clamp(0.0, 1.0);
        let eased = emphasized_deceleration(progress);
        let position = f64::from(start) + f64::from(target - start) * eased;
        paned.set_position(position.round() as i32);

        if progress >= 1.0 {
            paned.set_position(target);
            if !expanded {
                sidebar.set_visible(false);
            }
            paned.set_shrink_start_child(!expanded);
            animating.set(false);
            glib::ControlFlow::Break
        } else {
            glib::ControlFlow::Continue
        }
    });
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SinglePaneArrow {
    Native,
    Stay,
    Sidebar,
}

fn single_pane_arrow_action(
    mode: BrowserMode,
    key: gtk::gdk::Key,
    modifiers: gtk::gdk::ModifierType,
    at_left_edge: bool,
    sidebar_visible: bool,
) -> Option<SinglePaneArrow> {
    use gtk::gdk::{Key, ModifierType};
    if mode == BrowserMode::Columns
        || modifiers.intersects(ModifierType::ALT_MASK | ModifierType::SUPER_MASK)
    {
        return None;
    }
    let plain = !modifiers.intersects(ModifierType::CONTROL_MASK | ModifierType::SHIFT_MASK);
    match key {
        Key::Left if plain && at_left_edge && sidebar_visible => Some(SinglePaneArrow::Sidebar),
        Key::Left | Key::Right if mode == BrowserMode::List => Some(SinglePaneArrow::Stay),
        Key::Left | Key::Right | Key::Up | Key::Down => Some(SinglePaneArrow::Native),
        _ => None,
    }
}

pub(super) fn is_browser_navigation_key(
    key: gtk::gdk::Key,
    modifiers: gtk::gdk::ModifierType,
) -> bool {
    if modifiers
        .intersects(gtk::gdk::ModifierType::CONTROL_MASK | gtk::gdk::ModifierType::SUPER_MASK)
    {
        return false;
    }
    matches!(
        key,
        gtk::gdk::Key::j
            | gtk::gdk::Key::k
            | gtk::gdk::Key::h
            | gtk::gdk::Key::l
            | gtk::gdk::Key::Up
            | gtk::gdk::Key::Down
            | gtk::gdk::Key::Left
            | gtk::gdk::Key::Right
            | gtk::gdk::Key::Home
            | gtk::gdk::Key::End
            | gtk::gdk::Key::Page_Up
            | gtk::gdk::Key::Page_Down
            | gtk::gdk::Key::KP_Page_Up
            | gtk::gdk::Key::KP_Page_Down
            | gtk::gdk::Key::Tab
            | gtk::gdk::Key::ISO_Left_Tab
            | gtk::gdk::Key::Return
            | gtk::gdk::Key::KP_Enter
            | gtk::gdk::Key::BackSpace
    )
}

fn page_direction(key: gtk::gdk::Key) -> Option<i32> {
    match key {
        gtk::gdk::Key::Page_Up | gtk::gdk::Key::KP_Page_Up => Some(-1),
        gtk::gdk::Key::Page_Down | gtk::gdk::Key::KP_Page_Down => Some(1),
        _ => None,
    }
}

fn jump_direction(key: gtk::gdk::Key, modifiers: gtk::gdk::ModifierType) -> Option<i32> {
    use gtk::gdk::{Key, ModifierType};
    if !modifiers.contains(ModifierType::CONTROL_MASK)
        || modifiers.intersects(
            ModifierType::SHIFT_MASK | ModifierType::ALT_MASK | ModifierType::SUPER_MASK,
        )
    {
        return None;
    }
    match key {
        Key::Up => Some(-1),
        Key::Down => Some(1),
        _ => None,
    }
}

fn is_undo_shortcut(key: gtk::gdk::Key, modifiers: gtk::gdk::ModifierType) -> bool {
    modifiers.contains(gtk::gdk::ModifierType::CONTROL_MASK)
        && !modifiers
            .intersects(gtk::gdk::ModifierType::SHIFT_MASK | gtk::gdk::ModifierType::ALT_MASK)
        && matches!(key, gtk::gdk::Key::z | gtk::gdk::Key::Z)
}

fn type_to_search_query(
    key: gtk::gdk::Key,
    modifiers: gtk::gdk::ModifierType,
) -> Option<TypeToSearchQuery> {
    // Space belongs to quick preview; focused text fields handle their own spaces.
    if key == gtk::gdk::Key::space
        || modifiers.intersects(
            gtk::gdk::ModifierType::CONTROL_MASK
                | gtk::gdk::ModifierType::ALT_MASK
                | gtk::gdk::ModifierType::SUPER_MASK,
        )
    {
        return None;
    }
    if key == gtk::gdk::Key::slash {
        return Some(TypeToSearchQuery::Empty);
    }
    key.to_unicode()
        .filter(|character| !character.is_control())
        .map(TypeToSearchQuery::Character)
}

fn is_open_terminal_shortcut(key: gtk::gdk::Key, modifiers: gtk::gdk::ModifierType) -> bool {
    modifiers.contains(gtk::gdk::ModifierType::CONTROL_MASK)
        && !modifiers
            .intersects(gtk::gdk::ModifierType::SHIFT_MASK | gtk::gdk::ModifierType::ALT_MASK)
        && matches!(key, gtk::gdk::Key::t | gtk::gdk::Key::T)
}

fn is_toggle_hidden_shortcut(key: gtk::gdk::Key, modifiers: gtk::gdk::ModifierType) -> bool {
    modifiers.contains(gtk::gdk::ModifierType::CONTROL_MASK)
        && !modifiers
            .intersects(gtk::gdk::ModifierType::SHIFT_MASK | gtk::gdk::ModifierType::ALT_MASK)
        && matches!(
            key,
            gtk::gdk::Key::h | gtk::gdk::Key::H | gtk::gdk::Key::period
        )
}

const DEFAULT_ACCELS: &[(&str, &[&str])] = &[
    ("win.search", &["<Control>k"]),
    ("win.open-terminal", &["<Primary>t"]),
    ("win.refresh", &["F5"]),
];

fn is_refresh_shortcut(key: gtk::gdk::Key) -> bool {
    key == gtk::gdk::Key::F5
}

const RENAME_EXCLUDED_MODIFIERS: gtk::gdk::ModifierType = gtk::gdk::ModifierType::ALT_MASK
    .union(gtk::gdk::ModifierType::SHIFT_MASK)
    .union(gtk::gdk::ModifierType::SUPER_MASK);

pub(super) fn is_rename_shortcut(key: gtk::gdk::Key, modifiers: gtk::gdk::ModifierType) -> bool {
    if modifiers.intersects(RENAME_EXCLUDED_MODIFIERS) {
        return false;
    }
    let control = modifiers.contains(gtk::gdk::ModifierType::CONTROL_MASK);
    match key {
        gtk::gdk::Key::F2 => !control,
        gtk::gdk::Key::r | gtk::gdk::Key::R => control,
        _ => false,
    }
}

pub(super) fn is_sidebar_focus_shortcut(
    key: gtk::gdk::Key,
    modifiers: gtk::gdk::ModifierType,
) -> bool {
    modifiers.contains(gtk::gdk::ModifierType::CONTROL_MASK | gtk::gdk::ModifierType::SHIFT_MASK)
        && !modifiers.contains(gtk::gdk::ModifierType::ALT_MASK)
        && matches!(key, gtk::gdk::Key::b | gtk::gdk::Key::B)
}

pub(super) fn is_context_menu_shortcut(
    key: gtk::gdk::Key,
    modifiers: gtk::gdk::ModifierType,
) -> bool {
    let modifiers = modifiers
        & (gtk::gdk::ModifierType::SHIFT_MASK
            | gtk::gdk::ModifierType::CONTROL_MASK
            | gtk::gdk::ModifierType::ALT_MASK
            | gtk::gdk::ModifierType::SUPER_MASK
            | gtk::gdk::ModifierType::HYPER_MASK
            | gtk::gdk::ModifierType::META_MASK);
    match key {
        gtk::gdk::Key::Menu => modifiers.is_empty(),
        gtk::gdk::Key::F10 => modifiers == gtk::gdk::ModifierType::SHIFT_MASK,
        _ => false,
    }
}

fn sidebar_focus_direction(key: gtk::gdk::Key) -> Option<gtk::DirectionType> {
    match key {
        gtk::gdk::Key::Left => Some(gtk::DirectionType::Left),
        gtk::gdk::Key::Right => Some(gtk::DirectionType::Right),
        gtk::gdk::Key::Up => Some(gtk::DirectionType::Up),
        gtk::gdk::Key::Down => Some(gtk::DirectionType::Down),
        _ => vim_focus_direction(key),
    }
}

pub(super) fn vim_focus_direction(key: gtk::gdk::Key) -> Option<gtk::DirectionType> {
    match key {
        gtk::gdk::Key::h => Some(gtk::DirectionType::Left),
        gtk::gdk::Key::j => Some(gtk::DirectionType::Down),
        gtk::gdk::Key::k => Some(gtk::DirectionType::Up),
        gtk::gdk::Key::l => Some(gtk::DirectionType::Right),
        _ => None,
    }
}

pub(super) fn visible_modal_layer(window: &impl IsA<gtk::Window>) -> Option<gtk::Widget> {
    let overlay = window.child().and_downcast::<gtk::Overlay>()?;
    let mut child = overlay.first_child();
    let mut topmost = None;
    while let Some(widget) = child {
        child = widget.next_sibling();
        if widget.is_visible() && widget.has_css_class("app-modal-layer") {
            topmost = Some(widget);
        }
    }
    topmost
}

pub(super) fn install_modal_focus_trap(window: &impl IsA<gtk::Window>) {
    window.connect_focus_widget_notify(|window| {
        let Some(layer) = visible_modal_layer(window) else {
            return;
        };
        let focus_is_inside = gtk::prelude::RootExt::focus(window.as_ref())
            .is_some_and(|focus| focus == layer || focus.is_ancestor(&layer));
        if !focus_is_inside {
            layer.grab_focus();
        }
    });
}

pub(super) fn apply_browser_mode(
    view: &BrowserView,
    preferences: &super::theme::ThemeManager,
    mode: BrowserMode,
) {
    view.set_view_mode(mode);
    preferences.set_browser_mode(mode);
}

pub(super) fn browser_mode_for_digit(key: gtk::gdk::Key) -> Option<BrowserMode> {
    match key {
        gtk::gdk::Key::_1 | gtk::gdk::Key::KP_1 => Some(BrowserMode::Columns),
        gtk::gdk::Key::_2 | gtk::gdk::Key::KP_2 => Some(BrowserMode::Icons),
        gtk::gdk::Key::_3 | gtk::gdk::Key::KP_3 => Some(BrowserMode::List),
        _ => None,
    }
}

pub(super) fn build_appearance_menu(
    view: &BrowserView,
    controller: &Rc<Browser>,
    preferences: Rc<super::theme::ThemeManager>,
    preview: &super::preview::PreviewDrawer,
) -> gtk::MenuButton {
    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.add_css_class("appearance-menu");
    let popover = gtk::Popover::builder()
        .has_arrow(false)
        .halign(gtk::Align::End)
        .position(gtk::PositionType::Bottom)
        .build();
    popover.add_css_class("appearance-popover");
    super::scrolling::popover::dismiss_on_outside_scroll(&popover);
    let button = gtk::MenuButton::builder()
        .tooltip_text("Appearance")
        .popover(&popover)
        .build();
    // Without an explicit name GTK builds one from the whole open popover, so
    // a screen reader reads the entire menu back as the button's label.
    super::accessibility::set_label(&button, "Appearance");
    let popover_weak = popover.downgrade();
    append_menu_heading(&content, "VIEW");
    let current_mode = view.view_mode();
    let button_icon = crate::assets::chrome_icon(browser_mode_icon(current_mode));
    let (columns, columns_check, _) = appearance_option(
        crate::assets::icons::COLUMNS,
        "Columns",
        current_mode == BrowserMode::Columns,
        true,
    );
    let (icons, icons_check, _) = appearance_option(
        crate::assets::icons::ICONS,
        "Icons",
        current_mode == BrowserMode::Icons,
        true,
    );
    let (list, list_check, _) = appearance_option(
        crate::assets::icons::LIST,
        "List",
        current_mode == BrowserMode::List,
        true,
    );
    let grouped = preferences.group_by_type();
    let (group_by_type, group_check, _) = appearance_option(
        crate::assets::icons::LIST_CHECKS,
        "Group by file type",
        grouped,
        current_mode.supports_type_grouping(),
    );
    group_by_type.set_tooltip_text(Some("Group List entries under file-type headings"));
    preferences.bind_preference(
        &group_check,
        ThemeManager::group_by_type,
        |widget, enabled| widget.set_visible(enabled),
    );
    {
        let preferences = preferences.clone();
        let popover_weak = popover_weak.clone();
        group_by_type.connect_clicked(move |_| {
            preferences.set_group_by_type(!preferences.group_by_type());
            if let Some(popover) = popover_weak.upgrade() {
                popover.popdown();
            }
        });
    }
    for (button, mode) in [
        (&columns, BrowserMode::Columns),
        (&icons, BrowserMode::Icons),
        (&list, BrowserMode::List),
    ] {
        let view = view.clone();
        let preferences = preferences.clone();
        let popover_weak = popover_weak.clone();
        button.connect_clicked(move |_| {
            apply_browser_mode(&view, &preferences, mode);
            if let Some(popover) = popover_weak.upgrade() {
                popover.popdown();
            }
        });
    }
    {
        // The mode also changes from the keyboard, so track the view rather
        // than only the buttons in this menu.
        let columns_check = columns_check.clone();
        let icons_check = icons_check.clone();
        let list_check = list_check.clone();
        let group_by_type = group_by_type.clone();
        let button_icon = button_icon.clone();
        view.connect_view_mode_changed(move |mode| {
            columns_check.set_visible(mode == BrowserMode::Columns);
            icons_check.set_visible(mode == BrowserMode::Icons);
            list_check.set_visible(mode == BrowserMode::List);
            group_by_type.set_sensitive(mode.supports_type_grouping());
            crate::assets::set_primary_icon(&button_icon, browser_mode_icon(mode));
        });
    }
    content.append(&columns);
    content.append(&icons);
    content.append(&list);
    let (row, check, _) = appearance_row(
        crate::assets::icons::EYE,
        "Preview panel",
        "Space",
        preview.is_enabled(),
    );
    let preview_toggle = gtk::ToggleButton::builder()
        .child(&row)
        .has_frame(false)
        .build();
    preview_toggle.add_css_class("appearance-option");
    preview_toggle.add_css_class("preview-panel-option");
    super::accessibility::set_label(&preview_toggle, "Preview panel");
    preview_toggle.set_tooltip_text(Some("Toggle preview panel while browsing (Space)"));
    let actions = gio::SimpleActionGroup::new();
    actions.add_action(&preview.action());
    preview_toggle.insert_action_group("preview", Some(&actions));
    preview_toggle.set_action_name(Some("preview.preview-panel"));
    preview_toggle
        .bind_property("active", &check, "visible")
        .sync_create()
        .build();
    let closing = popover_weak.clone();
    preview_toggle.connect_clicked(move |_| {
        if let Some(popover) = closing.upgrade() {
            popover.popdown();
        }
    });
    content.append(&preview_toggle);

    content.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    append_menu_heading(&content, "DENSITY");
    let current_density = preferences.browser_density();
    let hidden_files_shown = preferences.sort_preferences().show_hidden;
    let (compact, compact_check, _) = appearance_option(
        crate::assets::icons::ROWS,
        "Compact",
        current_density == BrowserDensity::Compact,
        true,
    );
    let (airy, airy_check, _) = appearance_option(
        crate::assets::icons::ROWS,
        "Airy",
        current_density == BrowserDensity::Airy,
        true,
    );
    preferences.bind_preference(
        &compact_check,
        ThemeManager::browser_density,
        |widget, density| widget.set_visible(density == BrowserDensity::Compact),
    );
    preferences.bind_preference(
        &airy_check,
        ThemeManager::browser_density,
        |widget, density| widget.set_visible(density == BrowserDensity::Airy),
    );
    {
        let preferences = preferences.clone();
        let popover_weak = popover_weak.clone();
        compact.connect_clicked(move |_| {
            preferences.set_browser_density(BrowserDensity::Compact);
            if let Some(popover) = popover_weak.upgrade() {
                popover.popdown();
            }
        });
    }
    {
        let preferences = preferences.clone();
        let popover_weak = popover_weak.clone();
        airy.connect_clicked(move |_| {
            preferences.set_browser_density(BrowserDensity::Airy);
            if let Some(popover) = popover_weak.upgrade() {
                popover.popdown();
            }
        });
    }
    content.append(&compact);
    content.append(&airy);

    content.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    append_menu_heading(&content, "TEXT SIZE");
    let text_controls = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    for (label, tooltip, delta) in [
        ("−", "Decrease text size (Ctrl+−)", -1),
        ("+", "Increase text size (Ctrl++)", 1),
    ] {
        let button = gtk::Button::with_label(label);
        button.add_css_class("appearance-option");
        button.set_tooltip_text(Some(tooltip));
        super::accessibility::set_label(&button, tooltip);
        let manager = preferences.clone();
        button.connect_clicked(move |_| manager.set_text_size(manager.text_size().stepped(delta)));
        text_controls.append(&button);
    }
    let reset_size = gtk::Button::new();
    reset_size.add_css_class("appearance-option");
    reset_size.set_hexpand(true);
    reset_size.set_tooltip_text(Some("Reset text size (Ctrl+0)"));
    preferences.bind_preference(&reset_size, ThemeManager::text_size, |widget, size| {
        widget
            .downcast_ref::<gtk::Button>()
            .expect("text size reset")
            .set_label(&format!("{} px", size.root_font_px()));
    });
    let manager = preferences.clone();
    reset_size.connect_clicked(move |_| manager.set_text_size(super::theme::TextSize::default()));
    text_controls.append(&reset_size);
    content.append(&text_controls);

    content.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    content.append(&group_by_type);
    let (hidden, hidden_check, hidden_icon) = appearance_option_with_shortcut(
        if hidden_files_shown {
            crate::assets::icons::EYE
        } else {
            crate::assets::icons::EYE_OFF
        },
        "Hidden files",
        "Ctrl + H",
        hidden_files_shown,
        true,
    );
    let observed_hidden_check = hidden_check.clone();
    let observed_hidden_icon = hidden_icon.clone();
    controller.observe_preferences(move |preferences| {
        observed_hidden_check.set_visible(preferences.show_hidden);
        crate::assets::set_primary_icon(
            &observed_hidden_icon,
            if preferences.show_hidden {
                crate::assets::icons::EYE
            } else {
                crate::assets::icons::EYE_OFF
            },
        );
    });
    let weak_controller = Rc::downgrade(controller);
    let popover_weak = popover_weak.clone();
    hidden.connect_clicked(move |_| {
        if let Some(controller) = weak_controller.upgrade() {
            controller.toggle_hidden();
        }
        if let Some(popover) = popover_weak.upgrade() {
            popover.popdown();
        }
    });
    content.append(&hidden);

    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .propagate_natural_width(true)
        .propagate_natural_height(true)
        .max_content_height(600)
        .child(&content)
        .build();
    let anchor = button.downgrade();
    let constrained = scroll.downgrade();
    popover.connect_show(move |_| {
        if let (Some(anchor), Some(scroll)) = (anchor.upgrade(), constrained.upgrade())
            && let Some(window) = anchor.root().and_downcast::<gtk::Window>()
        {
            scroll.set_max_content_height((window.height() - anchor.height() - 48).max(1));
            scroll.set_max_content_width((window.width() - 24).max(1));
        }
    });
    popover.set_child(Some(&scroll));
    button.set_child(Some(&button_icon));
    button.add_css_class("header-action");
    button.set_cursor_from_name(Some("pointer"));
    button
}

fn browser_mode_icon(mode: BrowserMode) -> &'static str {
    match mode {
        BrowserMode::Columns => crate::assets::icons::COLUMNS,
        BrowserMode::Icons => crate::assets::icons::ICONS,
        BrowserMode::List => crate::assets::icons::LIST,
    }
}

fn appearance_option(
    icon: &str,
    label: &str,
    checked: bool,
    sensitive: bool,
) -> (gtk::Button, gtk::Image, gtk::Image) {
    appearance_option_with_shortcut(icon, label, "", checked, sensitive)
}

fn appearance_option_with_shortcut(
    icon: &str,
    label: &str,
    shortcut: &str,
    checked: bool,
    sensitive: bool,
) -> (gtk::Button, gtk::Image, gtk::Image) {
    let (row, check, option) = appearance_row(icon, label, shortcut, checked);
    let button = gtk::Button::builder()
        .child(&row)
        .sensitive(sensitive)
        .build();
    button.add_css_class("appearance-option");
    button.set_has_frame(false);
    (button, check, option)
}

fn appearance_row(
    icon: &str,
    label: &str,
    shortcut: &str,
    checked: bool,
) -> (gtk::Box, gtk::Image, gtk::Image) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    let check = crate::assets::primary_icon(crate::assets::icons::CHECK, 16);
    check.set_visible(checked);
    let option = crate::assets::primary_icon(icon, 17);
    let label = gtk::Label::new(Some(label));
    label.set_xalign(0.0);
    label.set_hexpand(true);
    row.append(&option);
    row.append(&label);
    if !shortcut.is_empty() {
        let shortcut = gtk::Label::new(Some(shortcut));
        shortcut.add_css_class("folder-context-shortcut");
        row.append(&shortcut);
    }
    row.append(&check);
    (row, check, option)
}

fn append_menu_heading(container: &gtk::Box, text: &str) {
    let heading = gtk::Label::new(Some(text));
    heading.set_xalign(0.0);
    heading.add_css_class("menu-heading");
    container.append(&heading);
}

pub(super) struct SidebarState {
    widget: gtk::Box,
    view: BrowserView,
    browser: Rc<Browser>,
    volume_monitor: gio::VolumeMonitor,
    mount_monitor: gio_unix::MountMonitor,
    theme_manager: Rc<super::theme::ThemeManager>,
    place_order: RefCell<Vec<&'static str>>,
    pinned_places: Rc<RefCell<Vec<(Location, String)>>>,
    place_rows: RefCell<Vec<(Location, gtk::Button)>>,
    trash_contents: Cell<TrashContents>,
    trash_menu_rows: RefCell<Option<TrashMenuRows>>,
    trash_monitor: RefCell<Option<gio::FileMonitor>>,
    trash_probe_running: Cell<bool>,
    trash_probe_pending: Cell<bool>,
    local_only: bool,
}

/// Rows of the Trash sidebar context menu that only make sense while Trash holds items.
struct TrashMenuRows {
    separator: gtk::Separator,
    empty: gtk::Button,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TrashContents {
    /// Not probed yet, or the probe failed.
    Unknown,
    Empty,
    NonEmpty,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TrashMenuVisibility {
    separator: bool,
    empty: bool,
}

/// Destructive actions stay hidden until Trash is confirmed to hold something, and the
/// separator goes with them so Properties is not left above an empty gap.
fn trash_menu_visibility(contents: TrashContents) -> TrashMenuVisibility {
    let visible = matches!(contents, TrashContents::NonEmpty);
    TrashMenuVisibility {
        separator: visible,
        empty: visible,
    }
}

fn sync_trash_menu_rows(rows: &TrashMenuRows, contents: TrashContents) {
    let visibility = trash_menu_visibility(contents);
    rows.separator.set_visible(visibility.separator);
    rows.empty.set_visible(visibility.empty);
}

fn trash_contents_from_probe(probe: Result<bool, glib::Error>) -> TrashContents {
    match probe {
        Ok(true) => TrashContents::NonEmpty,
        Ok(false) => TrashContents::Empty,
        Err(_) => TrashContents::Unknown,
    }
}

fn event_changes_trash_contents(event: &BrowserEvent) -> bool {
    matches!(
        event,
        BrowserEvent::DeletionFinished { .. }
            | BrowserEvent::RestorationFinished
            | BrowserEvent::TransferFinished { .. }
            | BrowserEvent::OperationCompletedWithErrors { .. }
            | BrowserEvent::OperationCancelled { .. }
    )
}

pub(super) struct SidebarView {
    pub(super) widget: gtk::Widget,
    pub(super) state: Rc<SidebarState>,
    update_notice: gtk::Button,
    update_area: gtk::Box,
    update_label: gtk::Label,
    handlers: RefCell<Vec<glib::SignalHandlerId>>,
    mount_handler: RefCell<Option<glib::SignalHandlerId>>,
}

impl SidebarView {
    pub(super) fn disconnect(&self) {
        for handler in self.handlers.take() {
            self.state.volume_monitor.disconnect(handler);
        }
        if let Some(handler) = self.mount_handler.take() {
            self.state.mount_monitor.disconnect(handler);
        }
    }
}

impl SidebarState {
    fn rebuild(self: &Rc<Self>) {
        while let Some(child) = self.widget.first_child() {
            self.widget.remove(&child);
        }
        self.place_rows.borrow_mut().clear();

        self.append_static_places();
        self.append_devices();
        self.sync_active_place();
    }

    fn append_static_places(self: &Rc<Self>) {
        self.append_place(
            crate::assets::icons::HOME,
            "Home",
            Location::local(home_directory()),
        );
        if !self.local_only {
            self.append_trash_place();
            self.append_place(
                crate::assets::icons::NETWORK,
                "Network",
                Location::uri("network:///"),
            );
        }
        self.append_separator();
        self.append_standard_places();
        self.append_pinned_places();
    }

    fn append_standard_places(self: &Rc<Self>) {
        for place in self.place_order.borrow().clone() {
            if let Some((icon, name, directory)) = standard_place(place)
                && let Some(path) = glib::user_special_dir(directory)
                    .filter(|path| should_show_standard_place(place, path, &home_directory()))
            {
                if self.local_only {
                    self.append_place(icon, name, Location::local(path));
                } else {
                    self.append_reorderable_place(place, icon, name, Location::local(path));
                }
            }
        }
    }

    fn append_pinned_places(self: &Rc<Self>) {
        let pinned = self
            .pinned_places
            .borrow()
            .iter()
            .enumerate()
            .filter(|(_, (location, _))| !is_standard_place_location(location))
            .filter(|(_, (location, _))| !self.local_only || location.native_path().is_some())
            .map(|(index, (location, name))| (index, location.clone(), name.clone()))
            .collect::<Vec<_>>();
        if !pinned.is_empty() {
            self.append_separator();
            self.append_heading("PINNED");
            for (index, location, name) in pinned {
                if self.local_only {
                    self.append_place(crate::assets::icons::FOLDER, &name, location);
                } else {
                    self.append_pinned_place(index, &name, location);
                }
            }
        }
    }

    fn append_devices(self: &Rc<Self>) {
        let volumes = self.volume_monitor.volumes();
        let mounts = self.mounts_without_volumes(&volumes);
        if volumes.is_empty() && mounts.is_empty() {
            return;
        }
        self.append_separator();
        self.append_heading("DEVICES");
        for volume in volumes {
            self.append_volume(volume);
        }
        for (name, location, mount) in mounts {
            self.append_mount(&name, location, mount);
        }
    }

    fn mounts_without_volumes(
        &self,
        volumes: &[gio::Volume],
    ) -> Vec<(String, Location, gio::Mount)> {
        self.volume_monitor
            .mounts()
            .into_iter()
            .filter(|mount| !mount.is_shadowed())
            .filter(|mount| {
                !mount
                    .volume()
                    .is_some_and(|volume| volumes.contains(&volume))
            })
            .filter_map(|mount| {
                let name = mount.name().to_string();
                let location = location_for_file(&mount.root())?;
                if self.local_only && location.native_path().is_none() {
                    return None;
                }
                Some((name, location, mount))
            })
            .collect()
    }

    fn append_mount(self: &Rc<Self>, name: &str, location: Location, mount: gio::Mount) {
        if is_smb_location(&location) {
            self.append_smb_mount(name, location, mount);
            return;
        }
        let action = mount_release_action(mount.can_eject(), mount.can_unmount());
        let release = action.map(|action| {
            let release_browser = self.browser.clone();
            let release_parent = self.view.widget();
            let release_in_flight = Rc::new(Cell::new(false));
            let on_release: Rc<dyn Fn()> = Rc::new(move || {
                release_device_mount(
                    &mount,
                    action,
                    &release_parent,
                    &release_browser,
                    &release_in_flight,
                );
            });
            (action, on_release)
        });
        self.append_device_place(crate::assets::icons::HARD_DRIVE, name, location, release);
    }

    fn pin_location(self: &Rc<Self>, location: Location, name: String) {
        self.update_pinned_places(|places| {
            if pin_status(places, &location) != PinStatus::Available {
                return false;
            }
            places.push((location, name));
            true
        });
    }

    fn unpin_location(self: &Rc<Self>, location: &Location) {
        self.update_pinned_places(|places| remove_pinned_place(places, location));
    }

    fn update_pinned_places(
        self: &Rc<Self>,
        change: impl FnOnce(&mut Vec<(Location, String)>) -> bool,
    ) {
        // Merge into the shared file, never this window's stale snapshot.
        let result = load_pinned_places().and_then(|mut places| {
            if change(&mut places) {
                save_pinned_places(&places)?;
            }
            Ok(places)
        });
        match result {
            Ok(places) => {
                self.pinned_places.replace(places);
                self.rebuild();
            }
            Err(error) => show_error_dialog(
                &self.view.widget(),
                "Unable to update pinned folders",
                &error.to_string(),
            ),
        }
    }
    fn event_changes_active_place(event: &BrowserEvent) -> bool {
        matches!(
            event,
            BrowserEvent::Reset
                | BrowserEvent::ColumnAdded { .. }
                | BrowserEvent::ColumnsTruncated { .. }
                | BrowserEvent::ColumnsRelocated { .. }
                | BrowserEvent::FocusChanged { .. }
        )
    }

    fn sync_active_place(&self) {
        let active = self.browser.active_location();
        let rows = self.place_rows.borrow();
        let selected = rows
            .iter()
            .position(|(location, row)| {
                active.as_ref() == Some(location) && row.has_css_class("active")
            })
            .or_else(|| {
                rows.iter()
                    .position(|(location, _)| active.as_ref() == Some(location))
            });
        for (index, (_, row)) in rows.iter().enumerate() {
            if selected == Some(index) {
                row.add_css_class("active");
            } else {
                row.remove_css_class("active");
            }
        }
    }

    pub(super) fn focus_active_place(&self) -> bool {
        let rows = self.place_rows.borrow();
        rows.iter()
            .find(|(_, row)| row.has_css_class("active"))
            .or_else(|| rows.first())
            .is_some_and(|(_, row)| row.grab_focus())
    }

    fn apply_trash_menu_visibility(&self) {
        if let Some(rows) = self.trash_menu_rows.borrow().as_ref() {
            sync_trash_menu_rows(rows, self.trash_contents.get());
        }
    }

    fn set_trash_contents(&self, contents: TrashContents) {
        if self.trash_contents.get() == contents {
            return;
        }
        self.trash_contents.set(contents);
        self.apply_trash_menu_visibility();
    }

    fn refresh_trash_contents(self: &Rc<Self>) {
        if self.local_only {
            return;
        }
        // A probe already in flight may have read Trash before this change landed, so queue
        // another pass instead of trusting the result it is about to return.
        if self.trash_probe_running.get() {
            self.trash_probe_pending.set(true);
            return;
        }
        self.trash_probe_running.set(true);
        let weak = Rc::downgrade(self);
        glib::MainContext::default().spawn_local(async move {
            let probe = trash_has_entries(&gio::File::for_uri("trash:///")).await;
            if let Err(error) = &probe {
                tracing::warn!(
                    error_domain = ?error.domain(),
                    error_code = error.code(),
                    "unable to read trash contents"
                );
            }
            if let Some(state) = weak.upgrade() {
                state.trash_probe_running.set(false);
                state.set_trash_contents(trash_contents_from_probe(probe));
                if state.trash_probe_pending.replace(false) {
                    state.refresh_trash_contents();
                }
            }
        });
    }

    /// Keeps the context menu current when another application changes `trash:///`.
    fn watch_trash(self: &Rc<Self>) {
        if self.trash_monitor.borrow().is_some() {
            return;
        }
        let trash = gio::File::for_uri("trash:///");
        let Ok(monitor) =
            trash.monitor_directory(gio::FileMonitorFlags::NONE, gio::Cancellable::NONE)
        else {
            return;
        };
        let weak = Rc::downgrade(self);
        monitor.connect_changed(move |_, _, _, _| {
            if let Some(state) = weak.upgrade() {
                state.refresh_trash_contents();
            }
        });
        *self.trash_monitor.borrow_mut() = Some(monitor);
    }

    fn append_trash_place(self: &Rc<Self>) {
        let location = Location::uri("trash:///");
        let row = sidebar_button(crate::assets::icons::TRASH, "Trash");
        row.set_tooltip_text(Some("trash:///"));
        self.bind_place_row(&row, location, PlaceNavigation::Direct);

        let menu = super::accessibility::menu_box();
        menu.add_css_class("folder-context-menu");
        let properties = sidebar_context_option(crate::assets::icons::INFO, "Properties", false);
        let empty = sidebar_context_option(crate::assets::icons::TRASH, "Empty Trash…", true);
        empty.add_css_class("danger");
        let separator = gtk::Separator::new(gtk::Orientation::Horizontal);
        menu.append(&properties);
        menu.append(&separator);
        menu.append(&empty);
        *self.trash_menu_rows.borrow_mut() = Some(TrashMenuRows {
            separator,
            empty: empty.clone(),
        });
        self.apply_trash_menu_visibility();
        self.watch_trash();
        self.refresh_trash_contents();
        self.view.set_trash_button(row.clone());
        let popover = gtk::Popover::builder()
            .child(&menu)
            .autohide(true)
            .has_arrow(false)
            .build();
        popover.add_css_class("folder-context-popover");
        popover.set_parent(&row);
        let properties_popover = popover.downgrade();
        let properties_view = self.view.clone();
        properties.connect_clicked(move |_| {
            if let Some(popover) = properties_popover.upgrade() {
                popover.popdown();
            }
            properties_view.show_location_properties(&Location::uri("trash:///"));
        });
        let empty_popover = popover.downgrade();
        let empty_view = self.view.clone();
        empty.connect_clicked(move |_| {
            if let Some(popover) = empty_popover.upgrade() {
                popover.popdown();
            }
            empty_view.confirm_empty_trash();
        });
        let context = gtk::GestureClick::new();
        context.set_button(3);
        let weak_popover = popover.downgrade();
        let weak_state = Rc::downgrade(self);
        context.connect_pressed(move |gesture, _, x, y| {
            gesture.set_state(gtk::EventSequenceState::Claimed);
            let Some(popover) = weak_popover.upgrade() else {
                return;
            };
            if let Some(state) = weak_state.upgrade() {
                state.refresh_trash_contents();
            }
            popover.set_pointing_to(Some(&gtk::gdk::Rectangle::new(
                x.round() as i32,
                y.round() as i32,
                1,
                1,
            )));
            popover.popup();
        });
        row.add_controller(context);
        self.widget.append(&row);
    }

    fn make_reorderable(
        self: &Rc<Self>,
        row: &gtk::Button,
        payload: impl Fn() -> String + 'static,
        on_drop: impl Fn(&Rc<Self>, &str, bool) -> bool + 'static,
    ) {
        row.add_css_class("reorderable");
        row.set_cursor_from_name(Some("pointer"));

        let drag = gtk::DragSource::builder()
            .actions(gtk::gdk::DragAction::MOVE)
            .build();
        drag.connect_prepare(move |_, _, _| {
            Some(gtk::gdk::ContentProvider::for_value(&payload().to_value()))
        });
        let dragged_row = row.clone();
        drag.connect_drag_begin(move |_, _| {
            dragged_row.add_css_class("dragging");
            dragged_row.set_cursor_from_name(Some("grabbing"));
        });
        let dragged_row = row.clone();
        drag.connect_drag_end(move |_, _, _| {
            dragged_row.remove_css_class("dragging");
            dragged_row.set_cursor_from_name(Some("pointer"));
        });
        row.add_controller(drag);

        let drop = gtk::DropTarget::new(String::static_type(), gtk::gdk::DragAction::MOVE);
        drop.connect_accept(|_, offered| {
            accepts_sidebar_reorder_payload(
                offered.formats().contains_type(String::static_type()),
                offered
                    .formats()
                    .contains_type(gtk::gdk::FileList::static_type()),
            )
        });
        let weak_state = Rc::downgrade(self);
        let target_row = row.clone();
        drop.connect_drop(move |_, value, _, y| {
            let Ok(source) = value.get::<String>() else {
                return false;
            };
            let after = y >= f64::from(target_row.height()) / 2.0;
            if let Some(state) = weak_state.upgrade() {
                return on_drop(&state, &source, after);
            }
            false
        });
        row.add_controller(drop);
    }

    fn append_reorderable_place(
        self: &Rc<Self>,
        id: &'static str,
        icon: &str,
        name: &str,
        location: Location,
    ) {
        let row = sidebar_button(icon, name);
        row.set_tooltip_text(Some(&location.display_path()));
        self.bind_place_row(&row, location, PlaceNavigation::Direct);

        self.make_reorderable(
            &row,
            // Standard rows drag their stable id, so a pinned row's numeric
            // payload is rejected by the standard-place drop handler.
            move || id.to_string(),
            move |state, source, after| {
                if source.starts_with(PINNED_DRAG_PREFIX) {
                    return false;
                }
                state.reorder_place(source, id, after);
                true
            },
        );
        self.widget.append(&row);
    }

    fn reorder_place(self: &Rc<Self>, source: &str, target: &str, after: bool) {
        let changed = reorder_places(&mut self.place_order.borrow_mut(), source, target, after);
        if changed {
            let order = self
                .place_order
                .borrow()
                .iter()
                .map(|place| (*place).to_owned())
                .collect();
            self.theme_manager.set_sidebar_order(order);
            self.rebuild();
        }
    }

    fn reorder_pinned_place(self: &Rc<Self>, source: usize, target: usize, after: bool) {
        let (source, target) = {
            let places = self.pinned_places.borrow();
            match (places.get(source), places.get(target)) {
                (Some(source), Some(target)) => (source.0.clone(), target.0.clone()),
                _ => return,
            }
        };
        self.update_pinned_places(|places| {
            let position =
                |location: &Location| places.iter().position(|(pinned, _)| pinned == location);
            match (position(&source), position(&target)) {
                (Some(source), Some(target)) => {
                    reorder_pinned_places(places, source, target, after)
                }
                _ => false,
            }
        });
    }

    fn append_separator(&self) {
        let separator = gtk::Separator::new(gtk::Orientation::Horizontal);
        separator.add_css_class("sidebar-separator");
        self.widget.append(&separator);
    }

    fn append_heading(&self, text: &str) {
        let heading = gtk::Label::new(Some(text));
        heading.add_css_class("sidebar-heading");
        heading.set_xalign(0.0);
        self.widget.append(&heading);
    }

    fn append_volume(&self, volume: gio::Volume) {
        let name = volume.name().to_string();
        let row = sidebar_button(crate::assets::icons::HARD_DRIVE, &name);
        row.set_tooltip_text(Some(&name));
        if let Some(mount) = volume.get_mount()
            && let Some(location) = location_for_file(&mount.root())
        {
            self.place_rows
                .borrow_mut()
                .push((location.clone(), row.clone()));
            install_sidebar_file_drop(&self.view, &row, location);
        }
        let weak_browser = Rc::downgrade(&self.browser);
        let sidebar = self.widget.clone();
        let selected_row = row.clone();
        let clicked_volume = volume.clone();
        let mount_view = self.view.clone();
        row.connect_clicked(move |_| {
            let volume = clicked_volume.clone();
            select_sidebar_row(&sidebar, &selected_row);
            let Some(browser) = weak_browser.upgrade() else {
                return;
            };
            if let Some(mount) = volume.get_mount() {
                navigate_to_gio_file(&browser, &mount.root());
                return;
            }

            mount_view.mount_volume(volume);
        });
        let (mount_can_eject, mount_can_unmount) = volume
            .get_mount()
            .map(|mount| (mount.can_eject(), mount.can_unmount()))
            .unwrap_or((false, false));
        match volume_release_action(volume.can_eject(), mount_can_eject, mount_can_unmount) {
            Some(action) => {
                let release_volume = volume.clone();
                let release_browser = self.browser.clone();
                let release_parent = self.view.widget();
                let release_in_flight = Rc::new(Cell::new(false));
                let on_release: Rc<dyn Fn()> = Rc::new(move || {
                    release_device_volume(
                        &release_volume,
                        action,
                        &release_parent,
                        &release_browser,
                        &release_in_flight,
                    );
                });
                let eject = sidebar_eject_button(action, {
                    let on_release = on_release.clone();
                    move || on_release()
                });
                attach_device_release_menu(&row, action, on_release);
                self.widget.append(&sidebar_device_row(&row, &eject));
            }
            None => {
                self.widget.append(&row);
            }
        }
    }

    fn append_smb_mount(self: &Rc<Self>, name: &str, location: Location, mount: gio::Mount) {
        let properties_location = location.clone();
        let row = self.append_place(crate::assets::icons::NETWORK, name, location);
        let menu = super::accessibility::menu_box();
        menu.add_css_class("folder-context-menu");
        let properties = sidebar_context_option(crate::assets::icons::INFO, "Properties", false);
        let disconnect = sidebar_context_option(crate::assets::icons::UNPLUG, "Disconnect", true);
        disconnect.add_css_class("danger");
        menu.append(&properties);
        menu.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        menu.append(&disconnect);
        let popover = gtk::Popover::builder()
            .child(&menu)
            .autohide(true)
            .has_arrow(false)
            .build();
        popover.add_css_class("folder-context-popover");
        popover.set_parent(&row);

        let properties_popover = popover.downgrade();
        let properties_view = self.view.clone();
        properties.connect_clicked(move |_| {
            if let Some(popover) = properties_popover.upgrade() {
                popover.popdown();
            }
            properties_view.show_location_properties(&properties_location);
        });

        let disconnect_popover = popover.downgrade();
        let parent = self.view.widget();
        disconnect.connect_clicked(move |_| {
            if let Some(popover) = disconnect_popover.upgrade() {
                popover.popdown();
            }
            let window = parent.root().and_downcast::<gtk::Window>();
            let operation = gtk::MountOperation::new(window.as_ref());
            let mount = mount.clone();
            let error_parent = parent.clone();
            glib::MainContext::default().spawn_local(async move {
                if let Err(error) = mount
                    .unmount_with_operation_future(gio::MountUnmountFlags::NONE, Some(&operation))
                    .await
                    && !error.matches(gio::IOErrorEnum::Cancelled)
                {
                    show_error_dialog(&error_parent, "Unable to disconnect", &error.to_string());
                }
            });
        });

        let context = gtk::GestureClick::new();
        context.set_button(3);
        let weak_popover = popover.downgrade();
        context.connect_pressed(move |gesture, _, x, y| {
            gesture.set_state(gtk::EventSequenceState::Claimed);
            let Some(popover) = weak_popover.upgrade() else {
                return;
            };
            popover.set_pointing_to(Some(&gtk::gdk::Rectangle::new(
                x.round() as i32,
                y.round() as i32,
                1,
                1,
            )));
            popover.popup();
        });
        row.add_controller(context);
    }

    fn append_pinned_place(self: &Rc<Self>, index: usize, name: &str, location: Location) {
        let row = self.append_place(crate::assets::icons::FOLDER, name, location.clone());
        self.make_pinned_row_reorderable(&row, index);
        let menu = super::accessibility::menu_box();
        menu.add_css_class("folder-context-menu");
        let unpin = sidebar_context_option(crate::assets::icons::PIN, "Unpin", false);
        let properties = sidebar_context_option(crate::assets::icons::INFO, "Properties", false);
        menu.append(&unpin);
        menu.append(&properties);
        let popover = gtk::Popover::builder()
            .child(&menu)
            .autohide(true)
            .has_arrow(false)
            .build();
        popover.add_css_class("folder-context-popover");
        popover.set_parent(&row);

        let weak_state = Rc::downgrade(self);
        let unpinned_location = location.clone();
        let unpin_popover = popover.downgrade();
        unpin.connect_clicked(move |_| {
            if let Some(popover) = unpin_popover.upgrade() {
                popover.popdown();
            }
            if let Some(state) = weak_state.upgrade() {
                state.unpin_location(&unpinned_location);
            }
        });
        let properties_view = self.view.clone();
        let properties_location = location;
        let properties_popover = popover.downgrade();
        properties.connect_clicked(move |_| {
            if let Some(popover) = properties_popover.upgrade() {
                popover.popdown();
            }
            properties_view.show_location_properties(&properties_location);
        });
        let context = gtk::GestureClick::new();
        context.set_button(3);
        let weak_popover = popover.downgrade();
        context.connect_pressed(move |gesture, _, x, y| {
            gesture.set_state(gtk::EventSequenceState::Claimed);
            let Some(popover) = weak_popover.upgrade() else {
                return;
            };
            popover.set_pointing_to(Some(&gtk::gdk::Rectangle::new(
                x.round() as i32,
                y.round() as i32,
                1,
                1,
            )));
            popover.popup();
        });
        row.add_controller(context);
    }

    fn make_pinned_row_reorderable(self: &Rc<Self>, row: &gtk::Button, index: usize) {
        self.make_reorderable(
            row,
            move || format!("{PINNED_DRAG_PREFIX}{index}"),
            move |state, source, after| {
                let Some(source) = parse_pinned_drag_source(source) else {
                    return false;
                };
                state.reorder_pinned_place(source, index, after);
                true
            },
        );
    }

    fn append_place(&self, icon: &str, name: &str, location: Location) -> gtk::Button {
        self.append_device_place(icon, name, location, None)
    }

    fn append_device_place(
        &self,
        icon: &str,
        name: &str,
        location: Location,
        release: Option<(MediaRelease, Rc<dyn Fn()>)>,
    ) -> gtk::Button {
        let row = sidebar_button(icon, name);
        row.set_tooltip_text(Some(&location.display_path()));
        self.bind_place_row(&row, location, PlaceNavigation::Validate);
        match release {
            Some((action, on_release)) => {
                let eject = sidebar_eject_button(action, {
                    let on_release = on_release.clone();
                    move || on_release()
                });
                attach_device_release_menu(&row, action, on_release);
                self.widget.append(&sidebar_device_row(&row, &eject));
            }
            None => {
                self.widget.append(&row);
            }
        }
        row
    }
}

fn accepts_sidebar_reorder_payload(has_string: bool, has_file_list: bool) -> bool {
    has_string && !has_file_list
}

fn sidebar_accepts_file_drop(location: &Location) -> bool {
    location.native_path().is_some()
}

fn install_sidebar_file_drop(
    view: &BrowserView,
    row: &impl IsA<gtk::Widget>,
    destination: Location,
) {
    if destination == Location::uri("trash:///") {
        install_sidebar_trash_drop(view, row);
        return;
    }
    if !sidebar_accepts_file_drop(&destination) {
        return;
    }
    row.add_css_class("file-drop-zone");
    let PreparedFileDrop {
        target: drop,
        state: drop_state,
    } = prepare_file_drop_target({
        let destination = destination.clone();
        move || Some(destination.clone())
    });
    drop.set_propagation_phase(gtk::PropagationPhase::Capture);
    let state_for_enter = drop_state.clone();
    drop.connect_enter(move |target, _, _| file_drop_action(target, &state_for_enter));
    let state_for_motion = drop_state.clone();
    drop.connect_motion(move |target, _, _| file_drop_action(target, &state_for_motion));
    let view = view.clone();
    drop.connect_drop(move |target, value, _, _| {
        let Some(sources) = locations_from_file_list_value(value) else {
            return false;
        };
        if sources.is_empty() {
            return false;
        }
        let commit = file_drop_commit(target, &destination, &sources, &drop_state);
        view.commit_file_drop(destination.clone(), sources, commit);
        true
    });
    row.add_controller(drop);
}

fn trash_file_drop_action(target: &gtk::DropTarget) -> gtk::gdk::DragAction {
    if target.value().as_ref().is_some_and(|value| {
        locations_from_file_list_value(value)
            .is_none_or(|sources| !BrowserView::can_trash_file_drop(&sources))
    }) {
        gtk::gdk::DragAction::empty()
    } else {
        gtk::gdk::DragAction::MOVE
    }
}

fn install_sidebar_trash_drop(view: &BrowserView, row: &impl IsA<gtk::Widget>) {
    row.add_css_class("file-drop-zone");
    let drop = gtk::DropTarget::new(
        gtk::gdk::FileList::static_type(),
        gtk::gdk::DragAction::MOVE,
    );
    drop.set_preload(true);
    drop.set_propagation_phase(gtk::PropagationPhase::Capture);
    drop.connect_enter(|target, _, _| trash_file_drop_action(target));
    drop.connect_motion(|target, _, _| trash_file_drop_action(target));
    drop.connect_value_notify(|target| {
        if let Some(offered) = target.current_drop() {
            offered.status(target.actions(), trash_file_drop_action(target));
        }
    });
    let view = view.clone();
    drop.connect_drop(move |_, value, _, _| {
        locations_from_file_list_value(value).is_some_and(|sources| view.trash_file_drop(sources))
    });
    row.add_controller(drop);
}

fn select_sidebar_row(sidebar: &gtk::Box, selected: &gtk::Button) {
    let mut child = sidebar.first_child();
    while let Some(widget) = child {
        if let Some(row) = sidebar_row_button(&widget) {
            row.remove_css_class("active");
        }
        child = widget.next_sibling();
    }
    selected.add_css_class("active");
}

/// Resolves the navigable row button for a sidebar child. Device rows wrap
/// their button with an eject sibling, so look one level down when the child
/// itself is a container.
fn sidebar_row_button(widget: &gtk::Widget) -> Option<gtk::Button> {
    widget.clone().downcast::<gtk::Button>().ok().or_else(|| {
        widget
            .first_child()
            .and_then(|child| child.downcast::<gtk::Button>().ok())
    })
}

fn reorder_places(order: &mut Vec<&'static str>, source: &str, target: &str, after: bool) -> bool {
    if source == target {
        return false;
    }
    let Some(source_index) = order.iter().position(|place| *place == source) else {
        return false;
    };
    let source = order.remove(source_index);
    let Some(target_index) = order.iter().position(|place| *place == target) else {
        order.insert(source_index, source);
        return false;
    };
    order.insert(target_index + usize::from(after), source);
    true
}

fn reorder_pinned_places(
    places: &mut Vec<(Location, String)>,
    source: usize,
    target: usize,
    after: bool,
) -> bool {
    if source == target || source >= places.len() || target >= places.len() {
        return false;
    }
    let place = places.remove(source);
    // `source` is a pre-removal index, but `target` must remap onto the shrunken
    // vector: any target that sat after `source` slides left by one. Since
    // `destination == source` is checked in post-removal coordinates, a match
    // means re-inserting into the original slot — a no-op worth reporting.
    let target = target - usize::from(target > source);
    let destination = (target + usize::from(after)).min(places.len());
    if destination == source {
        places.insert(source, place);
        return false;
    }
    places.insert(destination, place);
    true
}

fn parse_pinned_drag_source(source: &str) -> Option<usize> {
    source.strip_prefix(PINNED_DRAG_PREFIX)?.parse().ok()
}

fn pin_status(places: &[(Location, String)], location: &Location) -> PinStatus {
    if is_standard_place_location(location) {
        PinStatus::Unavailable
    } else if places.iter().any(|(pinned, _)| pinned == location) {
        PinStatus::Pinned
    } else {
        PinStatus::Available
    }
}

fn remove_pinned_place(places: &mut Vec<(Location, String)>, location: &Location) -> bool {
    let original_len = places.len();
    places.retain(|(pinned, _)| pinned != location);
    places.len() != original_len
}

fn is_smb_location(location: &Location) -> bool {
    location.uri_value().is_some_and(|uri| {
        uri.get(..4)
            .is_some_and(|scheme| scheme.eq_ignore_ascii_case("smb:"))
    })
}

/// Safe-removal action for a sidebar device row. Eject is preferred whenever
/// the drive reports it (USB sticks, optical discs); plain unmount covers
/// mounts without ejectable media. `None` means fixed internal storage with
/// no actionable release.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MediaRelease {
    EjectVolume,
    EjectMount,
    UnmountMount,
}

fn volume_release_action(
    can_eject_volume: bool,
    mount_can_eject: bool,
    mount_can_unmount: bool,
) -> Option<MediaRelease> {
    if can_eject_volume {
        Some(MediaRelease::EjectVolume)
    } else if mount_can_eject {
        Some(MediaRelease::EjectMount)
    } else if mount_can_unmount {
        Some(MediaRelease::UnmountMount)
    } else {
        None
    }
}

fn mount_release_action(can_eject: bool, can_unmount: bool) -> Option<MediaRelease> {
    if can_eject {
        Some(MediaRelease::EjectMount)
    } else if can_unmount {
        Some(MediaRelease::UnmountMount)
    } else {
        None
    }
}

fn media_release_label(action: MediaRelease) -> &'static str {
    match action {
        MediaRelease::EjectVolume | MediaRelease::EjectMount => "Eject",
        MediaRelease::UnmountMount => "Unmount",
    }
}

fn media_release_error_title(action: MediaRelease) -> &'static str {
    match action {
        MediaRelease::EjectVolume | MediaRelease::EjectMount => "Unable to eject device",
        MediaRelease::UnmountMount => "Unable to unmount device",
    }
}

fn navigate_home_if_within(browser: &Rc<Browser>, root: &gio::File) {
    let Some(device) = location_for_file(root) else {
        return;
    };
    let active = browser.active_location();
    if active
        .as_ref()
        .is_some_and(|active| active == &device || active.is_within(&device))
    {
        browser.navigate(Location::local(home_directory()));
    }
}

fn begin_media_release(in_flight: &Cell<bool>) -> bool {
    !in_flight.replace(true)
}

fn release_device_volume(
    volume: &gio::Volume,
    action: MediaRelease,
    parent: &gtk::Widget,
    browser: &Rc<Browser>,
    in_flight: &Rc<Cell<bool>>,
) {
    if !begin_media_release(in_flight) {
        return;
    }
    let mount = volume.get_mount();
    let away_root = mount.as_ref().map(gio::Mount::root);
    let window = parent.root().and_downcast::<gtk::Window>();
    let operation = gtk::MountOperation::new(window.as_ref());
    let error_parent = parent.clone();
    let browser = browser.clone();
    let volume = volume.clone();
    let in_flight = in_flight.clone();
    let title = media_release_error_title(action);
    glib::MainContext::default().spawn_local(async move {
        let result = match action {
            MediaRelease::EjectVolume => {
                volume
                    .eject_with_operation_future(gio::MountUnmountFlags::NONE, Some(&operation))
                    .await
            }
            MediaRelease::EjectMount => match mount {
                Some(mount) => {
                    mount
                        .eject_with_operation_future(gio::MountUnmountFlags::NONE, Some(&operation))
                        .await
                }
                None => {
                    in_flight.set(false);
                    return;
                }
            },
            MediaRelease::UnmountMount => match mount {
                Some(mount) => {
                    mount
                        .unmount_with_operation_future(
                            gio::MountUnmountFlags::NONE,
                            Some(&operation),
                        )
                        .await
                }
                None => {
                    in_flight.set(false);
                    return;
                }
            },
        };
        in_flight.set(false);
        match result {
            Ok(()) => {
                if let Some(root) = away_root {
                    navigate_home_if_within(&browser, &root);
                }
            }
            Err(error) if error.matches(gio::IOErrorEnum::Cancelled) => {}
            Err(error) => show_error_dialog(&error_parent, title, &error.to_string()),
        }
    });
}

fn release_device_mount(
    mount: &gio::Mount,
    action: MediaRelease,
    parent: &gtk::Widget,
    browser: &Rc<Browser>,
    in_flight: &Rc<Cell<bool>>,
) {
    if !begin_media_release(in_flight) {
        return;
    }
    let away_root = mount.root();
    let window = parent.root().and_downcast::<gtk::Window>();
    let operation = gtk::MountOperation::new(window.as_ref());
    let error_parent = parent.clone();
    let browser = browser.clone();
    let mount = mount.clone();
    let in_flight = in_flight.clone();
    let title = media_release_error_title(action);
    glib::MainContext::default().spawn_local(async move {
        let result = match action {
            MediaRelease::EjectVolume | MediaRelease::EjectMount => {
                mount
                    .eject_with_operation_future(gio::MountUnmountFlags::NONE, Some(&operation))
                    .await
            }
            MediaRelease::UnmountMount => {
                mount
                    .unmount_with_operation_future(gio::MountUnmountFlags::NONE, Some(&operation))
                    .await
            }
        };
        in_flight.set(false);
        match result {
            Ok(()) => navigate_home_if_within(&browser, &away_root),
            Err(error) if error.matches(gio::IOErrorEnum::Cancelled) => {}
            Err(error) => show_error_dialog(&error_parent, title, &error.to_string()),
        }
    });
}

fn is_standard_place_location(location: &Location) -> bool {
    let Some(path) = location.native_path() else {
        return false;
    };
    if path == home_directory() {
        return true;
    }
    [
        glib::UserDirectory::Desktop,
        glib::UserDirectory::Documents,
        glib::UserDirectory::Downloads,
        glib::UserDirectory::Pictures,
        glib::UserDirectory::Videos,
    ]
    .into_iter()
    .filter_map(glib::user_special_dir)
    .any(|standard| standard == path)
}

fn should_show_standard_place(id: &str, path: &std::path::Path, home: &std::path::Path) -> bool {
    id != "desktop" || path != home
}

fn resolve_place_order(persisted: &[String]) -> Vec<&'static str> {
    let mut order: Vec<&'static str> = Vec::new();
    for id in persisted {
        if let Some(canonical) = STANDARD_PLACE_IDS
            .iter()
            .find(|&&known| known == id.as_str())
            && !order.contains(canonical)
        {
            order.push(*canonical);
        }
    }
    for &id in STANDARD_PLACE_IDS {
        if !order.contains(&id) {
            order.push(id);
        }
    }
    order
}

fn standard_place(id: &str) -> Option<(&'static str, &'static str, glib::UserDirectory)> {
    match id {
        "desktop" => Some((
            crate::assets::icons::FOLDER,
            "Desktop",
            glib::UserDirectory::Desktop,
        )),
        "documents" => Some((
            crate::assets::icons::DOCUMENTS,
            "Documents",
            glib::UserDirectory::Documents,
        )),
        "downloads" => Some((
            crate::assets::icons::DOWNLOADS,
            "Downloads",
            glib::UserDirectory::Downloads,
        )),
        "pictures" => Some((
            crate::assets::icons::PICTURES,
            "Pictures",
            glib::UserDirectory::Pictures,
        )),
        "videos" => Some((
            crate::assets::icons::VIDEOS,
            "Videos",
            glib::UserDirectory::Videos,
        )),
        _ => None,
    }
}

async fn trash_has_entries(root: &gio::File) -> Result<bool, glib::Error> {
    let enumerator = root
        .enumerate_children_future(
            gio::FILE_ATTRIBUTE_STANDARD_NAME,
            gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
            glib::Priority::DEFAULT,
        )
        .await?;
    let children = enumerator
        .next_files_future(1, glib::Priority::DEFAULT)
        .await?;
    Ok(!children.is_empty())
}

fn sidebar_context_option(icon: &str, label: &str, danger: bool) -> gtk::Button {
    let button = super::accessibility::menu_item_button();
    super::accessibility::describe_menu_item(&button, label, "");
    button.add_css_class("item-context-option");
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let icon = if danger {
        crate::assets::danger_icon(icon, 15)
    } else {
        crate::assets::primary_icon(icon, 15)
    };
    icon.add_css_class("item-context-icon");
    let title = gtk::Label::new(Some(label));
    title.set_xalign(0.0);
    title.set_hexpand(true);
    row.append(&icon);
    row.append(&title);
    button.set_child(Some(&row));
    button
}

fn sidebar_button(icon: &str, name: &str) -> gtk::Button {
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let image = crate::assets::primary_icon(icon, 17);
    let label = gtk::Label::new(Some(name));
    label.set_xalign(0.0);
    label.set_hexpand(true);
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    content.append(&image);
    content.append(&label);

    let row = gtk::Button::builder()
        .child(&content)
        .halign(gtk::Align::Fill)
        .build();
    row.add_css_class("sidebar-row");
    row.set_cursor_from_name(Some("pointer"));
    row.set_has_frame(false);
    row
}

fn sidebar_eject_button(action: MediaRelease, on_release: impl Fn() + 'static) -> gtk::Button {
    let button = gtk::Button::builder()
        .tooltip_text(media_release_label(action))
        .build();
    button.set_child(Some(&crate::assets::primary_icon(
        crate::assets::icons::EJECT,
        14,
    )));
    button.add_css_class("sidebar-eject");
    button.set_cursor_from_name(Some("pointer"));
    button.set_has_frame(false);
    button.set_valign(gtk::Align::Center);
    button.connect_clicked(move |_| on_release());
    button
}

fn sidebar_device_row(row: &gtk::Button, eject: &gtk::Button) -> gtk::Box {
    let shell = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    shell.add_css_class("sidebar-device");
    row.set_hexpand(true);
    shell.append(row);
    shell.append(eject);
    shell
}

fn attach_device_release_menu(row: &gtk::Button, action: MediaRelease, on_release: Rc<dyn Fn()>) {
    let menu = super::accessibility::menu_box();
    menu.add_css_class("folder-context-menu");
    let release = sidebar_context_option(
        crate::assets::icons::EJECT,
        media_release_label(action),
        false,
    );
    menu.append(&release);
    let popover = gtk::Popover::builder()
        .child(&menu)
        .autohide(true)
        .has_arrow(false)
        .build();
    popover.add_css_class("folder-context-popover");
    popover.set_parent(row);
    let release_popover = popover.downgrade();
    release.connect_clicked(move |_| {
        if let Some(popover) = release_popover.upgrade() {
            popover.popdown();
        }
        on_release();
    });
    let context = gtk::GestureClick::new();
    context.set_button(3);
    let weak_popover = popover.downgrade();
    context.connect_pressed(move |gesture, _, x, y| {
        gesture.set_state(gtk::EventSequenceState::Claimed);
        let Some(popover) = weak_popover.upgrade() else {
            return;
        };
        popover.set_pointing_to(Some(&gtk::gdk::Rectangle::new(
            x.round() as i32,
            y.round() as i32,
            1,
            1,
        )));
        popover.popup();
    });
    row.add_controller(context);
}

fn navigate_to_gio_file(browser: &Rc<Browser>, file: &gio::File) {
    if let Some(location) = location_for_file(file) {
        browser.navigate(location);
    }
}

/// The sidebar update-notice pill's label text: `v{version} available` for a
/// stable offer, or `v{version} ({label}) available` for a prerelease --
/// e.g. `v0.5.0-rc.1 (Release candidate) available` -- so a preview build
/// offer is never mistaken for an ordinary stable update at a glance.
///
/// No channel guard belongs here: `check_for_updates` is already
/// channel-filtered upstream, so a Stable user's `release` can never carry
/// a prerelease kind in the first place.
fn sidebar_update_label(release: &ReleaseMetadata) -> String {
    if release.kind == BuildKind::Stable {
        format!("v{} available", release.version)
    } else {
        format!("v{} ({}) available", release.version, release.kind.label())
    }
}

fn pinned_places_path() -> PathBuf {
    glib::user_config_dir().join("gtk-3.0/bookmarks")
}

fn load_pinned_places() -> std::io::Result<Vec<(Location, String)>> {
    match std::fs::read(pinned_places_path()) {
        Ok(contents) => Ok(parse_pinned_places(&contents)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(error),
    }
}

/// GTK bookmarks may contain non-UTF-8 labels.
fn parse_pinned_places(contents: &[u8]) -> Vec<(Location, String)> {
    let mut places = Vec::new();
    for line in contents.split(|byte| *byte == b'\n') {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        let (uri, label) = match line.iter().position(|byte| *byte == b' ') {
            Some(space) => (&line[..space], Some(&line[space + 1..])),
            None => (line, None),
        };
        if uri.is_empty() {
            continue;
        }
        let Ok(uri) = std::str::from_utf8(uri) else {
            continue;
        };
        let file = gio::File::for_uri(uri);
        let Some(location) = location_for_file(&file) else {
            continue;
        };
        if places
            .iter()
            .any(|(existing, _): &(Location, String)| existing == &location)
        {
            continue;
        }
        let name = label
            .filter(|label| !label.is_empty())
            .map(|label| String::from_utf8_lossy(label).into_owned())
            .unwrap_or_else(|| location.display_name());
        places.push((location, name));
    }
    places
}

fn save_pinned_places(places: &[(Location, String)]) -> std::io::Result<()> {
    let path = pinned_places_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let contents = serialize_pinned_places(places);
    crate::storage::atomic_write(&path, contents.as_bytes())
}

fn serialize_pinned_places(places: &[(Location, String)]) -> String {
    let mut contents = String::new();
    for (location, name) in places {
        let uri = location
            .native_path()
            .map(gio::File::for_path)
            .map(|file| file.uri().to_string())
            .or_else(|| location.uri_value().map(str::to_owned));
        let Some(uri) = uri.and_then(|uri| sanitize_uri_credentials(&uri).ok().map(|(uri, _)| uri))
        else {
            continue;
        };
        let label = name.replace(['\n', '\r'], " ");
        contents.push_str(&format!("{uri} {label}\n"));
    }
    contents
}

pub(crate) fn home_directory() -> PathBuf {
    env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

/// Resolves the directory Strata opens at launch. Falls back to the home
/// directory when no default is saved or the saved directory no longer exists,
/// and clears a stale saved value so it stays reset until a new one is chosen.
fn startup_location(manager: &ThemeManager) -> Location {
    let saved = manager.default_directory();
    if let Some(path) = &saved
        && path.is_dir()
    {
        return Location::local(path);
    }
    if saved.is_some() {
        manager.set_default_directory(None);
    }
    Location::local(home_directory())
}

#[cfg(test)]
mod tests;

pub(super) fn load_styles() {
    let provider = gtk::CssProvider::new();
    provider.load_from_string(include_str!("../style.css"));

    if let Some(display) = gtk::gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}
