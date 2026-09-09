// SPDX-License-Identifier: MIT

use std::rc::Rc;

use gtk::prelude::*;

use crate::{
    sandbox::MediaPreviewBackend,
    ui::{
        browser_modes::{BrowserMode, ClickActivation, ClickCount},
        controls::{menu_option, segmented_control},
        theme::ThemeManager,
    },
};

use super::{
    ResponsiveActivationRow, append_heading,
    bindings::{bind_choice, bind_switch},
    page_content, scrollable_page, settings_option,
};

pub(super) fn general_page(
    manager: Rc<ThemeManager>,
) -> (gtk::Widget, Vec<gtk::Box>, Vec<ResponsiveActivationRow>) {
    let preferences = page_content();
    append_browsing_options(&preferences, &manager);

    append_heading(&preferences, "REFRESH");
    append_auto_refresh_option(&preferences, &manager);

    append_heading(&preferences, "VIDEO PREVIEWS");
    let video_row = append_video_preview_option(&preferences, &manager);

    append_heading(&preferences, "MOTION");
    append_preference_switch(
        &preferences,
        &manager,
        PreferenceSwitch {
            title: "Reduce motion",
            description: "Disable nonessential interface animations.",
            read: ThemeManager::reduce_motion,
            write: ThemeManager::set_reduce_motion,
        },
    );

    append_heading(&preferences, "CLICK ACTIVATION");
    let responsive_activation_rows = append_click_activation(&preferences, &manager);

    append_heading(&preferences, "DESKTOP INTEGRATION");
    let portal_row = crate::ui::portal_preferences::settings_row();
    preferences.append(&portal_row);

    (
        scrollable_page(&preferences, None),
        vec![video_row, portal_row],
        responsive_activation_rows,
    )
}

#[derive(Clone, Copy)]
struct PreferenceSwitch {
    title: &'static str,
    description: &'static str,
    read: fn(&ThemeManager) -> bool,
    write: fn(&ThemeManager, bool),
}

fn append_browsing_options(content: &gtk::Box, manager: &Rc<ThemeManager>) {
    append_heading(content, "BROWSING");
    for switch in [
        PreferenceSwitch {
            title: "Folder peeking",
            description: "Preview folders automatically while moving through a pane.",
            read: ThemeManager::folder_peeking,
            write: ThemeManager::set_folder_peeking,
        },
        PreferenceSwitch {
            title: "Single-click file previews",
            description: "Show a quick preview when selecting a supported file.",
            read: ThemeManager::single_click_previews,
            write: ThemeManager::set_single_click_previews,
        },
        PreferenceSwitch {
            title: "Open search results directly",
            description: "Launch files from search instead of opening Strata's quick preview.",
            read: ThemeManager::search_open_files_directly,
            write: ThemeManager::set_search_open_files_directly,
        },
        PreferenceSwitch {
            title: "Type to search",
            description: "Start filtering the active pane when you type in the file browser.",
            read: ThemeManager::type_to_search,
            write: ThemeManager::set_type_to_search,
        },
        PreferenceSwitch {
            title: "Include subfolders when filtering",
            description: "Search nested folders as well as the current folder when filtering a pane.",
            read: ThemeManager::filter_include_subfolders,
            write: ThemeManager::set_filter_include_subfolders,
        },
    ] {
        append_preference_switch(content, manager, switch);
    }
}

fn append_preference_switch(
    content: &gtk::Box,
    manager: &Rc<ThemeManager>,
    switch: PreferenceSwitch,
) {
    let (row, toggle) = settings_option(switch.title, switch.description, (switch.read)(manager));
    bind_switch(manager, &toggle, switch.read, switch.write);
    content.append(&row);
}

fn append_auto_refresh_option(content: &gtk::Box, manager: &Rc<ThemeManager>) {
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
    content.append(&refresh_row);
    for (idx, button) in buttons.iter().enumerate() {
        bind_choice(
            manager,
            button,
            secs[idx],
            ThemeManager::auto_refresh_interval,
            ThemeManager::set_auto_refresh_interval,
        );
    }
}

fn append_video_preview_option(content: &gtk::Box, manager: &Rc<ThemeManager>) -> gtk::Box {
    let description = "Choose a hardware backend.";
    let (video_row, acceleration, backend) = video_preview_option(manager, description);
    bind_switch(
        manager,
        &acceleration,
        ThemeManager::hardware_accelerated_video_previews,
        ThemeManager::set_hardware_accelerated_video_previews,
    );
    manager.bind_preference(
        &backend,
        ThemeManager::hardware_accelerated_video_previews,
        |widget, enabled| widget.set_sensitive(video_preview_control_state(enabled).2),
    );
    content.append(&video_row);
    video_row
}

fn append_click_activation(
    content: &gtk::Box,
    manager: &Rc<ThemeManager>,
) -> Vec<ResponsiveActivationRow> {
    let activation_options = gtk::Box::new(gtk::Orientation::Vertical, 4);
    let mut responsive_activation_rows = Vec::new();
    activation_options.add_css_class("settings-option");
    activation_options.add_css_class("click-activation-options");
    for (label, mode) in [
        ("Columns", BrowserMode::Columns),
        ("Icons", BrowserMode::Icons),
        ("List", BrowserMode::List),
    ] {
        let (row, options) = bind_click_activation_row(manager, label, mode);
        activation_options.append(&row);
        responsive_activation_rows.push(ResponsiveActivationRow { row, options });
    }
    content.append(&activation_options);
    responsive_activation_rows
}

