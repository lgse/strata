// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    cell::{Cell, RefCell},
    process::{Command, Stdio},
    rc::Rc,
    sync::mpsc::TryRecvError,
    time::Duration,
};

use gtk::{gdk, glib, prelude::*, subclass::prelude::*};

use crate::{
    assets::icons,
    sandbox::MediaPreviewBackend,
    services::{
        self, BuildKind, Channel, InstallRequest, InstallSource, ManagedInstall, ReleaseMetadata,
        ReleaseNoteBlock, ReleaseNotes, UpdateCheck, UpdateInstall, UpdateMethod, Version,
    },
};

#[cfg(test)]
mod tests;

use super::{
    blur::BlurBin,
    browser::{BrowserView, dismiss_modal_layer, modal_layer},
    browser_modes::{BrowserMode, ClickActivation, ClickCount},
    controls::{form_entry, menu_option, modal_layout, segmented_control},
    theme::{Theme, ThemeManager, ThemeTokens},
};

type ThemeCards = Rc<RefCell<Vec<(String, gtk::Button, gtk::Image)>>>;
pub(super) type UpdateNoticeHandler = Rc<dyn Fn(Option<(ReleaseMetadata, String, UpdateMethod)>)>;

struct UpdateCheckRow {
    row: gtk::Box,
    run_check: Rc<dyn Fn(bool)>,
    responsive_action: (gtk::Box, gtk::Button),
    install_underway: Rc<dyn Fn() -> bool>,
}

struct ResponsiveContent {
    flows: Vec<(gtk::FlowBox, u32)>,
    actions: Vec<(gtk::Box, gtk::Button)>,
    setting_rows: Vec<gtk::Box>,
    activation_rows: Vec<ResponsiveActivationRow>,
}

pub struct ResponsiveActivationRow {
    row: gtk::Box,
    options: Vec<gtk::Box>,
}

/// Shared "an install is running" guard across the update row and update
/// dialog, the two places [`services::install_update`] is called. Without one
/// process-wide guard, separate windows could replace the executable at the
/// same time. See [`start_install`].
pub(super) type InstallGuard = Rc<Cell<bool>>;

thread_local! {
    static INSTALL_GUARD: InstallGuard = Rc::new(Cell::new(false));
}

/// The one [`InstallGuard`] for this process.
///
/// Every install writes the *same* target -- the running executable -- so
/// the guard has to span every window, not just the controls within one. A
/// per-window guard would let installs started in separate windows replace
/// that executable concurrently, leaving the last writer as the installed
/// build.
///
/// A `thread_local` `Rc` (rather than a `Mutex`) is the whole story here
/// because every window is built on the single GTK main thread, from
/// `connect_activate`; this mirrors [`ThemeManager::shared`]. It is
/// deliberately never released: it is one `bool`, and the guard's
/// correctness should not depend on some window or in-flight install
/// happening to still hold a strong reference.
///
/// This bounds races to *this* process. A second `strata` process can
/// still install over the same executable, but `update_install` stages
/// into a unique path and lands it with a single atomic rename, so that
/// case degrades to last-writer-wins rather than a corrupted binary.
pub(super) fn install_guard() -> InstallGuard {
    INSTALL_GUARD.with(Rc::clone)
}

const DIALOG_WIDTH: i32 = 920;
const DIALOG_HEIGHT: i32 = 680;
const DIALOG_MARGIN: i32 = 24;
const COMPACT_NAVIGATION_BREAKPOINT: i32 = 700;

mod responsive_bin {
    use super::*;

