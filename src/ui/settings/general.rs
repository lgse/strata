// SPDX-License-Identifier: MIT

use std::rc::Rc;

use gtk::prelude::*;

use crate::{
    sandbox::MediaPreviewBackend,
    services::CrossVolumeDropStrategy,
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

    append_heading(&preferences, "OPENING ITEMS");
    let description = gtk::Label::new(Some("How many clicks open a file or folder in each view."));
    description.set_xalign(0.0);
    description.add_css_class("settings-section-description");
    preferences.append(&description);
    let responsive_activation_rows = append_click_activation(&preferences, &manager);

    let transfers = super::settings_group(&preferences, "FILE TRANSFERS");
    append_cross_volume_drop_option(&transfers, &manager);

    let performance = super::settings_group(&preferences, "PERFORMANCE");
    append_auto_refresh_option(&performance, &manager);
    append_video_preview_option(&performance, &manager);

    append_heading(&preferences, "DESKTOP INTEGRATION");
    let portal_row = crate::ui::portal_preferences::settings_row();
    preferences.append(&portal_row);

    (
        scrollable_page(&preferences, None),
        vec![portal_row],
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
    let browsing = super::settings_group(content, "BROWSING");
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
    ] {
        append_preference_switch(&browsing, manager, switch);
    }
    let search = super::settings_group(content, "SEARCH & FILTERING");
    for switch in [
        PreferenceSwitch {
            title: "Type to search",
            description: "Start filtering the active pane as soon as you type.",
            read: ThemeManager::type_to_search,
            write: ThemeManager::set_type_to_search,
        },
        PreferenceSwitch {
            title: "Include subfolders",
            description: "Also match items inside nested folders.",
            read: ThemeManager::filter_include_subfolders,
            write: ThemeManager::set_filter_include_subfolders,
        },
        PreferenceSwitch {
            title: "Open search results directly",
            description: "Launch files from search instead of showing the quick preview.",
            read: ThemeManager::search_open_files_directly,
            write: ThemeManager::set_search_open_files_directly,
        },
    ] {
        append_preference_switch(&search, manager, switch);
    }
}

fn append_preference_switch(
    content: &gtk::Box,
    manager: &Rc<ThemeManager>,
    switch: PreferenceSwitch,
) {
    let (row, toggle) = settings_option(switch.title, switch.description, (switch.read)(manager));
    bind_switch(manager, &toggle, switch.read, switch.write);
    if switch.title == "Include subfolders" {
        super::indent_row(&row);
    }
    content.append(&row);
}

fn append_cross_volume_drop_option(content: &gtk::Box, manager: &Rc<ThemeManager>) {
    let control = super::bindings::choice_menu(
        manager,
        "Drag & drop to another device",
        &[
            (
                cross_volume_drop_strategy_label(CrossVolumeDropStrategy::Copy),
                CrossVolumeDropStrategy::Copy,
            ),
            (
                cross_volume_drop_strategy_label(CrossVolumeDropStrategy::Move),
                CrossVolumeDropStrategy::Move,
            ),
            (
                cross_volume_drop_strategy_label(CrossVolumeDropStrategy::Ask),
                CrossVolumeDropStrategy::Ask,
            ),
        ],
        ThemeManager::cross_volume_drop_strategy,
        ThemeManager::set_cross_volume_drop_strategy,
    );
    content.append(&super::control_row(
        "Drag & drop to another device",
        "What happens when you drop items onto a different drive or share.",
        &control,
    ));
}

pub(super) fn cross_volume_drop_strategy_label(strategy: CrossVolumeDropStrategy) -> &'static str {
    match strategy {
        CrossVolumeDropStrategy::Copy => "Always copy",
        CrossVolumeDropStrategy::Move => "Always move",
        CrossVolumeDropStrategy::Ask => "Always ask",
    }
}

fn append_auto_refresh_option(content: &gtk::Box, manager: &Rc<ThemeManager>) {
    let control = super::bindings::choice_menu(
        manager,
        "Auto-refresh folder",
        &[("Off", 0), ("1 min", 60), ("5 min", 300), ("10 min", 600)],
        ThemeManager::auto_refresh_interval,
        ThemeManager::set_auto_refresh_interval,
    );
    content.append(&super::control_row("Auto-refresh folder", "Reload the current folder on a timer. Useful for network shares where file monitors miss changes.", &control));
}

fn append_video_preview_option(content: &gtk::Box, manager: &Rc<ThemeManager>) -> gtk::Box {
    let description = "Decode video thumbnails on the GPU.";
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
    let activation_options = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let mut responsive_activation_rows = Vec::new();
    activation_options.add_css_class("settings-group");
    activation_options.add_css_class("click-activation-options");
    let header = gtk::Box::new(gtk::Orientation::Horizontal, 24);
    header.add_css_class("activation-header");
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    header.append(&spacer);
    for text in ["FILES", "FOLDERS"] {
        let label = gtk::Label::new(Some(text));
        label.set_width_chars(17);
        header.append(&label);
    }
    activation_options.append(&header);
    for (label, mode) in [
        ("Columns view", BrowserMode::Columns),
        ("Icons view", BrowserMode::Icons),
        ("List view", BrowserMode::List),
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
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 24);
    row.add_css_class("click-activation-row");
    let title = gtk::Label::new(Some(mode));
    title.set_xalign(0.0);
    title.set_hexpand(true);
    title.add_css_class("settings-option-title");
    row.append(&title);

    let selected = |count| usize::from(count == ClickCount::Two);
    let (file_control, file_buttons) =
        segmented_control(&["Single", "Double"], selected(activation.files));
    let (folder_control, folder_buttons) =
        segmented_control(&["Single", "Double"], selected(activation.folders));
    let mut options = Vec::new();
    for (label, control, buttons) in [
        ("Files", &file_control, &file_buttons),
        ("Folders", &folder_control, &folder_buttons),
    ] {
        let option = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        option.add_css_class("activation-option");
        option.set_hexpand(false);
        option.set_valign(gtk::Align::Center);
        let label = gtk::Label::new(Some(label));
        label.set_xalign(0.0);
        label.set_width_chars(7);
        label.add_css_class("settings-option-description");
        control.set_hexpand(false);
        control.set_valign(gtk::Align::Center);
        control.add_css_class("click-activation-control");
        // Include the view and item kind so assistive tools distinguish all twelve choices.
        for button in buttons {
            button.update_relation(&[gtk::accessible::Relation::LabelledBy(&[
                title.upcast_ref(),
                label.upcast_ref(),
                button.upcast_ref(),
            ])]);
        }
        label.add_css_class("activation-inline-label");
        label.set_visible(false);
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
    let (acceleration_row, toggle) =
        settings_option("Hardware-accelerated video previews", description, active);
    let backend = video_preview_backend_control(manager, "Decoding backend", backend_sensitive);
    backend.add_css_class("settings-choice");
    toggle.set_sensitive(toggle_sensitive);
    let backend_row = super::control_row("Decoding backend", "", &backend);
    super::indent_row(&backend_row);
    let row = gtk::Box::new(gtk::Orientation::Vertical, 0);
    row.append(&acceleration_row);
    row.append(&backend_row);
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
