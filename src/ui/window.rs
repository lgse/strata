// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    cell::{Cell, RefCell},
    env,
    path::PathBuf,
    rc::Rc,
    time::{Duration, Instant},
};

use gtk::{gio, glib, prelude::*};

use crate::{
    adapters::{
        LocalFileSource, LocalOperationProvider, LocalPreviewProvider, RevealRequest,
        location_for_file,
    },
    app::{Browser, BrowserEvent},
    model::{EntryKind, FileEntry, Location, MetadataValue},
    services::{BuildKind, ReleaseMetadata, sanitize_uri_credentials},
};

use super::{
    blur::BlurBin,
    browser::{BrowserView, PeekBehavior, PinStatus, show_error_dialog},
    browser_modes::{BrowserDensity, BrowserMode},
    motion::{animations_enabled, emphasized_deceleration},
    preview::PreviewDrawer,
    search::SearchDialog,
    theme::ThemeManager,
};

const SIDEBAR_WIDTH: i32 = 208;
const MIN_SIDEBAR_WIDTH: i32 = 176;
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

impl TypeToSearch {
    fn show(&self, query: char) -> bool {
        self.preferences.type_to_search() && self.view.show_filter_with_query(&query.to_string())
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
    present_target(application, None, Vec::new(), false);
}

pub fn present_location(application: &gtk::Application, location: Option<PathBuf>) {
    present_target(
        application,
        location.map(Location::local),
        Vec::new(),
        false,
    );
}

/// Opens the window an `org.freedesktop.FileManager1` caller asked for: the
/// directory holding the named items, with those items selected.
pub fn present_reveal(application: &gtk::Application, request: RevealRequest) {
    present_target(
        application,
        Some(request.directory),
        request.selection,
        request.properties,
    );
}

fn present_target(
    application: &gtk::Application,
    location: Option<Location>,
    selection: Vec<String>,
    properties: bool,
) {
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

    let browser = BrowserView::new(Rc::new(LocalFileSource), PeekBehavior::default());
    browser.set_view_mode(theme_manager.browser_mode());
    browser.set_density(theme_manager.browser_density());
    browser.set_group_by_type(theme_manager.group_by_type());
    browser.set_operation_provider(Rc::new(LocalOperationProvider));
    browser.set_auto_refresh_interval(theme_manager.auto_refresh_interval());
    let controller = browser.browser();

    let preview_preferences = theme_manager.clone();
    let preview = PreviewDrawer::new(Rc::new(LocalPreviewProvider::new(Rc::new(move || {
        preview_preferences.media_preview_backend()
    }))));
    let preview_for_selection = preview.clone();
    let weak_controller = Rc::downgrade(&controller);
    controller.observe(move |event| match event {
        BrowserEvent::PreviewRequested { entry } => preview_for_selection.show(entry.clone()),
        BrowserEvent::FocusChanged {
            depth,
            position: Some(position),
        } if preview_for_selection.is_open() => {
            if let Some(entry) = weak_controller
                .upgrade()
                .and_then(|browser| browser.entry_at(*depth, *position))
            {
                if entry.is_directory() {
                    preview_for_selection.close();
                } else {
                    preview_for_selection.show(entry);
                }
            }
        }
        BrowserEvent::FocusChanged { position: None, .. } if preview_for_selection.is_open() => {
            preview_for_selection.close();
        }
        _ => {}
    });

    let header = gtk::HeaderBar::new();
    header.set_show_title_buttons(false);
    let sidebar_toggle = gtk::ToggleButton::builder()
        .active(true)
        .tooltip_text("Toggle sidebar (Ctrl+B)")
        .build();
    sidebar_toggle.set_child(Some(&crate::assets::primary_icon(
        crate::assets::icons::PANEL_LEFT,
        20,
    )));
    sidebar_toggle.add_css_class("sidebar-toggle");
    let location_widget = browser.location_widget();
    location_widget.set_hexpand(true);
    let search_button = gtk::Button::builder()
        .tooltip_text("Search (Ctrl+K)")
        .build();
    search_button.set_child(Some(&crate::assets::primary_icon(
        crate::assets::icons::SEARCH,
        20,
    )));
    search_button.add_css_class("header-action");
    let appearance = build_appearance_menu(&browser, &controller, theme_manager.clone());
    let settings = gtk::Button::builder().tooltip_text("Settings").build();
    settings.set_child(Some(&crate::assets::primary_icon(
        crate::assets::icons::SETTINGS,
        20,
    )));
    settings.add_css_class("header-action");
    let close_window = gtk::Button::builder().tooltip_text("Close window").build();
    close_window.set_child(Some(&crate::assets::primary_icon(
        crate::assets::icons::X,
        20,
    )));
    close_window.add_css_class("header-action");
    let closing_window = window.clone();
    close_window.connect_clicked(move |_| closing_window.close());
    let header_actions = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    header_actions.add_css_class("header-actions");
    header_actions.append(&search_button);
    header_actions.append(&appearance);
    header_actions.append(&settings);
    header_actions.append(&close_window);
    let header_content = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    header_content.set_hexpand(true);
    header_content.append(&sidebar_toggle);
    header_content.append(&location_widget);
    header_content.append(&header_actions);
    header.set_title_widget(Some(&header_content));

    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.append(&header);

    let content = gtk::Paned::new(gtk::Orientation::Horizontal);
    content.set_wide_handle(false);
    content.set_shrink_start_child(false);
    content.set_resize_start_child(false);
    content.set_position(SIDEBAR_WIDTH);
    content.set_vexpand(true);
    let sidebar = build_sidebar(browser.clone(), theme_manager.clone());
    let weak_sidebar = Rc::downgrade(&sidebar.state);
    let pinned_places = sidebar.state.pinned_places.clone();
    browser.set_pin_handlers(
        Rc::new(move |location, name| {
            if let Some(sidebar) = weak_sidebar.upgrade() {
                sidebar.pin_location(location, name);
            }
        }),
        Rc::new(move |location| pin_status(&pinned_places.borrow(), location)),
    );
    let preview_for_print = preview.clone();
    browser.set_print_handler(Rc::new(move |entry| preview_for_print.print_entry(entry)));
    sidebar.widget.set_size_request(MIN_SIDEBAR_WIDTH, -1);
    browser.add_marquee_origin(&sidebar.widget);
    content.set_start_child(Some(&sidebar.widget));
    content.set_end_child(Some(&browser.widget()));
    let animation_generation = Rc::new(Cell::new(0));
    let sidebar_animating = Rc::new(Cell::new(false));
    let constrained_content = content.clone();
    let constrained_toggle = sidebar_toggle.clone();
    let constrained_animation = sidebar_animating.clone();
    content.connect_position_notify(move |_| {
        if constrained_toggle.is_active()
            && !constrained_animation.get()
            && constrained_content.position() < MIN_SIDEBAR_WIDTH
        {
            constrained_content.set_position(MIN_SIDEBAR_WIDTH);
        }
    });
    let animated_content = content.clone();
    let animated_sidebar = sidebar.widget.clone();
    sidebar_toggle.connect_toggled(move |toggle| {
        animate_sidebar(
            &animated_content,
            &animated_sidebar,
            &animation_generation,
            &sidebar_animating,
            toggle.is_active(),
        );
    });
    let preview_split = gtk::Paned::new(gtk::Orientation::Horizontal);
    preview_split.add_css_class("preview-split");
    preview_split.set_wide_handle(false);
    preview_split.set_resize_start_child(true);
    preview_split.set_resize_end_child(false);
    preview_split.set_shrink_start_child(false);
    preview_split.set_shrink_end_child(true);
    preview_split.set_start_child(Some(&content));
    preview_split.set_end_child(Some(&preview.widget()));
    preview_split.set_position(i32::MAX);
    preview_split.set_vexpand(true);
    let measured_content = content.clone();
    let measured_browser = browser.clone();
    preview.attach_split(
        &preview_split,
        Rc::new(move || measured_content.position() + measured_browser.preview_occupied_width()),
    );
    root.append(&preview_split);

    let mouse_history = gtk::GestureClick::new();
    mouse_history.set_button(0);
    mouse_history.set_propagation_phase(gtk::PropagationPhase::Bubble);
    let weak_controller = Rc::downgrade(&controller);
    mouse_history.connect_pressed(move |gesture, _, _, _| {
        let Some(browser) = weak_controller.upgrade() else {
            return;
        };
        match mouse_history_action(gesture.current_button()) {
            Some(MouseHistoryAction::Back) if browser.can_go_back() => browser.back(),
            Some(MouseHistoryAction::Forward) if browser.can_go_forward() => browser.forward(),
            _ => return,
        }
        gesture.set_state(gtk::EventSequenceState::Claimed);
    });
    root.add_controller(mouse_history);
    super::scrolling::install_autoscroll_stop(&root);

    let window_overlay = gtk::Overlay::new();
    let blurred_root = BlurBin::new(&root);
    window_overlay.set_child(Some(&blurred_root));

    let search_controller = controller.clone();
    let search_preview = preview.clone();
    let search_preferences = theme_manager.clone();
    let activate_search_result = Rc::new(move |item: crate::services::SearchItem| {
        let location = Location::local(item.path.clone());
        if item.is_directory {
            search_preview.close();
            search_controller.navigate(location);
            return;
        }
        if let Some(parent) = item.path.parent() {
            search_controller.navigate(Location::local(parent));
        }
        if search_preferences.search_open_files_directly() {
            search_controller.open_location(location);
        } else {
            search_preview.show(FileEntry {
                location,
                native_name: item.path.file_name().unwrap_or_default().to_os_string(),
                display_name: item.name,
                kind: EntryKind::File,
                size: MetadataValue::Unknown,
                modified_unix_seconds: MetadataValue::Unknown,
                is_hidden: false,
                mode: MetadataValue::Unknown,
            });
        }
    });
    let dismissed_search_root = blurred_root.clone();
    let dismissed_search_button = search_button.clone();
    let dismiss_search = Rc::new(move || {
        dismissed_search_root.set_blurred(false);
        dismissed_search_button.remove_css_class("active");
    });
    let search_dialog = SearchDialog::new(activate_search_result, dismiss_search);
    window_overlay.add_overlay(&search_dialog.widget());
    let shown_search = search_dialog.clone();
    let search_blurred_root = blurred_root.clone();
    search_button.connect_clicked(move |button| {
        if shown_search.is_visible() {
            shown_search.hide();
            return;
        }
        let root = home_directory();
        button.add_css_class("active");
        search_blurred_root.set_blurred(true);
        shown_search.show(root);
    });
    let search_action = gio::SimpleAction::new("search", None);
    let shortcut_search = search_dialog.clone();
    let shortcut_search_button = search_button.clone();
    let shortcut_search_root = blurred_root.clone();
    search_action.connect_activate(move |_, _| {
        if shortcut_search.is_visible() {
            shortcut_search.hide();
        } else {
            let root = home_directory();
            shortcut_search_button.add_css_class("active");
            shortcut_search_root.set_blurred(true);
            shortcut_search.show(root);
        }
    });
    window.add_action(&search_action);
    application.set_accels_for_action("win.search", &["<Control>k"]);

    let terminal_view = browser.clone();
    let terminal_action = gio::SimpleAction::new("open-terminal", None);
    terminal_action.connect_activate(move |_, _| {
        terminal_view.open_terminal();
    });
    window.add_action(&terminal_action);
    application.set_accels_for_action("win.open-terminal", &["<Primary>t"]);

    let refresh_view = browser.clone();
    let refresh_action = gio::SimpleAction::new("refresh", None);
    refresh_action.connect_activate(move |_, _| {
        refresh_view.refresh();
    });
    window.add_action(&refresh_action);
    application.set_accels_for_action("win.refresh", &["F5", "<Primary>r"]);

    let update_button = sidebar.update_notice.clone();
    let update_area = sidebar.update_area.clone();
    let update_label = sidebar.update_label.clone();
    let available_update = Rc::new(RefCell::new(
        None::<(
            crate::services::ReleaseMetadata,
            String,
            crate::services::UpdateMethod,
        )>,
    ));
    // Process-wide, not per-window: shared across the settings page's
    // update/rollback rows, this dialog, and every other open window, so at
    // most one install ever runs at a time -- see
    // `settings::install_guard`.
    let install_guard = super::settings::install_guard();
    let available_for_click = available_update.clone();
    let update_parent = window.clone().upcast::<gtk::Window>();
    let install_guard_for_dialog = install_guard.clone();
    update_button.connect_clicked(move |_| {
        let Some((release, download_url, update_method)) = available_for_click.borrow().clone()
        else {
            return;
        };
        super::settings::show_update_dialog(
            &update_parent,
            &release,
            download_url,
            install_guard_for_dialog.clone(),
            update_method,
        );
    });
    let available_for_notice = available_update.clone();
    let update_notice: super::settings::UpdateNoticeHandler = Rc::new(move |release| {
        if let Some((release, download_url, update_method)) = release {
            let tooltip = match update_method {
                crate::services::UpdateMethod::InPlace => {
                    format!("Install Strata v{}", release.version)
                }
                crate::services::UpdateMethod::Aur => format!(
                    "Strata v{} is available through {}",
                    release.version,
                    crate::services::InstallSource::detect()
                        .managed()
                        .map(crate::services::ManagedInstall::manager)
                        .unwrap_or("your package manager")
                ),
                crate::services::UpdateMethod::Omarchy => {
                    format!("Strata v{} is available through Omarchy", release.version)
                }
                crate::services::UpdateMethod::Pacman => {
                    format!("Strata v{} is available through pacman", release.version)
                }
            };
            update_button.set_tooltip_text(Some(&tooltip));
            update_label.set_text(&sidebar_update_label(&release));
            if release.kind == BuildKind::Stable {
                update_button.remove_css_class("preview");
            } else {
                update_button.add_css_class("preview");
            }
            *available_for_notice.borrow_mut() = Some((release, download_url, update_method));
            update_area.set_visible(true);
        } else {
            available_for_notice.borrow_mut().take();
            update_area.set_visible(false);
        }
    });
    let settings_layer: Rc<RefCell<Option<gtk::Box>>> = Rc::new(RefCell::new(None));
    let ensure_settings_layer = {
        let browser = browser.clone();
        let settings_button = settings.clone();
        let blurred = blurred_root.clone();
        let themes = theme_manager.clone();
        let notice = update_notice.clone();
        let guard = install_guard.clone();
        let overlay = window_overlay.clone();
        let layers = settings_layer.clone();
        Rc::new(move || {
            if let Some(layer) = layers.borrow().clone() {
                return layer;
            }
            let layer = super::settings::build_layer(
                &browser,
                &settings_button,
                &blurred,
                themes.clone(),
                notice.clone(),
                guard.clone(),
            );
            overlay.add_overlay(&layer);
            layers.borrow_mut().replace(layer.clone());
            layer
        })
    };
    let shown_settings = ensure_settings_layer.clone();
    let settings_button = settings.clone();
    let settings_blurred_root = blurred_root.clone();
    settings.connect_clicked(move |_| {
        let layer = shown_settings();
        show_settings(&layer, &settings_button, &settings_blurred_root);
    });
    let settings_shortcut = gtk::EventControllerKey::new();
    let shown_settings = ensure_settings_layer.clone();
    let settings_button = settings.clone();
    let shortcut_blurred_root = blurred_root.clone();
    settings_shortcut.connect_key_pressed(move |_, key, _, modifiers| {
        if key != gtk::gdk::Key::comma || !modifiers.contains(gtk::gdk::ModifierType::CONTROL_MASK)
        {
            return glib::Propagation::Proceed;
        }
        let layer = shown_settings();
        show_settings(&layer, &settings_button, &shortcut_blurred_root);
        glib::Propagation::Stop
    });
    window.add_controller(settings_shortcut);
    window.set_child(Some(&window_overlay));
    let rename_cancel_view = browser.clone();
    let rename_cancel = gtk::GestureClick::new();
    rename_cancel.set_propagation_phase(gtk::PropagationPhase::Capture);
    rename_cancel.connect_pressed(move |gesture, _, x, y| {
        if !rename_cancel_view.rename_is_active() {
            return;
        }
        let on_entry = gesture
            .widget()
            .and_then(|widget| widget.pick(x, y, gtk::PickFlags::DEFAULT))
            .is_some_and(|target| {
                target.has_css_class("inline-rename")
                    || target.ancestor(gtk::Entry::static_type()).is_some()
            });
        if !on_entry {
            rename_cancel_view.cancel_rename();
        }
    });
    window.add_controller(rename_cancel);
    let location_cancel_view = browser.clone();
    let location_cancel = gtk::GestureClick::new();
    location_cancel.set_propagation_phase(gtk::PropagationPhase::Capture);
    location_cancel.connect_pressed(move |gesture, _, x, y| {
        if !location_cancel_view.location_edit_is_active() {
            return;
        }
        let on_location_edit = gesture
            .widget()
            .and_then(|widget| widget.pick(x, y, gtk::PickFlags::DEFAULT))
            .is_some_and(|target| location_cancel_view.location_edit_contains(&target));
        if !on_location_edit {
            location_cancel_view.cancel_location_edit();
        }
    });
    window.add_controller(location_cancel);
    install_modal_focus_trap(&window);
    let type_to_search = TypeToSearch {
        view: browser.clone(),
        preferences: theme_manager.clone(),
    };
    install_keyboard_navigation(
        &window,
        &browser,
        &sidebar,
        &sidebar_toggle,
        &preview,
        &type_to_search,
    );
    let browser_controller = browser.browser();
    schedule_after_first_paint(&window, &sidebar);
    window.connect_destroy(move |_| {
        browser_controller.clear_observer();
        sidebar.disconnect();
    });
    window.present();
    crate::metrics::mark_window_presented();
    let pending_location = location.unwrap_or_else(|| Location::local(home_directory()));
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
    schedule_due_update_check(&theme_manager, &update_notice);
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
    let target = if expanded { SIDEBAR_WIDTH } else { 0 };
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

fn install_keyboard_navigation(
    window: &gtk::ApplicationWindow,
    view: &BrowserView,
    sidebar: &SidebarView,
    sidebar_toggle: &gtk::ToggleButton,
    preview: &PreviewDrawer,
    type_to_search: &TypeToSearch,
) {
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let view = view.clone();
    let sidebar_state = sidebar.state.clone();
    let sidebar_widget = sidebar.widget.clone();
    let sidebar_toggle = sidebar_toggle.clone();
    let preview = preview.clone();
    let type_to_search = type_to_search.clone();
    let dialog_parent = window.clone();
    let focus_before_sidebar = Rc::new(RefCell::new(None::<gtk::Widget>));
    let weak_browser = Rc::downgrade(&view.browser());
    keys.connect_key_pressed(move |_, key, _, modifiers| {
        let Some(browser) = weak_browser.upgrade() else {
            return glib::Propagation::Proceed;
        };
        if let Some(layer) = visible_modal_layer(&dialog_parent) {
            let focus_is_inside = gtk::prelude::RootExt::focus(&dialog_parent)
                .is_some_and(|focus| focus == layer || focus.is_ancestor(&layer));
            if !focus_is_inside {
                layer.grab_focus();
                return glib::Propagation::Stop;
            }
            return glib::Propagation::Proceed;
        }
        if key == gtk::gdk::Key::Escape && super::scrolling::stop_autoscroll() {
            return glib::Propagation::Stop;
        }
        let alt = modifiers.contains(gtk::gdk::ModifierType::ALT_MASK);
        let control = modifiers.contains(gtk::gdk::ModifierType::CONTROL_MASK);
        let shift = modifiers.contains(gtk::gdk::ModifierType::SHIFT_MASK);
        let focused = gtk::prelude::RootExt::focus(&dialog_parent);
        let sidebar_has_focus = focused.as_ref().is_some_and(|focused| {
            focused == &sidebar_widget || focused.is_ancestor(&sidebar_widget)
        });
        if control && matches!(key, gtk::gdk::Key::k | gtk::gdk::Key::K) {
            if let Err(error) =
                gtk::prelude::WidgetExt::activate_action(&dialog_parent, "win.search", None)
            {
                tracing::warn!(%error, "unable to activate global search shortcut");
            }
            return glib::Propagation::Stop;
        }
        if key == gtk::gdk::Key::F2 && view.begin_rename() {
            return glib::Propagation::Stop;
        }
        if key == gtk::gdk::Key::Escape && view.cancel_new_entry() {
            return glib::Propagation::Stop;
        }
        if key == gtk::gdk::Key::Escape && view.cancel_rename() {
            return glib::Propagation::Stop;
        }
        if view.rename_is_active() || view.new_entry_is_active() {
            return glib::Propagation::Proceed;
        }
        if key == gtk::gdk::Key::Escape && view.dismiss_focused_filter() {
            return glib::Propagation::Stop;
        }
        if control
            && !shift
            && !alt
            && matches!(key, gtk::gdk::Key::f | gtk::gdk::Key::F)
            && view.show_filter()
        {
            return glib::Propagation::Stop;
        }
        if control && key == gtk::gdk::Key::l {
            view.begin_location_edit();
            return glib::Propagation::Stop;
        }
        let text_has_focus = focused.as_ref().is_some_and(|w| {
            w.is::<gtk::Text>() || w.is::<gtk::TextView>() || w.is::<gtk::Entry>()
        });
        if preview.has_video()
            && !text_has_focus
            && !alt
            && !control
            && !shift
            && matches!(
                key,
                gtk::gdk::Key::space
                    | gtk::gdk::Key::Up
                    | gtk::gdk::Key::Down
                    | gtk::gdk::Key::Left
                    | gtk::gdk::Key::Right
                    | gtk::gdk::Key::m
                    | gtk::gdk::Key::M
            )
        {
            preview.handle_video_key(key);
            return glib::Propagation::Stop;
        }
        if is_sidebar_focus_shortcut(key, modifiers) {
            view.keyboard_navigation();
            if sidebar_has_focus {
                let restored = focus_before_sidebar
                    .borrow_mut()
                    .take()
                    .is_some_and(|widget| widget.grab_focus());
                if !restored {
                    browser.focus_active();
                }
            } else {
                focus_before_sidebar.replace(focused.clone());
                if !sidebar_toggle.is_active() {
                    sidebar_toggle.set_active(true);
                }
                let sidebar = sidebar_state.clone();
                glib::idle_add_local_once(move || {
                    sidebar.focus_active_place();
                });
            }
            return glib::Propagation::Stop;
        }
        if control && !shift && matches!(key, gtk::gdk::Key::b | gtk::gdk::Key::B) {
            sidebar_toggle.set_active(!sidebar_toggle.is_active());
            return glib::Propagation::Stop;
        }
        if view.location_has_focus() {
            if key == gtk::gdk::Key::Escape {
                view.cancel_location_edit();
                return glib::Propagation::Stop;
            }
            return glib::Propagation::Proceed;
        }
        if !text_has_focus && is_undo_shortcut(key, modifiers) && view.undo_last_operation() {
            return glib::Propagation::Stop;
        }
        if view.item_view_has_focus()
            && let Some(query) = type_to_search_query(key, modifiers)
            && type_to_search.show(query)
        {
            return glib::Propagation::Stop;
        }
        if !text_has_focus && is_browser_navigation_key(key, modifiers) {
            view.keyboard_navigation();
        }
        if alt
            && !control
            && !shift
            && matches!(key, gtk::gdk::Key::Return | gtk::gdk::Key::KP_Enter)
            && view.show_focused_properties()
        {
            return glib::Propagation::Stop;
        }
        if control && shift && matches!(key, gtk::gdk::Key::n | gtk::gdk::Key::N) {
            view.create_new_folder();
            return glib::Propagation::Stop;
        }
        if control && !shift && key == gtk::gdk::Key::v {
            if view.filter_has_focus() {
                return glib::Propagation::Proceed;
            }
            view.paste();
            return glib::Propagation::Stop;
        }
        if control && !shift && key == gtk::gdk::Key::c {
            if view.filter_has_focus() {
                return glib::Propagation::Proceed;
            }
            if view.copy_selection() {
                return glib::Propagation::Stop;
            }
        }
        if control && !shift && matches!(key, gtk::gdk::Key::d | gtk::gdk::Key::D) {
            if view.filter_has_focus() {
                return glib::Propagation::Proceed;
            }
            if view.duplicate_selection() {
                return glib::Propagation::Stop;
            }
        }
        if control && !shift && key == gtk::gdk::Key::x {
            if view.filter_has_focus() {
                return glib::Propagation::Proceed;
            }
            if view.cut_selection() {
                return glib::Propagation::Stop;
            }
        }
        if control && !shift && key == gtk::gdk::Key::a {
            if view.filter_has_focus() {
                return glib::Propagation::Proceed;
            }
            view.select_all();
            return glib::Propagation::Stop;
        }
        if is_toggle_hidden_shortcut(key, modifiers) {
            browser.toggle_hidden();
            return glib::Propagation::Stop;
        }
        if is_open_terminal_shortcut(key, modifiers) {
            view.open_terminal();
            return glib::Propagation::Stop;
        }
        if is_refresh_shortcut(key, modifiers) {
            view.refresh();
            return glib::Propagation::Stop;
        }
        let column_popover = focused
            .as_ref()
            .and_then(|focused| focused.ancestor(gtk::Popover::static_type()))
            .and_downcast::<gtk::Popover>()
            .filter(|popover| popover.has_css_class("column-popover"));
        if let Some(popover) = column_popover
            && !control
            && !alt
            && let Some(direction) = vim_focus_direction(key)
        {
            popover.child_focus(direction);
            return glib::Propagation::Stop;
        }
        let mut header_left_boundary = false;
        if view.header_actions_have_focus() && !control && !alt {
            match key {
                gtk::gdk::Key::h | gtk::gdk::Key::Left => {
                    if view.move_header_focus(gtk::DirectionType::Left) {
                        return glib::Propagation::Stop;
                    }
                    header_left_boundary = true;
                }
                gtk::gdk::Key::l | gtk::gdk::Key::Right => {
                    view.move_header_focus(gtk::DirectionType::Right);
                    return glib::Propagation::Stop;
                }
                gtk::gdk::Key::j | gtk::gdk::Key::Down => {
                    view.focus_items_from_header();
                    return glib::Propagation::Stop;
                }
                _ => {}
            }
        }
        if sidebar_has_focus
            && !control
            && !alt
            && let Some(direction) = vim_focus_direction(key)
        {
            if direction == gtk::DirectionType::Right {
                focus_before_sidebar.borrow_mut().take();
                browser.focus_active();
            } else {
                sidebar_widget.child_focus(direction);
            }
            return glib::Propagation::Stop;
        }
        if key == gtk::gdk::Key::BackSpace
            && !control
            && !alt
            && view.dismiss_empty_focused_filter()
        {
            return glib::Propagation::Stop;
        }
        if key == gtk::gdk::Key::Delete && !view.filter_has_focus() && view.confirm_delete(shift) {
            return glib::Propagation::Stop;
        }
        if !control && !alt && !view.item_view_has_focus() && !header_left_boundary {
            return glib::Propagation::Proceed;
        }
        if !control
            && !alt
            && let Some(direction) = page_direction(key)
            && view.page_selection(direction)
        {
            return glib::Propagation::Stop;
        }
        if !control && !alt && matches!(key, gtk::gdk::Key::y | gtk::gdk::Key::Y) {
            view.copy_path();
            return glib::Propagation::Stop;
        }
        if !control && !alt && matches!(key, gtk::gdk::Key::p | gtk::gdk::Key::P) {
            view.pin_focused();
            return glib::Propagation::Stop;
        }
        if key == gtk::gdk::Key::space && !alt && !control {
            preview.toggle(browser.focused_entry());
            return glib::Propagation::Stop;
        }
        if key == gtk::gdk::Key::Escape && preview.is_open() {
            preview.close();
            return glib::Propagation::Stop;
        }
        if key == gtk::gdk::Key::BackSpace && !control && !alt {
            view.navigate_up();
            return glib::Propagation::Stop;
        }
        if shift && key == gtk::gdk::Key::Up {
            browser.extend_selection(-1);
            return glib::Propagation::Stop;
        }
        if shift && key == gtk::gdk::Key::Down {
            browser.extend_selection(1);
            return glib::Propagation::Stop;
        }
        if !shift
            && matches!(key, gtk::gdk::Key::k | gtk::gdk::Key::Up)
            && view.focus_header_from_top_item()
        {
            return glib::Propagation::Stop;
        }

        match (key, alt) {
            (gtk::gdk::Key::Left, true) => browser.back(),
            (gtk::gdk::Key::Right, true) => browser.forward(),
            (gtk::gdk::Key::Up, true) => browser.parent(),
            (gtk::gdk::Key::Home, true) => {
                browser.navigate(Location::local(home_directory()));
            }
            (gtk::gdk::Key::j | gtk::gdk::Key::Down, false) => browser.move_selection(1),
            (gtk::gdk::Key::k | gtk::gdk::Key::Up, false) => browser.move_selection(-1),
            (gtk::gdk::Key::h | gtk::gdk::Key::Left, false)
                if !control && view.first_column_has_focus() && sidebar_toggle.is_active() =>
            {
                focus_before_sidebar.replace(focused.clone());
                sidebar_state.focus_active_place();
            }
            (gtk::gdk::Key::h | gtk::gdk::Key::Left, false) => view.navigate_left(),
            (
                gtk::gdk::Key::l
                | gtk::gdk::Key::Right
                | gtk::gdk::Key::Return
                | gtk::gdk::Key::KP_Enter,
                false,
            ) => view.activate_focused(),
            (gtk::gdk::Key::Escape, false) => browser.escape(),
            _ => return glib::Propagation::Proceed,
        }
        glib::Propagation::Stop
    });
    window.add_controller(keys);
}

fn is_browser_navigation_key(key: gtk::gdk::Key, modifiers: gtk::gdk::ModifierType) -> bool {
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

fn is_undo_shortcut(key: gtk::gdk::Key, modifiers: gtk::gdk::ModifierType) -> bool {
    modifiers.contains(gtk::gdk::ModifierType::CONTROL_MASK)
        && !modifiers
            .intersects(gtk::gdk::ModifierType::SHIFT_MASK | gtk::gdk::ModifierType::ALT_MASK)
        && matches!(key, gtk::gdk::Key::z | gtk::gdk::Key::Z)
}

fn type_to_search_query(key: gtk::gdk::Key, modifiers: gtk::gdk::ModifierType) -> Option<char> {
    if modifiers.intersects(
        gtk::gdk::ModifierType::CONTROL_MASK
            | gtk::gdk::ModifierType::ALT_MASK
            | gtk::gdk::ModifierType::SUPER_MASK,
    ) {
        return None;
    }
    key.to_unicode().filter(|character| !character.is_control())
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

fn is_refresh_shortcut(key: gtk::gdk::Key, modifiers: gtk::gdk::ModifierType) -> bool {
    key == gtk::gdk::Key::F5
        || (modifiers.contains(gtk::gdk::ModifierType::CONTROL_MASK)
            && !modifiers.contains(gtk::gdk::ModifierType::ALT_MASK)
            && matches!(key, gtk::gdk::Key::r | gtk::gdk::Key::R))
}

fn is_sidebar_focus_shortcut(key: gtk::gdk::Key, modifiers: gtk::gdk::ModifierType) -> bool {
    modifiers.contains(gtk::gdk::ModifierType::CONTROL_MASK | gtk::gdk::ModifierType::SHIFT_MASK)
        && !modifiers.contains(gtk::gdk::ModifierType::ALT_MASK)
        && matches!(key, gtk::gdk::Key::b | gtk::gdk::Key::B)
}

fn vim_focus_direction(key: gtk::gdk::Key) -> Option<gtk::DirectionType> {
    match key {
        gtk::gdk::Key::h => Some(gtk::DirectionType::Left),
        gtk::gdk::Key::j => Some(gtk::DirectionType::Down),
        gtk::gdk::Key::k => Some(gtk::DirectionType::Up),
        gtk::gdk::Key::l => Some(gtk::DirectionType::Right),
        _ => None,
    }
}

fn visible_modal_layer(window: &gtk::ApplicationWindow) -> Option<gtk::Widget> {
    let overlay = window.child().and_downcast::<gtk::Overlay>()?;
    let mut child = overlay.first_child();
    while let Some(widget) = child {
        if widget.is_visible() && widget.has_css_class("app-modal-layer") {
            return Some(widget);
        }
        child = widget.next_sibling();
    }
    None
}

fn install_modal_focus_trap(window: &gtk::ApplicationWindow) {
    window.connect_focus_widget_notify(|window| {
        let Some(layer) = visible_modal_layer(window) else {
            return;
        };
        let focus_is_inside = gtk::prelude::RootExt::focus(window)
            .is_some_and(|focus| focus == layer || focus.is_ancestor(&layer));
        if !focus_is_inside {
            layer.grab_focus();
        }
    });
}

fn build_appearance_menu(
    view: &BrowserView,
    controller: &Rc<Browser>,
    preferences: Rc<super::theme::ThemeManager>,
) -> gtk::MenuButton {
    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.add_css_class("appearance-menu");
    let popover = gtk::Popover::builder()
        .has_arrow(false)
        .halign(gtk::Align::End)
        .position(gtk::PositionType::Bottom)
        .build();
    popover.add_css_class("appearance-popover");
    let button = gtk::MenuButton::builder()
        .tooltip_text("Appearance")
        .popover(&popover)
        .build();
    let popover_weak = popover.downgrade();
    append_menu_heading(&content, "VIEW");
    let current_mode = view.view_mode();
    let (list, list_check, _) = appearance_option(
        crate::assets::icons::LIST,
        "List",
        current_mode == BrowserMode::Columns,
        true,
    );
    let (grid, grid_check, _) = appearance_option(
        crate::assets::icons::GRID,
        "Grid",
        current_mode == BrowserMode::Grid,
        true,
    );
    let (explorer, explorer_check, _) = appearance_option(
        crate::assets::icons::ROWS,
        "Explorer",
        current_mode == BrowserMode::Explorer,
        true,
    );
    let grouped = preferences.group_by_type();
    let (group_by_type, group_check, _) = appearance_option(
        crate::assets::icons::LIST_CHECKS,
        "Group by file type",
        grouped,
        current_mode != BrowserMode::Columns,
    );
    group_by_type.set_tooltip_text(Some(
        "Group Explorer and Grid entries under file-type headings",
    ));
    {
        let view = view.clone();
        let preferences = preferences.clone();
        let popover_weak = popover_weak.clone();
        let grouped = Cell::new(grouped);
        group_by_type.connect_clicked(move |_| {
            let enabled = !grouped.get();
            grouped.set(enabled);
            view.set_group_by_type(enabled);
            preferences.set_group_by_type(enabled);
            group_check.set_visible(enabled);
            if let Some(popover) = popover_weak.upgrade() {
                popover.popdown();
            }
        });
    }
    for (button, mode) in [
        (&list, BrowserMode::Columns),
        (&grid, BrowserMode::Grid),
        (&explorer, BrowserMode::Explorer),
    ] {
        let view = view.clone();
        let list_check = list_check.clone();
        let grid_check = grid_check.clone();
        let explorer_check = explorer_check.clone();
        let group_by_type = group_by_type.clone();
        let preferences = preferences.clone();
        let popover_weak = popover_weak.clone();
        button.connect_clicked(move |_| {
            view.set_view_mode(mode);
            preferences.set_browser_mode(mode);
            list_check.set_visible(mode == BrowserMode::Columns);
            grid_check.set_visible(mode == BrowserMode::Grid);
            explorer_check.set_visible(mode == BrowserMode::Explorer);
            group_by_type.set_sensitive(mode != BrowserMode::Columns);
            if let Some(popover) = popover_weak.upgrade() {
                popover.popdown();
            }
        });
    }
    content.append(&list);
    content.append(&grid);
    content.append(&explorer);

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
    {
        let view = view.clone();
        let compact_check = compact_check.clone();
        let airy_check = airy_check.clone();
        let preferences = preferences.clone();
        let popover_weak = popover_weak.clone();
        compact.connect_clicked(move |_| {
            view.set_density(BrowserDensity::Compact);
            preferences.set_browser_density(BrowserDensity::Compact);
            compact_check.set_visible(true);
            airy_check.set_visible(false);
            if let Some(popover) = popover_weak.upgrade() {
                popover.popdown();
            }
        });
    }
    {
        let view = view.clone();
        let popover_weak = popover_weak.clone();
        airy.connect_clicked(move |_| {
            view.set_density(BrowserDensity::Airy);
            preferences.set_browser_density(BrowserDensity::Airy);
            compact_check.set_visible(false);
            airy_check.set_visible(true);
            if let Some(popover) = popover_weak.upgrade() {
                popover.popdown();
            }
        });
    }
    content.append(&compact);
    content.append(&airy);

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

    popover.set_child(Some(&content));
    let icon = crate::assets::primary_icon(crate::assets::icons::LIST, 20);
    button.set_child(Some(&icon));
    button.add_css_class("header-action");
    button.connect_active_notify(move |button| {
        crate::assets::set_primary_icon(
            &icon,
            if button.is_active() {
                crate::assets::icons::LIST_ACTIVE
            } else {
                crate::assets::icons::LIST
            },
        );
    });
    button
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
    let button = gtk::Button::builder()
        .child(&row)
        .sensitive(sensitive)
        .build();
    button.add_css_class("appearance-option");
    button.set_has_frame(false);
    (button, check, option)
}

fn show_settings(layer: &gtk::Box, button: &gtk::Button, root: &BlurBin) {
    root.set_blurred(true);
    layer.set_visible(true);
    layer.grab_focus();
    button.add_css_class("active");
    super::browser::animate_in(layer);
}

fn append_menu_heading(container: &gtk::Box, text: &str) {
    let heading = gtk::Label::new(Some(text));
    heading.set_xalign(0.0);
    heading.add_css_class("menu-heading");
    container.append(&heading);
}

struct SidebarState {
    widget: gtk::Box,
    view: BrowserView,
    browser: Rc<Browser>,
    volume_monitor: gio::VolumeMonitor,
    theme_manager: Rc<super::theme::ThemeManager>,
    place_order: RefCell<Vec<&'static str>>,
    pinned_places: Rc<RefCell<Vec<(Location, String)>>>,
    place_rows: RefCell<Vec<(Location, gtk::Button)>>,
}

struct SidebarView {
    widget: gtk::Widget,
    state: Rc<SidebarState>,
    update_notice: gtk::Button,
    update_area: gtk::Box,
    update_label: gtk::Label,
    handlers: RefCell<Vec<glib::SignalHandlerId>>,
}

impl SidebarView {
    fn disconnect(&self) {
        for handler in self.handlers.take() {
            self.state.volume_monitor.disconnect(handler);
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
        self.append_trash_place();
        self.append_place(
            crate::assets::icons::NETWORK,
            "Network",
            Location::uri("network:///"),
        );
        self.append_separator();

        for place in self.place_order.borrow().clone() {
            if let Some((icon, name, directory)) = standard_place(place)
                && let Some(path) = glib::user_special_dir(directory)
                    .filter(|path| should_show_standard_place(place, path, &home_directory()))
            {
                self.append_reorderable_place(place, icon, name, Location::local(path));
            }
        }

        let pinned = self
            .pinned_places
            .borrow()
            .iter()
            .enumerate()
            .filter(|(_, (location, _))| !is_standard_place_location(location))
            .map(|(index, (location, name))| (index, location.clone(), name.clone()))
            .collect::<Vec<_>>();
        if !pinned.is_empty() {
            self.append_separator();
            self.append_heading("PINNED");
            for (index, location, name) in pinned {
                self.append_pinned_place(index, &name, location);
            }
        }
    }

    fn append_devices(self: &Rc<Self>) {
        let volumes = self.volume_monitor.volumes();
        let mounts: Vec<_> = self
            .volume_monitor
            .mounts()
            .into_iter()
            .filter(|mount| !mount.is_shadowed() && mount.volume().is_none())
            .filter_map(|mount| {
                let name = mount.name().to_string();
                let location = location_for_file(&mount.root())?;
                Some((name, location, mount))
            })
            .collect();
        if !volumes.is_empty() || !mounts.is_empty() {
            self.append_separator();
            self.append_heading("DEVICES");
            for volume in volumes {
                self.append_volume(volume);
            }
            for (name, location, mount) in mounts {
                if is_smb_location(&location) {
                    self.append_smb_mount(&name, location, mount);
                } else {
                    let action = mount_release_action(mount.can_eject(), mount.can_unmount());
                    let release = action.map(|action| {
                        let release_mount = mount.clone();
                        let release_browser = self.browser.clone();
                        let release_parent = self.view.widget();
                        let release_in_flight = Rc::new(Cell::new(false));
                        let on_release: Rc<dyn Fn()> = Rc::new(move || {
                            release_device_mount(
                                &release_mount,
                                action,
                                &release_parent,
                                &release_browser,
                                &release_in_flight,
                            );
                        });
                        (action, on_release)
                    });
                    self.append_device_place(
                        crate::assets::icons::HARD_DRIVE,
                        &name,
                        location,
                        release,
                    );
                }
            }
        }
    }

    fn pin_location(self: &Rc<Self>, location: Location, name: String) {
        if pin_status(&self.pinned_places.borrow(), &location) != PinStatus::Available {
            return;
        }
        self.pinned_places.borrow_mut().push((location, name));
        save_pinned_places(&self.pinned_places.borrow());
        self.rebuild();
    }

    fn unpin_location(self: &Rc<Self>, location: &Location) {
        if remove_pinned_place(&mut self.pinned_places.borrow_mut(), location) {
            save_pinned_places(&self.pinned_places.borrow());
            self.rebuild();
        }
    }
    fn event_changes_active_place(event: &BrowserEvent) -> bool {
        matches!(
            event,
            BrowserEvent::Reset
                | BrowserEvent::ColumnAdded { .. }
                | BrowserEvent::ColumnsTruncated { .. }
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

    fn focus_active_place(&self) -> bool {
        let rows = self.place_rows.borrow();
        rows.iter()
            .find(|(_, row)| row.has_css_class("active"))
            .or_else(|| rows.first())
            .is_some_and(|(_, row)| row.grab_focus())
    }

    fn append_trash_place(self: &Rc<Self>) {
        let location = Location::uri("trash:///");
        let row = sidebar_button(crate::assets::icons::TRASH, "Trash");
        row.set_tooltip_text(Some("trash:///"));
        self.place_rows
            .borrow_mut()
            .push((location.clone(), row.clone()));
        let weak_browser = Rc::downgrade(&self.browser);
        let sidebar = self.widget.clone();
        let selected_row = row.clone();
        row.connect_clicked(move |_| {
            select_sidebar_row(&sidebar, &selected_row);
            if let Some(browser) = weak_browser.upgrade() {
                browser.navigate(location.clone());
            }
        });

        let menu = gtk::Box::new(gtk::Orientation::Vertical, 0);
        menu.add_css_class("folder-context-menu");
        let properties = sidebar_context_option(crate::assets::icons::INFO, "Properties", false);
        let empty = sidebar_context_option(crate::assets::icons::TRASH, "Empty Trash…", true);
        empty.add_css_class("danger");
        menu.append(&properties);
        menu.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        menu.append(&empty);
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
        self.widget.append(&row);
    }

    fn make_reorderable(
        self: &Rc<Self>,
        row: &gtk::Button,
        payload: impl Fn() -> String + 'static,
        on_drop: impl Fn(&Rc<Self>, &str, bool) -> bool + 'static,
    ) {
        row.add_css_class("reorderable");
        row.set_cursor_from_name(Some("grab"));

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
            dragged_row.set_cursor_from_name(Some("grab"));
        });
        row.add_controller(drag);

        let drop = gtk::DropTarget::new(String::static_type(), gtk::gdk::DragAction::MOVE);
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
        self.place_rows
            .borrow_mut()
            .push((location.clone(), row.clone()));
        let weak_browser = Rc::downgrade(&self.browser);
        let sidebar = self.widget.clone();
        let selected_row = row.clone();
        row.connect_clicked(move |_| {
            select_sidebar_row(&sidebar, &selected_row);
            if let Some(browser) = weak_browser.upgrade() {
                browser.navigate(location.clone());
            }
        });

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
        let changed =
            reorder_pinned_places(&mut self.pinned_places.borrow_mut(), source, target, after);
        if changed {
            save_pinned_places(&self.pinned_places.borrow());
            self.rebuild();
        }
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
            self.place_rows.borrow_mut().push((location, row.clone()));
        }
        let weak_browser = Rc::downgrade(&self.browser);
        let sidebar = self.widget.clone();
        let selected_row = row.clone();
        let clicked_volume = volume.clone();
        row.connect_clicked(move |button| {
            let volume = clicked_volume.clone();
            select_sidebar_row(&sidebar, &selected_row);
            let Some(browser) = weak_browser.upgrade() else {
                return;
            };
            if let Some(mount) = volume.get_mount() {
                navigate_to_gio_file(&browser, &mount.root());
                return;
            }

            let window = button.root().and_downcast::<gtk::Window>();
            let operation = gtk::MountOperation::new(window.as_ref());
            glib::MainContext::default().spawn_local(async move {
                match volume
                    .mount_future(gio::MountMountFlags::NONE, Some(&operation))
                    .await
                {
                    Ok(()) => {
                        if let Some(mount) = volume.get_mount() {
                            navigate_to_gio_file(&browser, &mount.root());
                        }
                    }
                    Err(error) => {
                        let dialog = gtk::AlertDialog::builder()
                            .modal(true)
                            .message("Unable to mount volume")
                            .detail(error.to_string())
                            .build();
                        dialog.show(window.as_ref());
                    }
                }
            });
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
        let menu = gtk::Box::new(gtk::Orientation::Vertical, 0);
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
        let menu = gtk::Box::new(gtk::Orientation::Vertical, 0);
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
        self.place_rows
            .borrow_mut()
            .push((location.clone(), row.clone()));
        let weak_browser = Rc::downgrade(&self.browser);
        let sidebar = self.widget.clone();
        let selected_row = row.clone();
        row.connect_clicked(move |_| {
            select_sidebar_row(&sidebar, &selected_row);
            if let Some(browser) = weak_browser.upgrade() {
                browser.navigate_location(location.clone());
            }
        });
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

fn sidebar_context_option(icon: &str, label: &str, danger: bool) -> gtk::Button {
    let button = gtk::Button::new();
    button.add_css_class("item-context-option");
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let icon = if danger {
        crate::assets::danger_icon(icon, 15)
    } else {
        crate::assets::primary_icon(icon, 15)
    };
    icon.add_css_class("item-context-icon");
    let label = gtk::Label::new(Some(label));
    label.set_xalign(0.0);
    label.set_hexpand(true);
    row.append(&icon);
    row.append(&label);
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
    let menu = gtk::Box::new(gtk::Orientation::Vertical, 0);
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

fn build_sidebar(view: BrowserView, theme_manager: Rc<super::theme::ThemeManager>) -> SidebarView {
    let widget = gtk::Box::new(gtk::Orientation::Vertical, 2);
    widget.add_css_class("sidebar");
    let scroller = gtk::ScrolledWindow::builder()
        .child(&widget)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .width_request(SIDEBAR_WIDTH)
        .vexpand(true)
        .build();
    scroller.add_css_class("sidebar-scroll");
    scroller.add_css_class("fixed-scrollbar");

    let update_content = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let dot = gtk::Label::new(Some("●"));
    dot.add_css_class("sidebar-update-dot");
    let update_label = gtk::Label::new(None);
    update_label.add_css_class("sidebar-update-label");
    update_label.set_xalign(0.0);
    update_label.set_hexpand(true);
    update_label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    update_content.append(&dot);
    update_content.append(&update_label);
    update_content.append(&crate::assets::primary_icon(
        crate::assets::icons::DOWNLOADS,
        17,
    ));
    let update_notice = gtk::Button::builder().child(&update_content).build();
    update_notice.add_css_class("sidebar-update");
    let update_separator = gtk::Separator::new(gtk::Orientation::Horizontal);
    update_separator.add_css_class("sidebar-separator");
    update_separator.add_css_class("sidebar-update-separator");
    let update_area = gtk::Box::new(gtk::Orientation::Vertical, 0);
    update_area.set_visible(false);
    update_area.append(&update_separator);
    update_area.append(&update_notice);

    let shell = gtk::Box::new(gtk::Orientation::Vertical, 0);
    shell.add_css_class("sidebar-shell");
    shell.append(&scroller);
    shell.append(&update_area);
    let volume_monitor = gio::VolumeMonitor::get();
    let place_order = resolve_place_order(&theme_manager.sidebar_order());
    let state = Rc::new(SidebarState {
        widget,
        browser: view.browser(),
        view,
        volume_monitor,
        theme_manager,
        place_order: RefCell::new(place_order),
        pinned_places: Rc::new(RefCell::new(load_pinned_places())),
        place_rows: RefCell::new(Vec::new()),
    });

    let weak = Rc::downgrade(&state);
    state.browser.observe(move |event| {
        if !SidebarState::event_changes_active_place(event) {
            return;
        }
        if let Some(state) = weak.upgrade() {
            state.sync_active_place();
        }
    });

    let mut handlers = Vec::new();
    let weak = Rc::downgrade(&state);
    handlers.push(state.volume_monitor.connect_mount_added(move |_, _| {
        if let Some(state) = weak.upgrade() {
            state.rebuild();
        }
    }));
    let weak = Rc::downgrade(&state);
    handlers.push(state.volume_monitor.connect_mount_removed(move |_, _| {
        if let Some(state) = weak.upgrade() {
            state.rebuild();
        }
    }));
    let weak = Rc::downgrade(&state);
    handlers.push(state.volume_monitor.connect_mount_changed(move |_, _| {
        if let Some(state) = weak.upgrade() {
            state.rebuild();
        }
    }));
    let weak = Rc::downgrade(&state);
    handlers.push(state.volume_monitor.connect_volume_added(move |_, _| {
        if let Some(state) = weak.upgrade() {
            state.rebuild();
        }
    }));
    let weak = Rc::downgrade(&state);
    handlers.push(state.volume_monitor.connect_volume_removed(move |_, _| {
        if let Some(state) = weak.upgrade() {
            state.rebuild();
        }
    }));
    let weak = Rc::downgrade(&state);
    handlers.push(state.volume_monitor.connect_volume_changed(move |_, _| {
        if let Some(state) = weak.upgrade() {
            state.rebuild();
        }
    }));
    state.append_static_places();
    state.sync_active_place();
    SidebarView {
        widget: shell.upcast(),
        state,
        update_notice,
        update_area,
        update_label,
        handlers: RefCell::new(handlers),
    }
}

fn pinned_places_path() -> PathBuf {
    glib::user_config_dir().join("gtk-3.0/bookmarks")
}

fn load_pinned_places() -> Vec<(Location, String)> {
    std::fs::read_to_string(pinned_places_path())
        .map(|contents| parse_pinned_places(&contents))
        .unwrap_or_default()
}

fn parse_pinned_places(contents: &str) -> Vec<(Location, String)> {
    let mut places = Vec::new();
    for line in contents.lines() {
        let (uri, label) = line
            .split_once(' ')
            .map_or((line, None), |(uri, label)| (uri, Some(label)));
        if uri.is_empty() {
            continue;
        }
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
            .map(str::to_owned)
            .unwrap_or_else(|| location.display_name());
        places.push((location, name));
    }
    places
}

fn save_pinned_places(places: &[(Location, String)]) {
    let path = pinned_places_path();
    let Some(parent) = path.parent() else {
        return;
    };
    if std::fs::create_dir_all(parent).is_err() {
        return;
    }
    let contents = serialize_pinned_places(places);
    let _result = crate::storage::atomic_write(&path, contents.as_bytes());
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

fn home_directory() -> PathBuf {
    env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

#[cfg(test)]
mod tests;

fn load_styles() {
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