fn bind_click_activation_row(
    manager: &Rc<ThemeManager>,
    label: &str,
    mode: BrowserMode,
) -> (gtk::Box, Vec<gtk::Box>) {
    let activation = manager.click_activation(mode);
    let (row, options, file_buttons, folder_buttons) = click_activation_option(label, activation);
    for (buttons, files) in [(&file_buttons, true), (&folder_buttons, false)] {
        for (button, count) in buttons.iter().zip([ClickCount::One, ClickCount::Two]) {
            bind_click_count(manager, button, ClickCountBinding { mode, files, count });
        }
    }
    (row, options)
}

struct ClickCountBinding {
    mode: BrowserMode,
    files: bool,
    count: ClickCount,
}

fn bind_click_count(
    manager: &Rc<ThemeManager>,
    button: &gtk::ToggleButton,
    binding: ClickCountBinding,
) {
    let ClickCountBinding { mode, files, count } = binding;
    bind_choice(
        manager,
        button,
        count,
        move |manager| {
            let activation = manager.click_activation(mode);
            if files {
                activation.files
            } else {
                activation.folders
            }
        },
        move |manager, count| {
            let mut activation = manager.click_activation(mode);
            if files {
                activation.files = count;
            } else {
                activation.folders = count;
            }
            manager.set_click_activation(mode, activation);
        },
    );
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
    for (label, control, buttons) in [
        ("Files", &file_control, &file_buttons),
        ("Folders", &folder_control, &folder_buttons),
    ] {
        let option = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        option.set_hexpand(true);
        let label = gtk::Label::new(Some(label));
        label.set_xalign(0.0);
        label.set_width_chars(7);
        label.add_css_class("settings-option-description");
        control.set_hexpand(true);
        control.add_css_class("click-activation-control");
        // Twelve buttons on this page read "1 click" or "2 clicks". Naming each
        // one after its row and its column turns them into distinguishable
        // choices such as "List Folders 1 click".
        for button in buttons {
            button.update_relation(&[gtk::accessible::Relation::LabelledBy(&[
                title.upcast_ref(),
                label.upcast_ref(),
                button.upcast_ref(),
            ])]);
        }
        option.append(&label);
        option.append(control);
        row.append(&option);
        options.push(option);
    }
    (row, options, file_buttons, folder_buttons)
}

fn video_preview_option(
    manager: &Rc<ThemeManager>,
    description: &str,
) -> (gtk::Box, gtk::Switch, gtk::MenuButton) {
    let (active, toggle_sensitive, backend_sensitive) =
        video_preview_control_state(manager.hardware_accelerated_video_previews());
    let (row, toggle) = settings_option(
        "Use hardware acceleration for video previews.",
        description,
        active,
    );
    row.remove(&toggle);
    let backend = video_preview_backend_control(manager, description, backend_sensitive);
    toggle.set_sensitive(toggle_sensitive);
    let controls = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    controls.set_valign(gtk::Align::Center);
    controls.append(&backend);
    controls.append(&toggle);
    row.append(&controls);
    (row, toggle, backend)
}

fn video_preview_backend_control(
    manager: &Rc<ThemeManager>,
    description: &str,
    backend_sensitive: bool,
) -> gtk::MenuButton {
    let selected_backend = manager.video_preview_backend();
    let menu = gtk::Box::new(gtk::Orientation::Vertical, 2);
    menu.add_css_class("column-menu");
    let options = [
        ("Automatic", MediaPreviewBackend::Automatic),
        ("VA-API", MediaPreviewBackend::VaApi),
        ("Vulkan", MediaPreviewBackend::Vulkan),
    ]
    .map(|(label, value)| {
        let (option, check) = menu_option(label, selected_backend == value);
        menu.append(&option);
        (value, option, check)
    });
    let popover = gtk::Popover::builder()
        .child(&menu)
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
    bind_video_preview_backend_menu(manager, &backend, options);
    backend
}

fn bind_video_preview_backend_menu(
    manager: &Rc<ThemeManager>,
    backend: &gtk::MenuButton,
    options: [(MediaPreviewBackend, gtk::Button, gtk::Image); 3],
) {
    manager.bind_preference(
        backend,
        ThemeManager::video_preview_backend,
        |widget, selected| {
            if let Some(button) = widget.downcast_ref::<gtk::MenuButton>() {
                button.set_label(video_preview_backend_label(selected));
            }
        },
    );
    for (value, option, check) in options {
        manager.bind_preference(
            &check,
            ThemeManager::video_preview_backend,
            move |widget, selected| widget.set_visible(selected == value),
        );
        let backend = backend.downgrade();
        let manager = manager.clone();
        option.connect_clicked(move |_| {
            manager.set_video_preview_backend(value);
            if let Some(backend) = backend.upgrade() {
                backend.popdown();
            }
        });
    }
}

pub(super) fn video_preview_backend_label(backend: MediaPreviewBackend) -> &'static str {
    match backend {
        MediaPreviewBackend::Automatic | MediaPreviewBackend::Software => "Automatic",
        MediaPreviewBackend::VaApi => "VA-API",
        MediaPreviewBackend::Vulkan => "Vulkan",
    }
}

pub(super) fn video_preview_control_state(enabled: bool) -> (bool, bool, bool) {
    (enabled, true, enabled)
}