    #[derive(Default)]
    pub struct ResponsiveBin {
        pub compact_navigation: Cell<bool>,
        pub navigation: RefCell<Option<gtk::Box>>,
        pub navigation_heading: RefCell<Option<gtk::Label>>,
        pub navigation_labels: RefCell<Vec<gtk::Label>>,
        pub navigation_contents: RefCell<Vec<gtk::Box>>,
        pub responsive_flows: RefCell<Vec<(gtk::FlowBox, u32)>>,
        pub responsive_actions: RefCell<Vec<(gtk::Box, gtk::Button)>>,
        pub responsive_setting_rows: RefCell<Vec<gtk::Box>>,
        pub responsive_activation_rows: RefCell<Vec<ResponsiveActivationRow>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ResponsiveBin {
        const NAME: &'static str = "StrataSettingsResponsiveBin";
        type Type = super::ResponsiveBin;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for ResponsiveBin {
        fn dispose(&self) {
            while let Some(child) = self.obj().first_child() {
                child.unparent();
            }
        }
    }

    impl WidgetImpl for ResponsiveBin {
        fn measure(&self, orientation: gtk::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            let natural = match orientation {
                gtk::Orientation::Horizontal => DIALOG_WIDTH + DIALOG_MARGIN * 2,
                gtk::Orientation::Vertical => DIALOG_HEIGHT + DIALOG_MARGIN * 2,
                _ => unreachable!("GTK orientations are horizontal or vertical"),
            };
            (1, natural, -1, -1)
        }

        fn size_allocate(&self, width: i32, height: i32, baseline: i32) {
            let Some(child) = self.obj().first_child() else {
                return;
            };
            let (child_width, child_height) = responsive_dialog_size(width, height);
            let compact = uses_compact_navigation(child_width);
            if self.compact_navigation.replace(compact) != compact {
                if let Some(navigation) = self.navigation.borrow().as_ref() {
                    if compact {
                        navigation.add_css_class("compact");
                    } else {
                        navigation.remove_css_class("compact");
                    }
                }
                if let Some(heading) = self.navigation_heading.borrow().as_ref() {
                    heading.set_visible(!compact);
                }
                for label in self.navigation_labels.borrow().iter() {
                    label.set_visible(!compact);
                }
                for content in self.navigation_contents.borrow().iter() {
                    content.set_halign(if compact {
                        gtk::Align::Center
                    } else {
                        gtk::Align::Fill
                    });
                }
                for (flow, expanded_columns) in self.responsive_flows.borrow().iter() {
                    flow.set_max_children_per_line(if compact { 1 } else { *expanded_columns });
                }
                for (row, action) in self.responsive_actions.borrow().iter() {
                    row.set_orientation(if compact {
                        gtk::Orientation::Vertical
                    } else {
                        gtk::Orientation::Horizontal
                    });
                    action.set_halign(gtk::Align::Fill);
                }
                for row in self.responsive_setting_rows.borrow().iter() {
                    row.set_orientation(if compact {
                        gtk::Orientation::Vertical
                    } else {
                        gtk::Orientation::Horizontal
                    });
                    row.set_spacing(if compact { 8 } else { 16 });
                }
                for responsive_row in self.responsive_activation_rows.borrow().iter() {
                    responsive_row.row.set_orientation(if compact {
                        gtk::Orientation::Vertical
                    } else {
                        gtk::Orientation::Horizontal
                    });
                    responsive_row.row.set_spacing(if compact { 4 } else { 12 });
                    for option in &responsive_row.options {
                        option.set_orientation(if compact {
                            gtk::Orientation::Vertical
                        } else {
                            gtk::Orientation::Horizontal
                        });
                        option.set_spacing(if compact { 2 } else { 6 });
                    }
                    if compact {
                        responsive_row.row.add_css_class("compact");
                    } else {
                        responsive_row.row.remove_css_class("compact");
                    }
                }
            }
            let x = ((width - child_width) / 2) as f32;
            let y = ((height - child_height) / 2) as f32;
            let transform = gtk::gsk::Transform::new().translate(&gtk::graphene::Point::new(x, y));
            child.allocate(child_width, child_height, baseline, Some(transform));
        }
    }
}

glib::wrapper! {
    pub struct ResponsiveBin(ObjectSubclass<responsive_bin::ResponsiveBin>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl ResponsiveBin {
    fn new(
        child: &impl IsA<gtk::Widget>,
        navigation: &gtk::Box,
        navigation_heading: &gtk::Label,
        navigation_labels: Vec<gtk::Label>,
        navigation_contents: Vec<gtk::Box>,
        responsive: ResponsiveContent,
    ) -> Self {
        let bin: Self = glib::Object::new();
        let imp = bin.imp();
        imp.navigation.replace(Some(navigation.clone()));
        imp.navigation_heading
            .replace(Some(navigation_heading.clone()));
        imp.navigation_labels.replace(navigation_labels);
        imp.navigation_contents.replace(navigation_contents);
        imp.responsive_flows.replace(responsive.flows);
        imp.responsive_actions.replace(responsive.actions);
        imp.responsive_setting_rows.replace(responsive.setting_rows);
        imp.responsive_activation_rows
            .replace(responsive.activation_rows);
        child.set_parent(&bin);
        bin
    }
}

fn responsive_dialog_size(width: i32, height: i32) -> (i32, i32) {
    (
        DIALOG_WIDTH.min((width - DIALOG_MARGIN * 2).max(1)),
        DIALOG_HEIGHT.min((height - DIALOG_MARGIN * 2).max(1)),
    )
}

fn uses_compact_navigation(dialog_width: i32) -> bool {
    dialog_width < COMPACT_NAVIGATION_BREAKPOINT
}

#[expect(
    deprecated,
    reason = "GTK 4.12 deprecated translate_coordinates and allocation without a replacement for click-in-bounds checks"
)]
pub fn build_layer(
    browser: &BrowserView,
    settings_button: &gtk::Button,
    root: &BlurBin,
    themes: Rc<ThemeManager>,
    update_notice: UpdateNoticeHandler,
    install_guard: InstallGuard,
) -> gtk::Box {
    let layer = gtk::Box::new(gtk::Orientation::Vertical, 0);
    layer.add_css_class("app-modal-layer");
    layer.add_css_class("settings-backdrop");
    layer.set_halign(gtk::Align::Fill);
    layer.set_valign(gtk::Align::Fill);
    layer.set_hexpand(true);
    layer.set_vexpand(true);
    layer.set_focusable(true);
    layer.set_visible(false);

    let panel = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    panel.add_css_class("settings-dialog");
    panel.set_overflow(gtk::Overflow::Hidden);

    let navigation = gtk::Box::new(gtk::Orientation::Vertical, 5);
    navigation.add_css_class("settings-navigation");
    let navigation_heading = append_heading(&navigation, "SETTINGS");

    let page = gtk::Box::new(gtk::Orientation::Vertical, 0);
    page.add_css_class("settings-page");
    page.set_hexpand(true);
    let titlebar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    titlebar.add_css_class("settings-titlebar");
    let title = gtk::Label::new(Some("General"));
    title.set_xalign(0.0);
    title.set_hexpand(true);
    title.add_css_class("settings-title");
    let close = gtk::Button::builder()
        .tooltip_text("Close settings")
        .build();
    close.set_child(Some(&crate::assets::primary_icon(icons::X, 18)));
    close.add_css_class("settings-close");
    close.set_valign(gtk::Align::Center);
    titlebar.append(&title);
    titlebar.append(&close);
    page.append(&titlebar);

    let stack = gtk::Stack::builder()
        .transition_type(gtk::StackTransitionType::Crossfade)
        .transition_duration(120)
        .hhomogeneous(false)
        .vhomogeneous(false)
        .hexpand(true)
        .vexpand(true)
        .build();
    let (general, responsive_setting_rows, responsive_activation_rows) =
        general_page(browser, themes.clone());
    stack.add_named(&general, Some("general"));
    let (updates, responsive_actions) = updates_page(themes.clone(), update_notice, install_guard);
    stack.add_named(&updates, Some("updates"));
    stack.add_named(&keybindings_page(), Some("keybindings"));
    let (theme_page, responsive_flows) = theme_page(themes);
    stack.add_named(&theme_page, Some("theme"));
    stack.add_named(&about_page(), Some("about"));
    page.append(&stack);

    let nav_buttons: Rc<RefCell<Vec<gtk::Button>>> = Rc::new(RefCell::new(Vec::new()));
    let mut navigation_labels = Vec::new();
    let mut navigation_contents = Vec::new();
    for (label, icon, name) in [
        ("General", icons::SLIDERS, "general"),
        ("Keybindings", icons::KEYBOARD, "keybindings"),
        ("Theme & appearance", icons::PALETTE, "theme"),
        ("Updates", icons::DOWNLOADS, "updates"),
        ("About", icons::INFO, "about"),
    ] {
        let active = name == "general";
        let (button, navigation_label, navigation_content) = navigation_button(icon, label);
        navigation_labels.push(navigation_label);
        navigation_contents.push(navigation_content);
        if active {
            button.add_css_class("settings-nav-active");
        }
        nav_buttons.borrow_mut().push(button.clone());
        let buttons = nav_buttons.clone();
        let stack = stack.clone();
        let title = title.clone();
        let page_title = label.to_owned();
        button.connect_clicked(move |clicked| {
            for candidate in buttons.borrow().iter() {
                if candidate == clicked {
                    candidate.add_css_class("settings-nav-active");
                } else {
                    candidate.remove_css_class("settings-nav-active");
                }
            }
            stack.set_visible_child_name(name);
            title.set_text(&page_title);
        });
        navigation.append(&button);
    }

    panel.append(&navigation);
    panel.append(&page);
    let responsive_panel = ResponsiveBin::new(
        &panel,
        &navigation,
        &navigation_heading,
        navigation_labels,
        navigation_contents,
        ResponsiveContent {
            flows: responsive_flows,
            actions: responsive_actions,
            setting_rows: responsive_setting_rows,
            activation_rows: responsive_activation_rows,
        },
    );
    responsive_panel.set_hexpand(false);
    responsive_panel.set_vexpand(false);
    let top = gtk::Box::new(gtk::Orientation::Vertical, 0);
    top.set_vexpand(true);
    let bottom = gtk::Box::new(gtk::Orientation::Vertical, 0);
    bottom.set_vexpand(true);
    let left = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    left.set_hexpand(true);
    let right = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    right.set_hexpand(true);
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    row.append(&left);
    row.append(&responsive_panel);
    row.append(&right);
    layer.append(&top);
    layer.append(&row);
    layer.append(&bottom);

    let hidden_layer = layer.clone();
    let inactive_settings = settings_button.clone();
    let unblurred_root = root.clone();
    close.connect_clicked(move |_| hide(&hidden_layer, &inactive_settings, &unblurred_root));
    let hidden_layer = layer.clone();
    let inactive_settings = settings_button.clone();
    let unblurred_root = root.clone();
    let keys = gtk::EventControllerKey::new();
    keys.connect_key_pressed(move |_, key, _, _| {
        if key != gdk::Key::Escape {
            return gtk::glib::Propagation::Proceed;
        }
        hide(&hidden_layer, &inactive_settings, &unblurred_root);
        gtk::glib::Propagation::Stop
    });
    layer.add_controller(keys);

    let click_layer = layer.clone();
    let click_dialog = responsive_panel.clone();
    let click_settings = settings_button.clone();
    let click_root = root.clone();
    let click = gtk::GestureClick::new();
    click.connect_pressed(move |_, _, x, y| {
        let on_dialog = click_dialog
            .translate_coordinates(&click_layer, 0.0, 0.0)
            .is_some_and(|(dx, dy)| {
                let alloc = click_dialog.allocation();
                x >= dx
                    && x < dx + alloc.width() as f64
                    && y >= dy
                    && y < dy + alloc.height() as f64
            });
        if !on_dialog {
            hide(&click_layer, &click_settings, &click_root);
        }
    });
    layer.add_controller(click);
    layer
}

fn hide(layer: &gtk::Box, button: &gtk::Button, root: &BlurBin) {
    if layer.has_css_class("dismissing") {
        return;
    }
    layer.add_css_class("dismissing");
    layer.set_sensitive(false);
    let layer_for_anim = layer.clone();
    let layer = layer.clone();
    let root = root.clone();
    let button = button.clone();
    super::browser::animate_out(&layer_for_anim, move || {
        layer.set_visible(false);
        layer.remove_css_class("dismissing");
        layer.set_sensitive(true);
        root.set_blurred(false);
        button.remove_css_class("active");
    });
}

fn general_page(
    browser: &BrowserView,
    manager: Rc<ThemeManager>,
) -> (gtk::Widget, Vec<gtk::Box>, Vec<ResponsiveActivationRow>) {
    let preferences = page_content();
    append_heading(&preferences, "BROWSING");
    let peeking_enabled = manager.folder_peeking();
    browser.set_peek_enabled(peeking_enabled);
    let (peeking_row, peeking) = settings_option(
        "Folder peeking",
        "Preview folders automatically while moving through a pane.",
        peeking_enabled,
    );
    let browser_for_peeking = browser.clone();
    let manager_for_peeking = manager.clone();
    peeking.connect_active_notify(move |toggle| {
        let enabled = toggle.is_active();
        browser_for_peeking.set_peek_enabled(enabled);
        manager_for_peeking.set_folder_peeking(enabled);
    });
    preferences.append(&peeking_row);

    let single_click_enabled = manager.single_click_previews();
    browser.set_single_click_previews(single_click_enabled);
    let (preview_row, single_click_previews) = settings_option(
        "Single-click file previews",
        "Show a quick preview when selecting a supported file.",
        single_click_enabled,
    );
    let browser_for_previews = browser.clone();
    let manager_for_previews = manager.clone();
    single_click_previews.connect_active_notify(move |toggle| {
        let enabled = toggle.is_active();
        browser_for_previews.set_single_click_previews(enabled);
        manager_for_previews.set_single_click_previews(enabled);
    });
    preferences.append(&preview_row);

    let direct_open_enabled = manager.search_open_files_directly();
    let (search_open_row, search_open_files) = settings_option(
        "Open search results directly",
        "Launch files from search instead of opening Strata's quick preview.",
        direct_open_enabled,
    );
    let manager_for_search_open = manager.clone();
    search_open_files.connect_active_notify(move |toggle| {
        manager_for_search_open.set_search_open_files_directly(toggle.is_active());
    });
    preferences.append(&search_open_row);

    append_heading(&preferences, "REFRESH");
    let interval = manager.auto_refresh_interval();
    let options = ["Off", "1 min", "5 min", "10 min"];
    let secs = [0, 60, 300, 600];
    let active = secs.iter().position(|&s| s == interval).unwrap_or(0);
    let (control, buttons) = segmented_control(&options, active);
    let refresh_row = gtk::Box::new(gtk::Orientation::Vertical, 8);
    refresh_row.add_css_class("settings-option");
    let refresh_copy = gtk::Box::new(gtk::Orientation::Vertical, 2);
    refresh_copy.set_hexpand(true);
    let refresh_title = gtk::Label::new(Some("Auto-refresh interval"));
    refresh_title.set_xalign(0.0);
    refresh_title.add_css_class("settings-option-title");
    let refresh_desc = gtk::Label::new(Some(
        "Automatically reload the current folder. Useful for network shares where file monitors may miss changes.",
    ));
    refresh_desc.set_xalign(0.0);
    refresh_desc.set_wrap(true);
    refresh_desc.add_css_class("settings-option-description");
    refresh_copy.append(&refresh_title);
    refresh_copy.append(&refresh_desc);
    refresh_row.append(&refresh_copy);
    refresh_row.append(&control);
    preferences.append(&refresh_row);
    for (idx, button) in buttons.iter().enumerate() {
        let manager = manager.clone();
        let browser = browser.clone();
        let secs = secs[idx];
        button.connect_toggled(move |toggled| {
            if toggled.is_active() {
                manager.set_auto_refresh_interval(secs);
                browser.set_auto_refresh_interval(secs);
            }
        });
    }

    append_heading(&preferences, "VIDEO PREVIEWS");
    let (acceleration_active, acceleration_sensitive, backend_sensitive) =
        video_preview_control_state(manager.hardware_accelerated_video_previews());
    let description = "Choose a hardware backend.";
    let selected_backend = manager.video_preview_backend();
    let manager_for_backend = manager.clone();
    let (video_row, acceleration, backend) = video_preview_option(
        description,
        acceleration_active,
        acceleration_sensitive,
        backend_sensitive,
        selected_backend,
        Rc::new(move |backend| manager_for_backend.set_video_preview_backend(backend)),
    );
    let manager_for_acceleration = manager.clone();
    let backend_for_acceleration = backend.clone();
    acceleration.connect_active_notify(move |toggle| {
        let enabled = toggle.is_active();
        backend_for_acceleration.set_sensitive(enabled);
        manager_for_acceleration.set_hardware_accelerated_video_previews(enabled);
    });
    preferences.append(&video_row);

    append_heading(&preferences, "MOTION");
    let (motion_row, reduce_motion) = settings_option(
        "Reduce motion",
        "Disable nonessential interface animations.",
        manager.reduce_motion(),
    );
    let manager_for_motion = manager.clone();
    reduce_motion.connect_active_notify(move |toggle| {
        manager_for_motion.set_reduce_motion(toggle.is_active());
    });
    preferences.append(&motion_row);

    append_heading(&preferences, "CLICK ACTIVATION");
    let activation_options = gtk::Box::new(gtk::Orientation::Vertical, 4);
    let mut responsive_activation_rows = Vec::new();
    activation_options.add_css_class("settings-option");
    activation_options.add_css_class("click-activation-options");
    for (label, mode) in [
        ("List", BrowserMode::Columns),
        ("Grid", BrowserMode::Grid),
        ("Explorer", BrowserMode::Explorer),
    ] {
        let activation = manager.click_activation(mode);
        browser.set_click_activation(mode, activation);
        let (row, options, file_buttons, folder_buttons) =
            click_activation_option(label, activation);
        let update = Rc::new({
            let browser = browser.clone();
            let manager = manager.clone();
            move |files, folders| {
                let activation = ClickActivation { files, folders };
                browser.set_click_activation(mode, activation);
                manager.set_click_activation(mode, activation);
            }
        });
        connect_click_activation_buttons(&file_buttons, &folder_buttons, update.clone());
        connect_click_activation_buttons(
            &folder_buttons,
            &file_buttons,
            Rc::new(move |folders, files| {
                update(files, folders);
            }),
        );
        activation_options.append(&row);
        responsive_activation_rows.push(ResponsiveActivationRow { row, options });
    }
    preferences.append(&activation_options);

    (
        scrollable_page(&preferences, None),
        vec![video_row],
        responsive_activation_rows,
    )
}

fn updates_page(
    manager: Rc<ThemeManager>,
    update_notice: UpdateNoticeHandler,
    install_guard: InstallGuard,
) -> (gtk::Widget, Vec<(gtk::Box, gtk::Button)>) {
    let preferences = page_content();
    append_heading(&preferences, "UPDATE PREFERENCES");
    let managed = InstallSource::detect().managed();
    if let Some(managed) = managed {
        preferences.append(&managed_install_row(managed));
    }

    let available_notes = release_notes_card(
        "Available release",
        "Check for updates to see the latest release notes.",
    );
    let update_method = services::update_method();
    let UpdateCheckRow {
        row: update_row,
        run_check,
        responsive_action,
        install_underway,
    } = update_check_row(
        manager.clone(),
        update_notice.clone(),
        available_notes.clone(),
        install_guard.clone(),
        update_method,
    );

    let auto_check_enabled = manager.checks_for_updates();
    let (auto_check_row, auto_check) = settings_option(
        "Automatically check for updates",
        match update_method {
            UpdateMethod::InPlace => "Check GitHub for a newer release when Strata starts.",
            UpdateMethod::Aur => "Check the AUR for a newer packaged release when Strata starts.",
            UpdateMethod::Omarchy => {
                "Check the Omarchy package repository for a newer release when Strata starts."
            }
            UpdateMethod::Pacman => {
                "Check the configured package repositories for a newer release when Strata starts."
            }
        },
        auto_check_enabled,
    );
    preferences.append(&auto_check_row);

    let (channel_row, sync_channel_selection) = channel_option(manager.clone(), managed);
    channel_row.set_sensitive(auto_check_enabled);
    channel_row.set_visible(managed.is_some() || !update_method.is_package_managed());
    preferences.append(&channel_row);
    preferences.append(&update_row);

    append_heading(&preferences, "RELEASE NOTES");
    let current_notes = release_notes_card(
        &format!(
            "Current release · v{}",
            crate::build_info::installed_version()
        ),
        "Loading release notes…",
    );
    preferences.append(&current_notes.container);
    load_current_release_notes(&current_notes);

    let manager_for_updates = manager.clone();
    let toggled_check = run_check.clone();
    auto_check.connect_active_notify(move |toggle| {
        let enabled = toggle.is_active();
        manager_for_updates.set_checks_for_updates(enabled);
        channel_row.set_sensitive(enabled);
        if enabled {
            toggled_check(false);
        } else {
            update_notice(None);
        }
    });
    if auto_check_enabled {
        run_check(false);
    }

    let page = scrollable_page(&preferences, None);
    let broadcast_check = run_check.clone();
    manager.on_release_channel_changed(
        &page,
        Rc::new(move || {
            sync_channel_selection();
            if !install_underway() {
                broadcast_check(false);
            }
        }),
    );
    (page, vec![responsive_action])
}

const RELEASE_CHANNEL_TITLE: &str = "Release channel";
const RELEASE_CHANNEL_DESCRIPTION: &str = "Preview receives alpha, beta, and release-candidate builds. Nightly also receives daily development builds.";

fn channel_option(
    manager: Rc<ThemeManager>,
    managed: Option<&ManagedInstall>,
) -> (gtk::Box, Rc<dyn Fn()>) {
    let row = gtk::Box::new(gtk::Orientation::Vertical, 12);
    row.add_css_class("settings-option");

    let copy = gtk::Box::new(gtk::Orientation::Vertical, 2);
    let title = gtk::Label::new(Some(RELEASE_CHANNEL_TITLE));
    title.set_xalign(0.0);
    title.add_css_class("settings-option-title");
    let locked_channel = managed.and_then(ManagedInstall::tracked_channel);
    if let Some(channel) = locked_channel {
        manager.set_release_channel(channel);
    }
    let description = gtk::Label::new(Some(&match managed {
        Some(managed) => managed_channel_description(managed),
        None => RELEASE_CHANNEL_DESCRIPTION.to_owned(),
    }));
    description.set_xalign(0.0);
    description.set_wrap(true);
    description.add_css_class("settings-option-description");
    copy.append(&title);
    copy.append(&description);
    row.append(&copy);

    let (control, buttons) = segmented_control(
        &["Stable", "Preview", "Nightly"],
        channel_index(manager.release_channel()),
    );
    let weak_buttons: Vec<glib::WeakRef<gtk::ToggleButton>> =
        buttons.iter().map(|button| button.downgrade()).collect();
    for (button, channel) in buttons.into_iter().zip(CHANNEL_ORDER) {
        let manager = manager.clone();
        button.connect_active_notify(move |button| {
            if button.is_active() {
                manager.set_release_channel(channel);
            }
        });
    }
    control.set_sensitive(managed.is_none());
    row.append(&control);

    let sync = {
        let manager = manager.clone();
        Rc::new(move || {
            let Some(button) = weak_buttons
                .get(channel_index(manager.release_channel()))
                .and_then(glib::WeakRef::upgrade)
            else {
                return;
            };
            if !button.is_active() {
                button.set_active(true);
            }
        }) as Rc<dyn Fn()>
    };

    (row, sync)
}

const CHANNEL_ORDER: [Channel; 3] = [Channel::Stable, Channel::Preview, Channel::Nightly];

fn channel_index(channel: Channel) -> usize {
    match channel {
        Channel::Stable => 0,
        Channel::Preview => 1,
        Channel::Nightly => 2,
    }
}

fn managed_channel_description(managed: &ManagedInstall) -> String {
    let tracked = match managed.channel() {
        Some(channel) => format!("This install tracks the {channel} release channel."),
        None => "The installed package decides the release channel.".to_owned(),
    };
    match managed.alternate_instruction() {
        Some(alternate) => format!("{tracked} {alternate}"),
        None => tracked,
    }
}

fn release_notes_label() -> gtk::Label {
    let label = gtk::Label::new(None);
    label.add_css_class("release-notes-content");
    label.set_xalign(0.0);
    label.set_yalign(0.0);
    label.set_hexpand(true);
    label.set_wrap(true);
    label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    label.set_selectable(true);
    label.set_use_markup(true);
    label
}

fn clear_release_notes(notes: &gtk::Box) {
    while let Some(child) = notes.first_child() {
        notes.remove(&child);
    }
}

fn set_release_notes_message(notes: &gtk::Box, message: &str) {
    clear_release_notes(notes);
    let label = release_notes_label();
    label.set_text(message);
    notes.append(&label);
}

fn set_release_note_blocks(notes: &gtk::Box, blocks: &[ReleaseNoteBlock]) {
    clear_release_notes(notes);
    for block in blocks {
        match block {
            ReleaseNoteBlock::Heading { level, markup } => {
                let label = release_notes_label();
                label.add_css_class("release-notes-heading");
                label.add_css_class(&format!("level-{level}"));
                label.set_markup(markup);
                notes.append(&label);
            }
            ReleaseNoteBlock::Paragraph(markup) => {
                let label = release_notes_label();
                label.set_markup(markup);
                notes.append(&label);
            }
            ReleaseNoteBlock::ListItem {
                marker,
                depth,
                markup,
            } => {
                let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                row.set_valign(gtk::Align::Start);
                row.set_margin_start(i32::try_from(depth.saturating_mul(18)).unwrap_or(i32::MAX));
                let bullet = gtk::Label::new(Some(marker));
                bullet.add_css_class("release-notes-bullet");
                bullet.set_valign(gtk::Align::Start);
                let copy = release_notes_label();
                copy.set_markup(markup);
                row.append(&bullet);
                row.append(&copy);
                notes.append(&row);
            }
            ReleaseNoteBlock::Code(markup) => {
                let label = release_notes_label();
                label.add_css_class("release-notes-code");
                label.set_markup(&format!("<tt>{markup}</tt>"));
                notes.append(&label);
            }
            ReleaseNoteBlock::Rule => {
                let separator = gtk::Separator::new(gtk::Orientation::Horizontal);
                separator.add_css_class("release-notes-rule");
                notes.append(&separator);
            }
        }
    }
}

#[derive(Clone)]
struct ReleaseNotesCard {
    container: gtk::Box,
    title: gtk::Label,
    badge: gtk::Label,
    notes: gtk::Box,
    fallback: gtk::LinkButton,
}

fn release_notes_card(title: &str, initial: &str) -> ReleaseNotesCard {
    let container = gtk::Box::new(gtk::Orientation::Vertical, 8);
    container.add_css_class("release-notes-card");
    let title_label = gtk::Label::new(Some(title));
    title_label.add_css_class("release-notes-title");
    title_label.set_xalign(0.0);
    title_label.set_wrap(true);
    let badge = gtk::Label::new(None);
    badge.add_css_class("prerelease-badge");
    badge.set_xalign(0.0);
    badge.set_halign(gtk::Align::Start);
    badge.set_visible(false);
    let notes = gtk::Box::new(gtk::Orientation::Vertical, 6);
    set_release_notes_message(&notes, initial);
    let fallback =
        gtk::LinkButton::with_label("https://github.com/lgse/strata/releases", "View on GitHub");
    fallback.add_css_class("release-notes-fallback");
    fallback.set_halign(gtk::Align::Start);
    fallback.set_visible(false);
    container.append(&title_label);
    container.append(&badge);
    container.append(&notes);
    container.append(&fallback);
    ReleaseNotesCard {
        container,
        title: title_label,
        badge,
        notes,
        fallback,
    }
}

/// Shows `release`'s notes in `card`, including a visible prerelease badge
/// above the notes whenever `release.kind` is not [`BuildKind::Stable`] --
/// release notes and update surfaces must visibly label prerelease software.
fn show_release_notes(card: &ReleaseNotesCard, release: &ReleaseMetadata) {
    card.container.set_visible(true);
    card.title.set_text(&format!(
        "{} · v{}",
        card.title
            .text()
            .split('·')
            .next()
            .unwrap_or("Release")
            .trim(),
        release.version
    ));
    if release.kind == BuildKind::Stable {
        card.badge.set_visible(false);
    } else {
        card.badge.set_text(release.kind.label());
        card.badge.set_visible(true);
    }
    if release.notes.trim().is_empty() {
        set_release_notes_message(
            &card.notes,
            "No release notes were provided for this release.",
        );
    } else {
        set_release_note_blocks(&card.notes, &release.note_blocks);
    }
    card.fallback.set_uri(&release.url);
    card.fallback.set_visible(true);
}

fn load_current_release_notes(card: &ReleaseNotesCard) {
    let receiver = services::fetch_release_notes(crate::build_info::RELEASE_TAG);
    let card = card.clone();
    glib::timeout_add_local(Duration::from_millis(100), move || {
        match receiver.try_recv() {
            Ok(ReleaseNotes::Found(release)) => {
                show_release_notes(&card, &release);
                glib::ControlFlow::Break
            }
            Ok(ReleaseNotes::Unavailable { url }) => {
                set_release_notes_message(
                    &card.notes,
                    "Release notes are unavailable because this version’s tag was not found.",
                );
                card.fallback.set_uri(&url);
                card.fallback.set_visible(true);
                glib::ControlFlow::Break
            }
            Ok(ReleaseNotes::Failed { message, url }) => {
                set_release_notes_message(
                    &card.notes,
                    &format!("Couldn’t load release notes: {message}"),
                );
                card.fallback.set_uri(&url);
                card.fallback.set_visible(true);
                glib::ControlFlow::Break
            }
            Err(TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(TryRecvError::Disconnected) => {
                set_release_notes_message(
                    &card.notes,
                    "Couldn’t load release notes because the request ended unexpectedly.",
                );
                glib::ControlFlow::Break
            }
        }
    });
}

/// Whether a check's result -- issued under `result_generation` -- has been
/// superseded by a newer check, whose generation is `current_generation`.
///
/// A toggle mid-check must start a fresh check rather than being silently
/// dropped (see `run_check`'s doc comment), which means an older check's
/// result can still land after a newer one has already started or even
/// finished. Applying that stale result regardless is exactly how a Preview
/// fetch in flight when the user flips back to Stable could still offer an
/// RC to a Stable user: the result carries no channel of its own, so
/// nothing but generation order distinguishes it from a current one.
fn managed_install_row(managed: &ManagedInstall) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Vertical, 2);
    row.add_css_class("settings-option");
    let title = gtk::Label::new(Some("Package-managed installation"));
    title.set_xalign(0.0);
    title.add_css_class("settings-option-title");
    let description = gtk::Label::new(Some(&managed_install_summary(managed)));
    description.set_xalign(0.0);
    description.set_wrap(true);
    description.set_selectable(true);
    description.add_css_class("settings-option-description");
    row.append(&title);
    row.append(&description);
    row
}

fn managed_install_summary(managed: &ManagedInstall) -> String {
    let mut lines = vec![managed.ownership_summary()];
    if let Some(channel) = managed.channel() {
        lines.push(format!("Tracking the {channel} release channel."));
    }
    lines.push(managed.update_instruction());
    lines.extend(managed.alternate_instruction());
    lines.join("\n")
}

fn is_stale_check(result_generation: u64, current_generation: u64) -> bool {
    result_generation != current_generation
}

/// An update that a completed check offered and that the next click will
/// install, held with the [`BuildKind`] it was offered as.
///
/// The kind is what lets the click re-test the offer against the channel
/// preference in force *then* rather than when the check ran -- see
/// [`offer_still_eligible`]. It is kept here, beside the request, rather
/// than on [`InstallRequest`], which deliberately carries nothing but the
/// URL the installer actually uses.
struct PendingInstall {
    kind: BuildKind,
    returns_to_stable: bool,
    request: InstallRequest,
}

/// Whether an offer for a `kind` build may still be installed by a user now
/// on `channel`.
///
/// `is_stale_check` only covers a check whose *result* has not landed yet,
/// and only within the one row that started it. This covers the other half:
/// an offer that already landed and is sitting in a window's "Install
/// update" button or an open update dialog. The channel preference is
/// process-wide ([`ThemeManager::shared`]), so switching back to Stable in
/// one window leaves every other window holding a cached RC offer it would
/// otherwise happily install. Re-testing at the moment of the click is what
/// makes the preference authoritative regardless of how many views cached
/// an offer under the old one.
fn effective_update_channel(selected: Channel, update_method: UpdateMethod) -> Channel {
    match update_method {
        UpdateMethod::InPlace | UpdateMethod::Aur => selected,
        UpdateMethod::Omarchy | UpdateMethod::Pacman => Channel::Stable,
    }
}

fn offer_still_eligible(channel: Channel, kind: BuildKind) -> bool {
    match channel {
        Channel::Stable => kind == BuildKind::Stable,
        Channel::Preview => kind != BuildKind::Nightly,
        Channel::Nightly => true,
    }
}

fn update_check_row(
    manager: Rc<ThemeManager>,
    update_notice: UpdateNoticeHandler,
    available_notes: ReleaseNotesCard,
    install_guard: InstallGuard,
    update_method: UpdateMethod,
) -> UpdateCheckRow {
    let row = gtk::Box::new(gtk::Orientation::Vertical, 0);
    row.add_css_class("settings-option");
    let summary = gtk::Box::new(gtk::Orientation::Horizontal, 16);
    summary.set_vexpand(true);
    let copy = gtk::Box::new(gtk::Orientation::Vertical, 2);
    copy.set_hexpand(true);
    copy.set_valign(gtk::Align::Center);
    let title = gtk::Label::new(Some("Updates"));
    title.set_xalign(0.0);
    title.add_css_class("settings-option-title");
    let status = gtk::Label::new(Some(&installed_version_status(
        &crate::build_info::installed_version(),
        crate::build_info::build_kind(),
        update_method,
    )));
    status.set_xalign(0.0);
    status.set_wrap(true);
    status.set_use_markup(true);
    status.add_css_class("settings-option-description");
    copy.append(&title);
    copy.append(&status);
    let progress = gtk::ProgressBar::new();
    progress.add_css_class("settings-update-progress");
    progress.set_hexpand(true);
    progress.set_visible(false);
    copy.append(&progress);
    let button = gtk::Button::with_label("Check now");
    button.add_css_class("settings-update-check");
    button.set_valign(gtk::Align::Center);
    summary.append(&copy);
    summary.append(&button);
    row.append(&summary);
    available_notes.container.add_css_class("inline");
    available_notes.container.set_visible(false);
    row.append(&available_notes.container);

    let checking = Rc::new(Cell::new(false));
    // Set once a check finds an update this platform can install; consumed by the
    // button's next click instead of re-running a check.
    let pending_download = Rc::new(RefCell::new(None::<PendingInstall>));
    // Set once an install finishes, so the next click restarts instead of re-checking.
    let installed = Rc::new(Cell::new(false));
    let installing = Rc::new(Cell::new(false));
    let install_underway: Rc<dyn Fn() -> bool> = Rc::new({
        let installed = installed.clone();
        let installing = installing.clone();
        move || installed.get() || installing.get()
    });
    let managed_update_available = Rc::new(Cell::new(false));
    // The generation of the most recently started check. Each call to
    // `run_check` captures the next value and compares against this when its
    // result arrives; a mismatch means a newer check (e.g. from a channel
    // toggle mid-flight) has since superseded it. Without this, a Preview
    // fetch still in flight when the user flips back to Stable could land
    // after the flip and offer an RC to a Stable user -- exactly the bug
    // `is_stale_check` exists to prevent. See its doc comment.
    let generation = Rc::new(Cell::new(0_u64));

    let run_check: Rc<dyn Fn(bool)> = Rc::new({
        let checking = checking.clone();
        let generation = generation.clone();
        let status = status.clone();
        let button = button.clone();
        let update_notice = update_notice.clone();
        let pending_download = pending_download.clone();
        let installed = installed.clone();
        let managed_update_available = managed_update_available.clone();
        let progress = progress.clone();
        let available_notes = available_notes.clone();
        let manager = manager.clone();
        move |force: bool| {
            // Always start a fresh check rather than dropping it: a channel
            // toggle must never be silently ignored just because a previous
            // check (for the old channel) is still in flight. The stale
            // check's own result is discarded below instead, once its
            // generation no longer matches.
            let my_generation = generation.get().saturating_add(1);
            generation.set(my_generation);
            checking.set(true);
            *pending_download.borrow_mut() = None;
            installed.set(false);
            managed_update_available.set(false);
            button.set_label("Check now");
            progress.set_fraction(0.0);
            progress.set_visible(false);
            progress.remove_css_class("error");
            status.set_text("Checking for updates…");
            available_notes.container.set_visible(false);
            available_notes.fallback.set_visible(false);
            // Clear any previously offered release immediately, not only once
            // this check's own result lands: otherwise the sidebar keeps
            // showing a (possibly prerelease) offer from before the channel
            // was switched for the whole duration of this check.
            update_notice(None);
            button.set_sensitive(false);
            // Read the channel now, not once when the row was built: a
            // mid-session channel toggle must be reflected by the very next
            // check, including this one if it was triggered by that toggle.
            let channel = effective_update_channel(manager.release_channel(), update_method);
            let receiver = services::check_for_updates(
                channel,
                crate::build_info::installed_version(),
                update_method,
                force,
            );
            let checking = checking.clone();
            let generation = generation.clone();
            let status = status.clone();
            let button = button.clone();
            let update_notice = update_notice.clone();
            let pending_download = pending_download.clone();
            let managed_update_available = managed_update_available.clone();
            let available_notes = available_notes.clone();
            glib::timeout_add_local(Duration::from_millis(100), move || {
                if is_stale_check(my_generation, generation.get()) {
                    // A newer check has since started; that one owns
                    // `checking`, `status`, and every other piece of shared
                    // state this closure would otherwise touch. Stop polling
                    // without applying this result.
                    return glib::ControlFlow::Break;
                }
                match receiver.try_recv() {
                    Ok(result) => {
                        let returns_to_stable = matches!(
                            &result,
                            UpdateCheck::Available { release, .. }
                                if channel == Channel::Stable
                                    && crate::build_info::build_kind() != BuildKind::Stable
                                    && release.kind == BuildKind::Stable
                        );
                        let message = if returns_to_stable
                            && matches!(update_method, UpdateMethod::InPlace | UpdateMethod::Aur)
                        {
                            let UpdateCheck::Available { release, .. } = &result else {
                                unreachable!();
                            };
                            format!(
                                "Stable channel target: <a href=\"{}\">v{}</a>",
                                glib::markup_escape_text(&release.url),
                                glib::markup_escape_text(&release.version),
                            )
                        } else {
                            update_check_message(&result, update_method)
                        };
                        status.set_markup(&update_status_markup(
                            message,
                            &result,
                            InstallSource::detect(),
                        ));
                        available_notes
                            .container
                            .set_visible(shows_available_release_notes(&result));
                        match &result {
                            UpdateCheck::Available {
                                release,
                                download_url,
                            } => update_notice(Some((
                                release.clone(),
                                download_url.clone(),
                                update_method,
                            ))),
                            UpdateCheck::UpToDate | UpdateCheck::Failed(_) => update_notice(None),
                        }
                        match &result {
                            UpdateCheck::Available {
                                release,
                                download_url,
                            } => {
                                show_release_notes(&available_notes, release);
                                if update_method.is_package_managed() {
                                    managed_update_available.set(true);
                                    button.set_label(match update_method {
                                        UpdateMethod::Omarchy => "Open Omarchy Update",
                                        UpdateMethod::Aur | UpdateMethod::Pacman => "Check again",
                                        UpdateMethod::InPlace => unreachable!(),
                                    });
                                } else {
                                    *pending_download.borrow_mut() = Some(PendingInstall {
                                        kind: release.kind,
                                        returns_to_stable,
                                        request: InstallRequest {
                                            download_url: download_url.clone(),
                                        },
                                    });
                                    button.set_label(if returns_to_stable {
                                        "Return to stable"
                                    } else {
                                        "Install update"
                                    });
                                }
                            }
                            UpdateCheck::UpToDate | UpdateCheck::Failed(_) => {}
                        }
                        button.set_sensitive(true);
                        checking.set(false);
                        glib::ControlFlow::Break
                    }
                    Err(TryRecvError::Empty) => glib::ControlFlow::Continue,
                    Err(TryRecvError::Disconnected) => {
                        status.set_markup(
                            "Couldn't check for updates · <a href=\"https://github.com/lgse/strata/releases/latest\">View releases on GitHub</a>",
                        );
                        available_notes.container.set_visible(false);
                        button.set_sensitive(true);
                        checking.set(false);
                        glib::ControlFlow::Break
                    }
                }
            });
        }
    });

    let clicked_check = run_check.clone();
    button.connect_clicked(move |button| {
        if update_method == UpdateMethod::Omarchy && managed_update_available.get() {
            match launch_omarchy_update() {
                Ok(()) => status.set_text("Omarchy Update opened in your terminal."),
                Err(error) => status.set_text(&format!("Couldn’t open Omarchy Update: {error}")),
            }
            return;
        }
        if installed.get() {
            restart_application(button);
            return;
        }
        if let Some(pending) = pending_download.borrow_mut().take() {
            if !offer_still_eligible(manager.release_channel(), pending.kind) {
                // The channel was switched back to Stable -- possibly from
                // another window, which this row never hears about -- after
                // this offer was cached. Drop it and re-check rather than
                // installing a prerelease the user has since opted out of;
                // `clicked_check` clears the sidebar notice and relabels the
                // button on its way.
                clicked_check(true);
                return;
            }
            let PendingInstall {
                kind: offered_kind,
                returns_to_stable,
                request,
            } = pending;
            if checking.replace(true) {
                return;
            }
            installing.set(true);
            status.set_text(if returns_to_stable {
                "Downloading stable release…"
            } else {
                "Downloading update…"
            });
            progress.set_fraction(0.0);
            progress.set_visible(true);
            progress.remove_css_class("error");
            button.set_sensitive(false);
            let progress_for_progress = progress.clone();
            let status_for_progress = status.clone();
            let checking_for_installed = checking.clone();
            let status_for_installed = status.clone();
            let button_for_installed = button.clone();
            let installed_for_installed = installed.clone();
            let installing_for_failed = installing.clone();
            let checking_for_failed = checking.clone();
            let status_for_failed = status.clone();
            let button_for_failed = button.clone();
            let progress_for_failed = progress.clone();
            let started = start_install(
                &install_guard,
                request,
                move |event| {
                    apply_install_progress(&status_for_progress, &progress_for_progress, event)
                },
                move || {
                    status_for_installed.set_text(if returns_to_stable {
                        "Stable release installed — restart to apply"
                    } else {
                        "Update installed — restart to apply"
                    });
                    button_for_installed.set_label("Restart now");
                    button_for_installed.set_sensitive(true);
                    installed_for_installed.set(true);
                    checking_for_installed.set(false);
                },
                move |message| {
                    match message {
                        Some(message) => status_for_failed
                            .set_text(&format!("Couldn't install update: {message}")),
                        None => status_for_failed.set_text("Couldn't install update"),
                    }
                    progress_for_failed.add_css_class("error");
                    button_for_failed.set_label("Check now");
                    button_for_failed.set_sensitive(true);
                    checking_for_failed.set(false);
                    installing_for_failed.set(false);
                },
            );
            if let Err(request) = started {
                // An install from an update dialog or another window is
                // already running. Leave this row
                // re-triable rather than stuck mid-"downloading" with
                // nothing actually happening.
                status.set_text("Another install is already running — try again shortly.");
                progress.set_visible(false);
                button.set_label(if returns_to_stable {
                    "Return to stable"
                } else {
                    "Install update"
                });
                button.set_sensitive(true);
                checking.set(false);
                installing.set(false);
                *pending_download.borrow_mut() = Some(PendingInstall {
                    kind: offered_kind,
                    returns_to_stable,
                    request,
                });
            }
        } else {
            clicked_check(true);
        }
    });
    UpdateCheckRow {
        row,
        run_check,
        responsive_action: (summary, button),
        install_underway,
    }
}

/// The three non-terminal states [`drive_install`] reports through
/// `on_progress`. Keeping this separate from [`UpdateInstall`] means callers
/// never need to (incorrectly) handle `Installed`/`Failed` in that closure --
/// those terminal states are always reported through the driver's other two
/// callbacks instead.
enum InstallProgress {
    Downloading { downloaded: u64, total: Option<u64> },
    Verifying,
    Installing,
}

/// Drives an install `receiver` on the GTK main loop until it reports a
/// terminal outcome, then stops.
///
/// This is the shared shape behind `update_check_row`'s and
/// `show_update_dialog`'s install flows: poll `receiver` every 100ms, forward
/// non-terminal updates to `on_progress`, and invoke exactly one of
/// `on_installed`/`on_failed` once a terminal state is reached. Deliberately
/// does *not* format status text itself because the two call sites use
/// different wording. `on_failed` receives `Some(message)` for an explicit
/// [`UpdateInstall::Failed`] or `None` when the receiver disconnected
/// without ever reporting one, since callers render those two cases
/// differently too.
fn drive_install(
    receiver: std::sync::mpsc::Receiver<UpdateInstall>,
    on_progress: impl Fn(InstallProgress) + 'static,
    on_installed: impl Fn() + 'static,
    on_failed: impl Fn(Option<String>) + 'static,
) {
    glib::timeout_add_local(Duration::from_millis(100), move || {
        loop {
            match receiver.try_recv() {
                Ok(UpdateInstall::Downloading { downloaded, total }) => {
                    on_progress(InstallProgress::Downloading { downloaded, total });
                }
                Ok(UpdateInstall::Verifying) => on_progress(InstallProgress::Verifying),
                Ok(UpdateInstall::Installing) => on_progress(InstallProgress::Installing),
                Ok(UpdateInstall::Installed) => {
                    on_installed();
                    return glib::ControlFlow::Break;
                }
                Ok(UpdateInstall::Failed(message)) => {
                    on_failed(Some(message));
                    return glib::ControlFlow::Break;
                }
                Err(TryRecvError::Empty) => return glib::ControlFlow::Continue,
                Err(TryRecvError::Disconnected) => {
                    on_failed(None);
                    return glib::ControlFlow::Break;
                }
            }
        }
    });
}

/// Starts `request`'s install unless another install-guarded flow is
/// already running, driving it with [`drive_install`] and clearing `guard`
/// once it reaches a terminal state.
///
/// `guard` is shared by [`update_check_row`] and [`show_update_dialog`] -- the
/// only call sites of [`services::install_update`]. Without it, controls in
/// separate windows could start two replacement threads concurrently.
///
/// Returns `Ok(())` once an install has started, or `Err(request)` --
/// handing `request` back unused -- if `guard` was already held. Callers
/// must handle the `Err` case by leaving their own button/status in a
/// re-triable state, since the click that produced it did not actually
/// start anything.
fn start_install(
    guard: &InstallGuard,
    request: InstallRequest,
    on_progress: impl Fn(InstallProgress) + 'static,
    on_installed: impl Fn() + 'static,
    on_failed: impl Fn(Option<String>) + 'static,
) -> Result<(), InstallRequest> {
    if guard.replace(true) {
        return Err(request);
    }
    let receiver = services::install_update(request);
    let guard_for_installed = guard.clone();
    let guard_for_failed = guard.clone();
    drive_install(
        receiver,
        on_progress,
        move || {
            guard_for_installed.set(false);
            on_installed();
        },
        move |message| {
            guard_for_failed.set(false);
            on_failed(message);
        },
    );
    Ok(())
}

/// The update row's compact progress rendering, distinct from the update
/// dialog's dialog-specific wording.
fn apply_install_progress(
    status: &gtk::Label,
    progress: &gtk::ProgressBar,
    event: InstallProgress,
) {
    match event {
        InstallProgress::Downloading { downloaded, total } => {
            if let Some(total) = total.filter(|total| *total > 0) {
                let fraction = (downloaded as f64 / total as f64).clamp(0.0, 1.0);
                progress.set_fraction(fraction);
                status.set_text(&format!("Downloading update… {:.0}%", fraction * 100.0));
            } else {
                progress.pulse();
                status.set_text(&format!(
                    "Downloading update… {:.1} MB",
                    downloaded as f64 / 1_048_576.0
                ));
            }
        }
        InstallProgress::Verifying => status.set_text("Verifying update…"),
        InstallProgress::Installing => {
            progress.set_fraction(1.0);
            status.set_text("Installing update…");
        }
    }
}

/// Relaunches the (just-updated) executable and quits the current instance.
fn restart_application(button: &gtk::Button) {
    let application = button
        .root()
        .and_then(|root| root.downcast::<gtk::Window>().ok())
        .and_then(|window| window.application());
    restart(application.as_ref());
}

fn restart(application: Option<&gtk::Application>) {
    use std::{os::unix::process::CommandExt, process::Stdio};

    let Ok(mut current_exe) = std::env::current_exe() else {
        return;
    };
    // On Linux, replacing the running executable makes /proc/self/exe resolve to
    // the old path with " (deleted)" appended. Relaunch the replacement at the
    // original path instead of treating that suffix as part of the filename.
    if !current_exe.exists()
        && let Some(path) = current_exe
            .to_str()
            .and_then(|path| path.strip_suffix(" (deleted)"))
        && std::path::Path::new(path).is_file()
    {
        current_exe = path.into();
    }
    // Wait for this process to exit completely before relaunching. A fixed
    // delay could overlap the old and new GTK/Wayland clients and rapidly hand
    // keyboard focus through an underlying terminal. Besides re-activating the
    // old GApplication instance, that exposed a Foot/libxkbcommon crash on
    // affected systems. Detach the waiter from inherited terminal streams and
    // put it in its own process group so applying an update cannot disturb the
    // terminal that launched Strata.
    let parent_pid = std::process::id().to_string();
    if std::process::Command::new("sh")
        .args([
            "-c",
            "while kill -0 \"$1\" 2>/dev/null; do sleep 0.1; done; sleep 0.5; exec \"$2\"",
            "strata-restart",
        ])
        .arg(parent_pid)
        .arg(current_exe)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .is_err()
    {
        return;
    }
    match application {
        Some(application) => application.quit(),
        None => std::process::exit(0),
    }
}

pub(super) fn show_update_dialog(
    parent: &gtk::Window,
    release: &ReleaseMetadata,
    download_url: String,
    install_guard: InstallGuard,
    update_method: UpdateMethod,
) {
    let Some(window_overlay) = parent.child().and_downcast::<gtk::Overlay>() else {
        return;
    };
    let blurred_root = window_overlay.child().and_downcast::<BlurBin>();
    if let Some(root) = blurred_root.as_ref() {
        root.set_blurred(true);
    }

    let layout = modal_layout(
        icons::DOWNLOADS,
        &format!("Strata v{} is available", release.version),
        &format!(
            "Installed v{}  →  Available v{}",
            crate::build_info::installed_version(),
            release.version
        ),
        match update_method {
            UpdateMethod::InPlace => "Download update",
            UpdateMethod::Omarchy => "Open Omarchy Update",
            UpdateMethod::Aur | UpdateMethod::Pacman => "Close",
        },
    );
    layout.content.add_css_class("update-dialog");
    layout.content.set_size_request(560, -1);
    // A prerelease offer must be visibly labelled, and must let the user
    // confirm exactly what they are about to install before doing so: which
    // channel it is, its precise tag, the source commit, and when it was
    // published.
    if release.kind != BuildKind::Stable {
        let badge = gtk::Label::new(Some(release.kind.label()));
        badge.add_css_class("prerelease-badge");
        badge.set_xalign(0.0);
        badge.set_halign(gtk::Align::Start);
        layout.body.append(&badge);
        layout.body.append(&update_dialog_details(release));
    }
    let notes_heading = gtk::Label::new(Some("What’s new"));
    notes_heading.add_css_class("release-notes-title");
    notes_heading.set_xalign(0.0);
    let notes = gtk::Box::new(gtk::Orientation::Vertical, 6);
    if release.notes.trim().is_empty() {
        set_release_notes_message(
            &notes,
            "No release notes were provided. Review this release on GitHub before continuing.",
        );
    } else {
        set_release_note_blocks(&notes, &release.note_blocks);
    }
    let notes_scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .min_content_height(120)
        .max_content_height(300)
        .propagate_natural_height(true)
        .child(&notes)
        .build();
    notes_scroll.add_css_class("update-dialog-notes");
    let fallback = gtk::LinkButton::with_label(&release.url, "View release on GitHub");
    fallback.add_css_class("release-notes-fallback");
    fallback.set_halign(gtk::Align::Start);
    let status_message = match update_method {
        UpdateMethod::InPlace => {
            "Review the release notes before downloading the update.".to_owned()
        }
        UpdateMethod::Aur => InstallSource::detect()
            .managed()
            .map(update_dialog_status)
            .unwrap_or_else(|| "This installation is managed by its package manager.".to_owned()),
        UpdateMethod::Omarchy => {
            "This installation is managed by Omarchy. Run “omarchy update” to install it."
                .to_owned()
        }
        UpdateMethod::Pacman => {
            "This installation is managed by pacman. Install it through a full system update."
                .to_owned()
        }
    };
    let status = gtk::Label::new(Some(&status_message));
    status.add_css_class("update-dialog-status");
    status.set_xalign(0.0);
    status.set_wrap(true);
    let progress = gtk::ProgressBar::new();
    progress.add_css_class("update-dialog-progress");
    progress.set_fraction(0.0);
    progress.set_visible(false);
    layout.body.append(&notes_heading);
    layout.body.append(&notes_scroll);
    layout.body.append(&fallback);
    layout.body.append(&status);
    layout.body.append(&progress);
    let content = layout.content;
    let close = layout.close;
    let cancel = layout.cancel;
    let action = layout.confirm;

    let layer = modal_layer(&content, &window_overlay, blurred_root.clone(), None);
    window_overlay.add_overlay(&layer);
    action.grab_focus();

    let started = Rc::new(Cell::new(false));
    let cancel_layer = layer.clone();
    let cancel_overlay = window_overlay.clone();
    let cancel_root = blurred_root.clone();
    let cancel_started = started.clone();
    cancel.connect_clicked(move |_| {
        if !cancel_started.get() {
            dismiss_modal_layer(&cancel_layer, &cancel_overlay, cancel_root.as_ref());
        }
    });
    let close_layer = layer.clone();
    let close_overlay = window_overlay.clone();
    let close_root = blurred_root.clone();
    let close_started = started.clone();
    close.connect_clicked(move |_| {
        if !close_started.get() {
            dismiss_modal_layer(&close_layer, &close_overlay, close_root.as_ref());
        }
    });
    let escape = gtk::EventControllerKey::new();
    let escape_layer = layer.clone();
    let escape_overlay = window_overlay.clone();
    let escape_root = blurred_root.clone();
    let escape_started = started.clone();
    escape.connect_key_pressed(move |_, key, _, _| {
        if key == gtk::gdk::Key::Escape {
            if !escape_started.get() {
                dismiss_modal_layer(&escape_layer, &escape_overlay, escape_root.as_ref());
            }
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    layer.add_controller(escape);

    let installed = Rc::new(Cell::new(false));
    // Set when the offer this dialog was opened with is no longer eligible on
    // the current channel, which turns the action button into a plain Close.
    let withdrawn = Rc::new(Cell::new(false));
    let offered_kind = release.kind;
    let withdraw: Rc<dyn Fn()> = Rc::new({
        let withdrawn = withdrawn.clone();
        let status = status.clone();
        let action = action.clone();
        move || {
            withdrawn.set(true);
            status.set_text(
                "This build is no longer offered on your update channel — check for updates again.",
            );
            action.set_label("Close");
        }
    });
    ThemeManager::shared().on_release_channel_changed(&layer, {
        let withdraw = withdraw.clone();
        let withdrawn = withdrawn.clone();
        let started = started.clone();
        let installed = installed.clone();
        Rc::new(move || {
            if started.get() || installed.get() || withdrawn.get() {
                return;
            }
            if !offer_still_eligible(ThemeManager::shared().release_channel(), offered_kind) {
                withdraw();
            }
        })
    });
    let action_layer = layer.clone();
    let action_overlay = window_overlay.clone();
    let action_root = blurred_root.clone();
    let application = parent.application();
    let action_close = close.clone();
    action.connect_clicked(move |button| {
        if update_method == UpdateMethod::Omarchy {
            match launch_omarchy_update() {
                Ok(()) => {
                    dismiss_modal_layer(&action_layer, &action_overlay, action_root.as_ref());
                    button.set_sensitive(false);
                }
                Err(error) => status.set_text(&format!("Couldn’t open Omarchy Update: {error}")),
            }
            return;
        }
        if matches!(update_method, UpdateMethod::Aur | UpdateMethod::Pacman) {
            dismiss_modal_layer(&action_layer, &action_overlay, action_root.as_ref());
            button.set_sensitive(false);
            return;
        }
        if installed.get() {
            restart(application.as_ref());
            button.set_sensitive(false);
            return;
        }
        if withdrawn.get() {
            dismiss_modal_layer(&action_layer, &action_overlay, action_root.as_ref());
            button.set_sensitive(false);
            return;
        }
        // Read the channel at the click, not when the dialog was opened: this
        // dialog is driven by the sidebar notice, whose cached offer survives
        // a channel switch made anywhere in the process -- including in
        // another window. `withdrawn` rather than `started` so Cancel and
        // Escape keep dismissing normally.
        if !offer_still_eligible(ThemeManager::shared().release_channel(), offered_kind) {
            withdraw();
            return;
        }
        if started.replace(true) {
            dismiss_modal_layer(&action_layer, &action_overlay, action_root.as_ref());
            button.set_sensitive(false);
            return;
        }

        button.set_sensitive(false);
        cancel.set_sensitive(false);
        action_close.set_sensitive(false);
        progress.set_visible(true);
        status.set_text("Starting download…");
        let progress_for_progress = progress.clone();
        let status_for_progress = status.clone();
        let progress_for_installed = progress.clone();
        let status_for_installed = status.clone();
        let action_for_installed = button.clone();
        let installed_for_installed = installed.clone();
        let progress_for_failed = progress.clone();
        let status_for_failed = status.clone();
        let action_for_failed = button.clone();
        let status_for_guard = status.clone();
        let progress_for_guard = progress.clone();
        let action_for_guard = button.clone();
        let started_for_guard = started.clone();
        let install_guard = install_guard.clone();
        let request = InstallRequest {
            download_url: download_url.clone(),
        };
        let outcome = start_install(
            &install_guard,
            request,
            move |event| match event {
                InstallProgress::Downloading { downloaded, total } => {
                    if let Some(total) = total.filter(|total| *total > 0) {
                        let fraction = (downloaded as f64 / total as f64).clamp(0.0, 1.0);
                        progress_for_progress.set_fraction(fraction);
                        status_for_progress.set_text(&format!(
                            "Downloading… {:.0}%  ({:.1} of {:.1} MB)",
                            fraction * 100.0,
                            downloaded as f64 / 1_048_576.0,
                            total as f64 / 1_048_576.0,
                        ));
                    } else {
                        progress_for_progress.pulse();
                        status_for_progress.set_text(&format!(
                            "Downloading… {:.1} MB",
                            downloaded as f64 / 1_048_576.0
                        ));
                    }
                }
                InstallProgress::Verifying => status_for_progress.set_text("Verifying update…"),
                InstallProgress::Installing => {
                    progress_for_progress.set_fraction(1.0);
                    status_for_progress.set_text("Installing update…");
                }
            },
            move || {
                progress_for_installed.set_fraction(1.0);
                status_for_installed.set_text("Update installed — restart to apply");
                action_for_installed.set_label("Restart now");
                action_for_installed.add_css_class("suggested-action");
                action_for_installed.set_sensitive(true);
                installed_for_installed.set(true);
            },
            move |message| {
                match message {
                    Some(message) => {
                        status_for_failed.set_text(&format!("Couldn’t install update: {message}"));
                        progress_for_failed.add_css_class("error");
                    }
                    None => status_for_failed.set_text("Couldn’t install update"),
                }
                action_for_failed.set_label("Close");
                action_for_failed.set_sensitive(true);
            },
        );
        if outcome.is_err() {
            // An install from the update row or another window is already
            // running. Reset `started` too, so the next
            // click retries the install instead of being treated as a
            // dismissal -- this click never actually started one.
            status_for_guard.set_text("Another install is already running — try again shortly.");
            progress_for_guard.set_visible(false);
            action_for_guard.set_sensitive(true);
            cancel.set_sensitive(true);
            started_for_guard.set(false);
        }
    });
}

fn omarchy_update_command() -> Command {
    let mut command = Command::new("xdg-terminal-exec");
    command
        .args(["--", "omarchy", "update"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command
}

fn launch_omarchy_update() -> Result<(), String> {
    omarchy_update_command()
        .spawn()
        .map(|_child| ())
        .map_err(|error| error.to_string())
}

/// Renders `release`'s channel, tag, source commit, and publication date as
/// a small identity block, for the dialog to show above the notes whenever
/// it is offering a prerelease -- the issue requires the user be able to
/// confirm exactly what they are about to install before doing so.
fn update_dialog_details(release: &ReleaseMetadata) -> gtk::Box {
    let details = gtk::Box::new(gtk::Orientation::Vertical, 2);
    details.add_css_class("update-dialog-details");
    for (label, value) in [
        ("Channel", release.kind.label().to_owned()),
        ("Tag", release.tag.clone()),
        (
            "Commit",
            release
                .commit
                .clone()
                .unwrap_or_else(|| "Unknown".to_owned()),
        ),
        (
            "Published",
            release
                .published_at
                .clone()
                .unwrap_or_else(|| "Unknown".to_owned()),
        ),
    ] {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        row.add_css_class("update-dialog-detail-row");
        let label_widget = gtk::Label::new(Some(label));
        label_widget.add_css_class("update-dialog-detail-label");
        label_widget.set_xalign(0.0);
        label_widget.set_hexpand(true);
        let value_widget = gtk::Label::new(Some(&value));
        value_widget.add_css_class("update-dialog-detail-value");
        value_widget.set_xalign(0.0);
        value_widget.set_selectable(true);
        value_widget.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        if label == "Commit" {
            value_widget.add_css_class("monospace");
        }
        row.append(&label_widget);
        row.append(&value_widget);
        details.append(&row);
    }
    details
}

fn shows_available_release_notes(result: &UpdateCheck) -> bool {
    matches!(result, UpdateCheck::Available { .. })
}

/// The installed build's identity, for the update row's idle status line:
/// just the version for a stable build, or `Version {version} · {label}`
/// when running a prerelease -- so a user on an RC or nightly always sees
/// what they currently have installed, not just a bare version number.
fn installed_version_status(
    version: &Version,
    kind: BuildKind,
    update_method: UpdateMethod,
) -> String {
    let version = if kind == BuildKind::Stable {
        format!("Version {version}")
    } else {
        format!("Version {version} · {}", kind.label())
    };
    match update_method {
        UpdateMethod::InPlace => version,
        UpdateMethod::Aur => format!(
            "{version} · Managed by {}",
            InstallSource::detect()
                .managed()
                .map(ManagedInstall::manager)
                .unwrap_or("a package manager")
        ),
        UpdateMethod::Omarchy => format!("{version} · Managed by Omarchy"),
        UpdateMethod::Pacman => format!("{version} · Managed by pacman"),
    }
}

fn update_status_markup(message: String, result: &UpdateCheck, source: &InstallSource) -> String {
    match (source.managed(), result) {
        (Some(managed), UpdateCheck::Available { .. }) => format!(
            "{message}\n{}",
            glib::markup_escape_text(&managed.update_instruction())
        ),
        _ => message,
    }
}

fn update_dialog_status(managed: &ManagedInstall) -> String {
    format!(
        "{} {}",
        managed.ownership_summary(),
        managed.update_instruction()
    )
}

fn update_check_message(result: &UpdateCheck, update_method: UpdateMethod) -> String {
    match result {
        UpdateCheck::UpToDate => {
            format!(
                "Up to date — version {}",
                crate::build_info::installed_version()
            )
        }
        UpdateCheck::Available { release, .. } => {
            let instruction = match update_method {
                UpdateMethod::InPlace | UpdateMethod::Aur => "",
                UpdateMethod::Omarchy => " · Run “omarchy update” to install",
                UpdateMethod::Pacman => " · Install through a full system update",
            };
            format!(
                "Update available: <a href=\"{}\">v{}</a>{instruction}",
                glib::markup_escape_text(&release.url),
                glib::markup_escape_text(&release.version),
            )
        }
        UpdateCheck::Failed(message) => format!(
            "Couldn't check for updates: {} · <a href=\"https://github.com/lgse/strata/releases/latest\">View releases on GitHub</a>",
            glib::markup_escape_text(message)
        ),
    }
}

fn keybindings_page() -> gtk::Widget {
    let content = page_content();
    append_heading(&content, "NAVIGATION");
    for (label, keys) in [
        ("Move through items", "J / K  or  ↑ / ↓"),
        ("Open folder", "L / → / Enter"),
        ("Go to parent", "H / ←"),
        ("Edit location", "Ctrl + L"),
        ("Filter items", "Ctrl + F"),
        ("Toggle sidebar", "Ctrl + B"),
    ] {
        append_keybinding(&content, label, keys);
    }

    append_heading(&content, "FILE OPERATIONS");
    for (label, keys) in [
        ("Create new folder", "Ctrl + Shift + N"),
        ("Cut", "Ctrl + X"),
        ("Copy", "Ctrl + C"),
        ("Paste", "Ctrl + V"),
    ] {
        append_keybinding(&content, label, keys);
    }

    append_heading(&content, "APPLICATION");
    for (label, keys) in [
        ("Search", "Ctrl + K"),
        ("Open terminal", "Ctrl + T"),
        ("Refresh", "F5 / Ctrl + R"),
        ("Open settings", "Ctrl + ,"),
    ] {
        append_keybinding(&content, label, keys);
    }

    scrollable_page(&content, Some("settings-keybindings-scroll"))
}

fn about_page() -> gtk::Widget {
    let content = page_content();
    content.add_css_class("about-page");

    let identity = gtk::Box::new(gtk::Orientation::Vertical, 7);
    identity.add_css_class("about-identity");
    identity.set_halign(gtk::Align::Center);

    let name = gtk::Label::new(Some("Strata"));
    name.add_css_class("about-name");
    let description = gtk::Label::new(Some(crate::build_info::DESCRIPTION));
    description.add_css_class("about-description");
    description.set_justify(gtk::Justification::Center);
    description.set_wrap(true);
    identity.append(&name);
    identity.append(&description);
    content.append(&identity);

    append_heading(&content, "BUILD INFORMATION");
    let build = gtk::Box::new(gtk::Orientation::Vertical, 0);
    build.add_css_class("about-details");
    let version = crate::build_info::installed_version().to_string();
    append_about_detail(&build, "Version", &version, false);
    let build_kind = crate::build_info::build_kind();
    if build_kind != services::BuildKind::Stable {
        append_about_detail(&build, "Build", build_kind.label(), false);
    }
    append_about_detail(&build, "Commit", crate::build_info::COMMIT, true);
    content.append(&build);

    append_heading(&content, "PROJECT");
    let project = gtk::Box::new(gtk::Orientation::Vertical, 0);
    project.add_css_class("about-details");
    append_about_detail(&project, "Author", crate::build_info::AUTHOR, false);

    let repository = gtk::LinkButton::builder()
        .uri(crate::build_info::REPOSITORY)
        .tooltip_text("Open the Strata repository")
        .build();
    repository.add_css_class("about-repository");
    let repository_content = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let repository_label = gtk::Label::new(Some("GitHub repository"));
    repository_label.set_xalign(0.0);
    repository_label.set_hexpand(true);
    repository_content.append(&repository_label);
    repository_content.append(&crate::assets::primary_icon(icons::EXTERNAL_LINK, 16));
    repository.set_child(Some(&repository_content));
    project.append(&repository);
    content.append(&project);

    scrollable_page(&content, None)
}

fn append_about_detail(container: &gtk::Box, label: &str, value: &str, monospace: bool) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 16);
    row.add_css_class("about-detail-row");
    let label = gtk::Label::new(Some(label));
    label.add_css_class("about-detail-label");
    label.set_xalign(0.0);
    label.set_hexpand(true);
    let value = gtk::Label::new(Some(value));
    value.add_css_class("about-detail-value");
    value.set_selectable(true);
    if monospace {
        value.add_css_class("monospace");
    }
    row.append(&label);
    row.append(&value);
    container.append(&row);
}

fn append_keybinding(content: &gtk::Box, label: &str, keys: &str) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 16);
    row.add_css_class("keybinding-row");
    let label = gtk::Label::new(Some(label));
    label.set_xalign(0.0);
    label.set_hexpand(true);
    let keys = gtk::Label::new(Some(keys));
    keys.add_css_class("keybinding-keys");
    row.append(&label);
    row.append(&keys);
    content.append(&row);
}

fn theme_page(manager: Rc<ThemeManager>) -> (gtk::Widget, Vec<(gtk::FlowBox, u32)>) {
    let content = page_content();
    content.add_css_class("theme-page");

    let follow = gtk::Switch::builder()
        .active(manager.follows_omarchy())
        .valign(gtk::Align::Center)
        .build();
    if manager.is_omarchy_available() {
        append_heading(&content, "SYSTEM");
        let system = gtk::Box::new(gtk::Orientation::Horizontal, 14);
        system.add_css_class("settings-option");
        let icon = crate::assets::primary_icon(icons::MONITOR, 22);
        icon.add_css_class("system-theme-icon");
        let copy = gtk::Box::new(gtk::Orientation::Vertical, 2);
        copy.set_hexpand(true);
        copy.set_valign(gtk::Align::Center);
        let system_title = gtk::Label::new(Some("Follow Omarchy"));
        system_title.set_xalign(0.0);
        system_title.add_css_class("settings-option-title");
        let system_description = gtk::Label::new(Some(
            "Use the active Omarchy Quattro theme and follow system theme changes.",
        ));
        system_description.set_xalign(0.0);
        system_description.set_wrap(true);
        system_description.add_css_class("settings-option-description");
        copy.append(&system_title);
        copy.append(&system_description);
        system.append(&icon);
        system.append(&copy);
        system.append(&follow);
        content.append(&system);
    }

    append_heading(&content, "THEMES");
    let packaged = gtk::FlowBox::builder()
        .column_spacing(12)
        .row_spacing(12)
        .max_children_per_line(3)
        .min_children_per_line(1)
        .selection_mode(gtk::SelectionMode::None)
        .homogeneous(true)
        .build();
    packaged.add_css_class("theme-grid");
    let theme_search = gtk::Entry::new();
    theme_search.add_css_class("form-control");
    theme_search.add_css_class("theme-search");
    theme_search.set_placeholder_text(Some("Search themes"));
    let search_keys = gtk::EventControllerKey::new();
    search_keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let selected_search = theme_search.downgrade();
    search_keys.connect_key_pressed(move |_, key, _, modifiers| {
        if modifiers.contains(gdk::ModifierType::CONTROL_MASK)
            && matches!(key, gdk::Key::a | gdk::Key::A)
            && let Some(search) = selected_search.upgrade()
        {
            search.select_region(0, -1);
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    theme_search.add_controller(search_keys);
    let clear_search = gtk::Button::builder()
        .child(&crate::assets::primary_icon(icons::X, 15))
        .tooltip_text("Clear theme search")
        .halign(gtk::Align::End)
        .valign(gtk::Align::Center)
        .margin_end(6)
        .visible(false)
        .build();
    clear_search.add_css_class("theme-search-clear");
    clear_search.set_has_frame(false);
    let search_overlay = gtk::Overlay::new();
    search_overlay.set_child(Some(&theme_search));
    search_overlay.add_overlay(&clear_search);
    content.append(&search_overlay);
    let cleared_search = theme_search.clone();
    clear_search.connect_clicked(move |_| {
        cleared_search.set_text("");
        cleared_search.grab_focus();
    });
    let (appearance_filter, appearance_buttons) = segmented_control(&["All", "Light", "Dark"], 0);
    appearance_filter.add_css_class("theme-appearance-filter");
    content.append(&appearance_filter);
    let catalog_scroll = gtk::ScrolledWindow::builder()
        .child(&packaged)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .min_content_height(240)
        .max_content_height(330)
        .propagate_natural_height(true)
        .build();
    catalog_scroll.add_css_class("theme-catalog-scroll");
    let catalog_container = gtk::Box::new(gtk::Orientation::Vertical, 0);
    catalog_container.add_css_class("theme-catalog-container");
    catalog_container.append(&catalog_scroll);
    content.append(&catalog_container);

    append_heading(&content, "YOUR THEMES");
    let custom = gtk::FlowBox::builder()
        .column_spacing(12)
        .row_spacing(12)
        .max_children_per_line(3)
        .min_children_per_line(1)
        .selection_mode(gtk::SelectionMode::None)
        .homogeneous(true)
        .build();
    custom.add_css_class("theme-grid");
    content.append(&custom);

    let cards: ThemeCards = Rc::new(RefCell::new(Vec::new()));
    let mut catalog_cards = Vec::new();
    for theme in manager.themes() {
        let custom_theme = theme.custom;
        let name = theme.tokens.name.clone();
        let light = theme_is_light(&theme.tokens);
        let flow = if custom_theme { &custom } else { &packaged };
        let child = append_theme_card(flow, theme, &manager, &follow, &cards);
        if !custom_theme {
            catalog_cards.push((child, name, light));
        }
    }
    let catalog_cards = Rc::new(catalog_cards);
    let appearance = Rc::new(Cell::new(ThemeAppearance::All));
    let filtered_cards = catalog_cards.clone();
    let filtered_appearance = appearance.clone();
    let filter_search = theme_search.clone();
    let apply_catalog_filter: Rc<dyn Fn()> = Rc::new(move || {
        let query = filter_search.text();
        let appearance = filtered_appearance.get();
        for (child, name, light) in filtered_cards.iter() {
            let appearance_matches = match appearance {
                ThemeAppearance::All => true,
                ThemeAppearance::Dark => !light,
                ThemeAppearance::Light => *light,
            };
            child.set_visible(appearance_matches && theme_name_matches(name, &query));
        }
    });
    let search_filter = apply_catalog_filter.clone();
    theme_search.connect_changed(move |search| {
        clear_search.set_visible(!search.text().is_empty());
        search_filter();
    });
    for (button, value) in appearance_buttons.into_iter().zip([
        ThemeAppearance::All,
        ThemeAppearance::Light,
        ThemeAppearance::Dark,
    ]) {
        let appearance = appearance.clone();
        let apply_filter = apply_catalog_filter.clone();
        button.connect_toggled(move |button| {
            if button.is_active() {
                appearance.set(value);
                apply_filter();
            }
        });
    }

    let add = gtk::Button::new();
    add.add_css_class("add-theme-card");
    add.set_has_frame(false);
    let add_content = gtk::Box::new(gtk::Orientation::Vertical, 7);
    add_content.set_halign(gtk::Align::Center);
    add_content.set_valign(gtk::Align::Center);
    let plus = crate::assets::primary_icon(icons::PLUS, 22);
    let add_label = gtk::Label::new(Some("Add a theme"));
    add_content.append(&plus);
    add_content.append(&add_label);
    add.set_child(Some(&add_content));
    custom.insert(&add, -1);

    let (editor, editor_fields) = theme_editor(
        manager.clone(),
        custom.clone(),
        follow.clone(),
        cards.clone(),
    );
    editor.set_reveal_child(false);
    content.append(&editor);
    let shown_editor = editor.clone();
    add.connect_clicked(move |_| shown_editor.set_reveal_child(true));

    let scroller = scrollable_page(&content, None);
    let manager_for_follow = manager;
    follow.connect_active_notify(move |toggle| {
        let active = toggle.is_active();
        manager_for_follow.set_follow_omarchy(active);
        let selected_id = manager_for_follow.selected_id();
        for (id, card, check) in cards.borrow().iter() {
            let selected = !active && id == &selected_id;
            if selected {
                card.add_css_class("selected");
            } else {
                card.remove_css_class("selected");
            }
            check.set_visible(selected);
        }
    });
    (
        scroller,
        vec![(packaged, 3), (custom, 3), (editor_fields, 4)],
    )
}

#[derive(Clone, Copy)]
enum ThemeAppearance {
    All,
    Dark,
    Light,
}

fn theme_name_matches(name: &str, query: &str) -> bool {
    let query = query.trim().to_lowercase();
    query.is_empty() || name.to_lowercase().contains(&query)
}

fn theme_is_light(tokens: &ThemeTokens) -> bool {
    theme_background_is_light(&tokens.background)
}

fn theme_background_is_light(background: &str) -> bool {
    let value = background.strip_prefix('#').unwrap_or_default();
    let Ok(color) = u32::from_str_radix(value, 16) else {
        return false;
    };
    let channel = |shift| {
        let value = f64::from((color >> shift) & 0xff_u32) / 255.0;
        if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    let luminance = 0.2126 * channel(16) + 0.7152 * channel(8) + 0.0722 * channel(0);
    luminance > 0.4
}

fn append_theme_card(
    flow: &gtk::FlowBox,
    theme: Theme,
    manager: &Rc<ThemeManager>,
    follow: &gtk::Switch,
    cards: &ThemeCards,
) -> gtk::FlowBoxChild {
    let card = gtk::Button::new();
    card.add_css_class("theme-card");
    card.set_has_frame(false);
    card.set_overflow(gtk::Overflow::Visible);
    let content = gtk::Box::new(gtk::Orientation::Vertical, 6);
    let preview = gtk::Overlay::new();
    preview.set_child(Some(&theme_preview(&theme.tokens)));
    let check = gtk::Image::from_icon_name(icons::CHECK_ON_PRIMARY);
    check.add_css_class("theme-card-check");
    check.set_halign(gtk::Align::End);
    check.set_valign(gtk::Align::Start);
    check.set_margin_top(8);
    check.set_margin_end(8);
    check.set_pixel_size(10);
    preview.add_overlay(&check);
    content.append(&preview);
    let label_row = gtk::Box::new(gtk::Orientation::Horizontal, 7);
    let selected = !manager.follows_omarchy() && manager.selected_id() == theme.id;
    check.set_visible(selected);
    if selected {
        card.add_css_class("selected");
    }
    let label = gtk::Label::new(Some(&theme.tokens.name));
    label.set_xalign(0.0);
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    label_row.append(&label);
    content.append(&label_row);
    card.set_child(Some(&content));
    cards
        .borrow_mut()
        .push((theme.id.clone(), card.clone(), check));

    let theme_id = theme.id;
    let manager = manager.clone();
    let follow = follow.clone();
    let cards = cards.clone();
    card.connect_clicked(move |_| {
        manager.select_theme(&theme_id);
        follow.set_active(false);
        for (id, candidate, check) in cards.borrow().iter() {
            let selected = id == &theme_id;
            if selected {
                candidate.add_css_class("selected");
            } else {
                candidate.remove_css_class("selected");
            }
            check.set_visible(selected);
        }
    });
    flow.insert(&card, -1);
    card.parent()
        .and_downcast::<gtk::FlowBoxChild>()
        .expect("FlowBox must wrap inserted theme cards")
}

fn theme_preview(tokens: &ThemeTokens) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::new();
    area.add_css_class("theme-preview");
    area.set_content_width(190);
    area.set_content_height(72);
    let tokens = tokens.clone();
    area.set_draw_func(move |_, context, width, height| {
        let color = |value: &str| gdk::RGBA::parse(value).unwrap_or(gdk::RGBA::BLACK);
        let paint = |context: &gtk::cairo::Context, value: &str| {
            let value = color(value);
            context.set_source_rgba(
                f64::from(value.red()),
                f64::from(value.green()),
                f64::from(value.blue()),
                1.0,
            );
        };
        context.rounded_rectangle(0.0, 0.0, f64::from(width), f64::from(height), 6.0);
        context.clip();
        paint(context, &tokens.background);
        context.rectangle(0.0, 0.0, f64::from(width), f64::from(height));
        let _ = context.fill();
        paint(context, &tokens.surface);
        context.rectangle(0.0, 0.0, f64::from(width) * 0.40, f64::from(height));
        let _ = context.fill();
        for (x, y, w, value) in [
            (10.0, 23.0, 45.0, &tokens.dim_text),
            (10.0, 36.0, 59.0, &tokens.accent),
            (10.0, 51.0, 39.0, &tokens.dim_text),
            (f64::from(width) * 0.45, 23.0, 47.0, &tokens.accent),
            (f64::from(width) * 0.45, 37.0, 83.0, &tokens.dim_text),
            (f64::from(width) * 0.45, 51.0, 66.0, &tokens.dim_text),
        ] {
            paint(context, value);
            context.rounded_rectangle(x, y, w, 5.0, 2.5);
            let _ = context.fill();
        }
    });
    area
}

fn theme_editor(
    manager: Rc<ThemeManager>,
    custom: gtk::FlowBox,
    follow: gtk::Switch,
    cards: ThemeCards,
) -> (gtk::Revealer, gtk::FlowBox) {
    let panel = gtk::Box::new(gtk::Orientation::Vertical, 12);
    panel.add_css_class("theme-editor");
    let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let title = gtk::Label::new(Some("Add a theme"));
    title.add_css_class("settings-option-title");
    title.set_xalign(0.0);
    title.set_hexpand(true);
    header.append(&title);
    panel.append(&header);
    let name = form_entry();
    name.set_placeholder_text(Some("Theme name"));
    panel.append(&name);

    let values = Rc::new(RefCell::new(manager.starter_tokens()));
    let fields = gtk::FlowBox::builder()
        .column_spacing(18)
        .row_spacing(10)
        .max_children_per_line(4)
        .min_children_per_line(1)
        .selection_mode(gtk::SelectionMode::None)
        .homogeneous(true)
        .build();
    fields.add_css_class("theme-color-fields");
    for (label_text, field) in [
        ("Background", ColorField::Background),
        ("Surface", ColorField::Surface),
        ("Text", ColorField::Text),
        ("Accent", ColorField::Accent),
        ("Danger", ColorField::Danger),
        ("Muted", ColorField::Muted),
        ("Highlight", ColorField::Highlight),
        ("Border", ColorField::Border),
        ("Dim text", ColorField::DimText),
    ] {
        let field_row = gtk::Box::new(gtk::Orientation::Horizontal, 7);
        let label = gtk::Label::new(Some(label_text));
        label.set_xalign(0.0);
        let dialog = gtk::ColorDialog::builder()
            .title(format!("Choose {label_text}"))
            .with_alpha(false)
            .build();
        let picker = gtk::ColorDialogButton::new(Some(dialog));
        picker.add_css_class("theme-color-picker");
        if let Ok(color) = gdk::RGBA::parse(field.get(&values.borrow())) {
            picker.set_rgba(&color);
        }
        let values_for_color = values.clone();
        let manager_for_color = manager.clone();
        picker.connect_rgba_notify(move |picker| {
            field.set(
                &mut values_for_color.borrow_mut(),
                picker.rgba().to_string(),
            );
            manager_for_color.preview(&values_for_color.borrow());
        });
        field_row.append(&picker);
        field_row.append(&label);
        fields.insert(&field_row, -1);
    }
    panel.append(&fields);
    let error = gtk::Label::new(None);
    error.add_css_class("theme-editor-error");
    error.set_xalign(0.0);
    error.set_visible(false);
    panel.append(&error);
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    actions.set_halign(gtk::Align::End);
    let cancel = gtk::Button::with_label("Cancel");
    cancel.add_css_class("action-dialog-cancel");
    let save = gtk::Button::with_label("Add theme");
    save.add_css_class("action-dialog-confirm");
    actions.append(&cancel);
    actions.append(&save);
    panel.append(&actions);
    let revealer = gtk::Revealer::builder()
        .transition_type(gtk::RevealerTransitionType::SlideDown)
        .child(&panel)
        .build();
    let hidden = revealer.clone();
    let manager_for_cancel = manager.clone();
    cancel.connect_clicked(move |_| {
        manager_for_cancel.cancel_preview();
        hidden.set_reveal_child(false);
    });
    let hidden = revealer.clone();
    save.connect_clicked(move |_| {
        let mut tokens = values.borrow().clone();
        tokens.name = name.text().trim().to_owned();
        match manager.save_custom_theme(tokens.clone()) {
            Ok(id) => {
                error.set_visible(false);
                append_theme_card(
                    &custom,
                    Theme {
                        id,
                        tokens,
                        custom: true,
                    },
                    &manager,
                    &follow,
                    &cards,
                );
                hidden.set_reveal_child(false);
            }
            Err(message) => {
                error.set_text(&message.to_string());
                error.set_visible(true);
            }
        }
    });
    (revealer, fields)
}

#[derive(Clone, Copy)]
enum ColorField {
    Background,
    Surface,
    Text,
    Accent,
    Danger,
    Muted,
    Highlight,
    Border,
    DimText,
}
impl ColorField {
    fn get(self, tokens: &ThemeTokens) -> &str {
        match self {
            Self::Background => &tokens.background,
            Self::Surface => &tokens.surface,
            Self::Text => &tokens.text,
            Self::Accent => &tokens.accent,
            Self::Danger => &tokens.danger,
            Self::Muted => &tokens.muted,
            Self::Highlight => &tokens.highlight,
            Self::Border => &tokens.border,
            Self::DimText => &tokens.dim_text,
        }
    }
    fn set(self, tokens: &mut ThemeTokens, value: String) {
        *match self {
            Self::Background => &mut tokens.background,
            Self::Surface => &mut tokens.surface,
            Self::Text => &mut tokens.text,
            Self::Accent => &mut tokens.accent,
            Self::Danger => &mut tokens.danger,
            Self::Muted => &mut tokens.muted,
            Self::Highlight => &mut tokens.highlight,
            Self::Border => &mut tokens.border,
            Self::DimText => &mut tokens.dim_text,
        } = value;
    }
}

fn navigation_button(icon: &str, label: &str) -> (gtk::Button, gtk::Label, gtk::Box) {
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    let icon_image = crate::assets::primary_icon(icon, 18);
    let text = gtk::Label::new(Some(label));
    text.set_xalign(0.0);
    content.append(&icon_image);
    content.append(&text);
    let button = gtk::Button::builder()
        .child(&content)
        .tooltip_text(label)
        .build();
    button.set_has_frame(false);
    (button, text, content)
}

fn scrollable_page(content: &gtk::Box, class: Option<&str>) -> gtk::Widget {
    content.set_hexpand(true);
    let scroller = gtk::ScrolledWindow::builder()
        .child(content)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .hexpand(true)
        .vexpand(true)
        .build();
    scroller.add_css_class("settings-content-scroll");
    if let Some(class) = class {
        scroller.add_css_class(class);
    }
    scroller.upcast()
}

fn page_content() -> gtk::Box {
    let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
    content.add_css_class("settings-preferences");
    content
}

fn click_activation_option(
    mode: &str,
    activation: ClickActivation,
) -> (
    gtk::Box,
    Vec<gtk::Box>,
    Vec<gtk::ToggleButton>,
    Vec<gtk::ToggleButton>,
) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    row.add_css_class("click-activation-row");
    let title = gtk::Label::new(Some(mode));
    title.set_xalign(0.0);
    title.set_width_chars(8);
    title.add_css_class("settings-option-title");
    row.append(&title);

    let selected = |count| usize::from(count == ClickCount::Two);
    let (file_control, file_buttons) =
        segmented_control(&["1 click", "2 clicks"], selected(activation.files));
    let (folder_control, folder_buttons) =
        segmented_control(&["1 click", "2 clicks"], selected(activation.folders));
    let mut options = Vec::new();
    for (label, control) in [("Files", &file_control), ("Folders", &folder_control)] {
        let option = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        option.set_hexpand(true);
        let label = gtk::Label::new(Some(label));
        label.set_xalign(0.0);
        label.set_width_chars(7);
        label.add_css_class("settings-option-description");
        control.set_hexpand(true);
        control.add_css_class("click-activation-control");
        option.append(&label);
        option.append(control);
        row.append(&option);
        options.push(option);
    }
    (row, options, file_buttons, folder_buttons)
}

fn connect_click_activation_buttons(
    buttons: &[gtk::ToggleButton],
    other_buttons: &[gtk::ToggleButton],
    update: Rc<dyn Fn(ClickCount, ClickCount)>,
) {
    for button in buttons {
        let buttons = buttons.to_vec();
        let other_buttons = other_buttons.to_vec();
        let update = update.clone();
        button.connect_toggled(move |button| {
            if !button.is_active() {
                return;
            }
            let selected = |buttons: &[gtk::ToggleButton]| {
                if buttons.get(1).is_some_and(gtk::ToggleButton::is_active) {
                    ClickCount::Two
                } else {
                    ClickCount::One
                }
            };
            update(selected(&buttons), selected(&other_buttons));
        });
    }
}

fn settings_option(title: &str, description: &str, active: bool) -> (gtk::Box, gtk::Switch) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 16);
    row.add_css_class("settings-option");
    let copy = gtk::Box::new(gtk::Orientation::Vertical, 2);
    copy.set_hexpand(true);
    copy.set_valign(gtk::Align::Center);
    let title_label = gtk::Label::new(Some(title));
    title_label.set_xalign(0.0);
    title_label.set_wrap(true);
    title_label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    title_label.add_css_class("settings-option-title");
    let description_label = gtk::Label::new(Some(description));
    description_label.set_xalign(0.0);
    description_label.set_wrap(true);
    description_label.add_css_class("settings-option-description");
    copy.append(&title_label);
    copy.append(&description_label);
    let toggle = gtk::Switch::builder()
        .active(active)
        .valign(gtk::Align::Center)
        .build();
    toggle.update_property(&[
        gtk::accessible::Property::Label(title),
        gtk::accessible::Property::Description(description),
    ]);
    row.append(&copy);
    row.append(&toggle);
    (row, toggle)
}

fn video_preview_option(
    description: &str,
    active: bool,
    toggle_sensitive: bool,
    backend_sensitive: bool,
    selected_backend: MediaPreviewBackend,
    on_backend_selected: Rc<dyn Fn(MediaPreviewBackend)>,
) -> (gtk::Box, gtk::Switch, gtk::MenuButton) {
    let (row, toggle) = settings_option(
        "Use hardware acceleration for video previews.",
        description,
        active,
    );
    row.remove(&toggle);

    let content = gtk::Box::new(gtk::Orientation::Vertical, 2);
    content.add_css_class("column-menu");
    let options = [
        ("Automatic", MediaPreviewBackend::Automatic),
        ("VA-API", MediaPreviewBackend::VaApi),
        ("Vulkan", MediaPreviewBackend::Vulkan),
    ]
    .map(|(label, value)| {
        let (option, check) = menu_option(label, selected_backend == value);
        content.append(&option);
        (label, value, option, check)
    });
    let popover = gtk::Popover::builder()
        .child(&content)
        .has_arrow(false)
        .halign(gtk::Align::End)
        .position(gtk::PositionType::Bottom)
        .build();
    popover.add_css_class("column-popover");
    let backend = gtk::MenuButton::builder()
        .label(video_preview_backend_label(selected_backend))
        .always_show_arrow(true)
        .popover(&popover)
        .build();
    backend.add_css_class("form-control");
    backend.set_sensitive(backend_sensitive);
    backend.set_valign(gtk::Align::Center);
    backend.update_property(&[
        gtk::accessible::Property::Label("Video preview hardware backend"),
        gtk::accessible::Property::Description(description),
    ]);
    let checks = Rc::new(
        options
            .iter()
            .map(|(_, value, _, check)| (*value, check.clone()))
            .collect::<Vec<_>>(),
    );
    for (label, value, option, _) in options {
        let backend = backend.clone();
        let checks = checks.clone();
        let on_backend_selected = on_backend_selected.clone();
        option.connect_clicked(move |_| {
            backend.set_label(label);
            for (candidate, check) in checks.iter() {
                check.set_visible(*candidate == value);
            }
            backend.popdown();
            on_backend_selected(value);
        });
    }
    toggle.set_sensitive(toggle_sensitive);
    let controls = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    controls.set_valign(gtk::Align::Center);
    controls.append(&backend);
    controls.append(&toggle);
    row.append(&controls);
    (row, toggle, backend)
}

fn video_preview_backend_label(backend: MediaPreviewBackend) -> &'static str {
    match backend {
        MediaPreviewBackend::Automatic | MediaPreviewBackend::Software => "Automatic",
        MediaPreviewBackend::VaApi => "VA-API",
        MediaPreviewBackend::Vulkan => "Vulkan",
    }
}

fn video_preview_control_state(enabled: bool) -> (bool, bool, bool) {
    (enabled, true, enabled)
}

fn append_heading(container: &gtk::Box, text: &str) -> gtk::Label {
    let heading = gtk::Label::new(Some(text));
    heading.set_xalign(0.0);
    heading.add_css_class("menu-heading");
    container.append(&heading);
    heading
}

trait RoundedRectangle {
    fn rounded_rectangle(&self, x: f64, y: f64, width: f64, height: f64, radius: f64);
}
impl RoundedRectangle for gtk::cairo::Context {
    fn rounded_rectangle(&self, x: f64, y: f64, width: f64, height: f64, radius: f64) {
        let degrees = std::f64::consts::PI / 180.0;
        self.new_sub_path();
        self.arc(x + width - radius, y + radius, radius, -90.0 * degrees, 0.0);
        self.arc(
            x + width - radius,
            y + height - radius,
            radius,
            0.0,
            90.0 * degrees,
        );
        self.arc(
            x + radius,
            y + height - radius,
            radius,
            90.0 * degrees,
            180.0 * degrees,
        );
        self.arc(
            x + radius,
            y + radius,
            radius,
            180.0 * degrees,
            270.0 * degrees,
        );
        self.close_path();
    }
}
