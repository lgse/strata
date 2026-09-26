// SPDX-License-Identifier: MIT

use std::rc::Rc;

use gtk::prelude::*;

use crate::{
    assets::icons,
    sandbox::MediaPreviewBackend,
    services::CrossVolumeDropStrategy,
    ui::{
        browser_modes::{BrowserMode, ClickActivation, ClickCount},
        controls::{menu_option, segmented_control},
        preferences::PreferenceManager,
    },
};

use super::{
    ResponsiveActivationRow, append_heading,
    bindings::{bind_choice, bind_switch},
    page_content, scrollable_page, settings_option,
};

pub(super) fn general_page(
    manager: Rc<PreferenceManager>,
) -> (gtk::Widget, Vec<gtk::Box>, Vec<ResponsiveActivationRow>) {
    let preferences = page_content();

    append_browsing_options(&preferences, &manager);
    append_layout_options(&preferences, &manager);
    append_sidebar_options(&preferences, &manager);

    append_heading(&preferences, "OPENING ITEMS");
    let description = gtk::Label::new(Some("How many clicks open a file or folder in each view."));
    description.set_xalign(0.0);
    description.add_css_class("settings-section-description");
    preferences.append(&description);
    let responsive_activation_rows = append_click_activation(&preferences, &manager);

    let transfers = super::settings_group(&preferences, "FILE TRANSFERS");
    append_cross_volume_drop_option(&transfers, &manager);
    append_preference_switch(
        &transfers,
        &manager,
        PreferenceSwitch {
            title: "Open folder after dropping files",
            description: "Show the destination folder after a successful drag and drop.",
            read: PreferenceManager::open_folder_after_drop,
            write: PreferenceManager::set_open_folder_after_drop,
        },
    );

    let date_time = super::settings_group(&preferences, "DATE & TIME");
    append_date_format_option(&date_time, &manager);

    let performance = super::settings_group(&preferences, "PERFORMANCE");
    append_auto_refresh_option(&performance, &manager);
    append_thumbnail_workers_option(&performance, &manager);
    append_video_preview_option(&performance, &manager);

    let desktop = super::settings_group(&preferences, "DESKTOP INTEGRATION");
    let portal_row = crate::ui::portal_preferences::settings_row();
    super::search::tag(&portal_row, "Desktop integration");
    desktop.append(&portal_row);
    let udiskie_row = crate::ui::udiskie_preferences::settings_row();
    super::search::tag(&udiskie_row, "Unlock encrypted volumes");
    desktop.append(&udiskie_row);

    let startup = super::settings_group(&preferences, "STARTUP");
    append_default_directory_option(&startup, &manager);

    (
        scrollable_page(&preferences, None),
        vec![portal_row, udiskie_row],
        responsive_activation_rows,
    )
}

fn append_date_format_option(content: &gtk::Box, manager: &Rc<PreferenceManager>) {
    const CHOICES: [(&str, crate::util::DateFormat); 3] = [
        ("Relative", crate::util::DateFormat::Relative),
        ("ISO 8601", crate::util::DateFormat::Iso8601),
        ("Long", crate::util::DateFormat::Long),
    ];
    let menu = gtk::Box::new(gtk::Orientation::Vertical, 2);
    menu.add_css_class("column-menu");
    let popover = gtk::Popover::builder()
        .child(&menu)
        .has_arrow(false)
        .build();
    popover.add_css_class("column-popover");
    let button = gtk::MenuButton::builder()
        .popover(&popover)
        .always_show_arrow(true)
        .valign(gtk::Align::Center)
        .build();
    button.add_css_class("form-control");
    button.add_css_class("settings-choice");
    button.set_tooltip_text(Some("Modified date format"));
    crate::ui::accessibility::set_label(&button, "Modified date format");
    manager.bind_preference(&button, PreferenceManager::date_format, |widget, format| {
        if let Some(button) = widget.downcast_ref::<gtk::MenuButton>() {
            button.set_label(match format {
                crate::util::DateFormat::Relative => "Relative",
                crate::util::DateFormat::Iso8601 => "ISO 8601",
                crate::util::DateFormat::Long => "Long",
            });
        }
    });
    let mut examples = Vec::new();
    for (name, format) in CHOICES {
        let copy = gtk::Box::new(gtk::Orientation::Vertical, 2);
        copy.set_hexpand(true);
        let title = gtk::Label::new(Some(name));
        title.set_xalign(0.0);
        let example = gtk::Label::new(None);
        example.set_xalign(0.0);
        example.add_css_class("settings-option-description");
        copy.append(&title);
        copy.append(&example);
        let check = crate::assets::primary_icon(icons::CHECK, 16);
        check.set_visible(manager.date_format() == format);
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        row.append(&copy);
        row.append(&check);
        let option = gtk::Button::builder().child(&row).build();
        option.add_css_class("column-menu-option");
        option.set_has_frame(false);
        manager.bind_preference(
            &check,
            PreferenceManager::date_format,
            move |widget, selected| widget.set_visible(selected == format),
        );
        let weak_button = button.downgrade();
        let manager = manager.clone();
        option.connect_clicked(move |_| {
            manager.set_date_format(format);
            if let Some(button) = weak_button.upgrade() {
                button.popdown();
            }
        });
        menu.append(&option);
        examples.push((example, format));
    }
    let examples = Rc::new(examples);
    let refresh = {
        let examples = examples.clone();
        move || {
            for (label, format) in examples.iter() {
                label.set_text(&crate::util::modified_date_example(*format));
            }
        }
    };
    refresh();
    popover.connect_show(move |_| refresh());
    content.append(&super::control_row(
        "Modified date format",
        "How file modified times appear in lists and details.",
        &button,
    ));
}

fn append_thumbnail_workers_option(content: &gtk::Box, manager: &Rc<PreferenceManager>) {
    let (control, [decrease, reset, increase]) = crate::ui::controls::stepper([
        "Decrease thumbnail workers",
        "Reset thumbnail workers",
        "Increase thumbnail workers",
    ]);
    for (button, increase) in [(decrease, false), (increase, true)] {
        manager.bind_preference(
            &button,
            PreferenceManager::thumbnail_workers,
            move |widget, workers| {
                widget.set_sensitive(if increase {
                    workers < crate::sandbox::browser::MAX_WORKERS
                } else {
                    workers > 1
                });
            },
        );
        let manager = manager.clone();
        button.connect_clicked(move |_| {
            let workers = manager.thumbnail_workers();
            manager.set_thumbnail_workers(if increase {
                workers.saturating_add(1)
            } else {
                workers.saturating_sub(1)
            });
        });
    }
    manager.bind_preference(
        &reset,
        PreferenceManager::thumbnail_workers,
        |widget, workers| {
            widget
                .downcast_ref::<gtk::Button>()
                .expect("worker count")
                .set_label(&workers.to_string());
        },
    );
    let manager = manager.clone();
    reset.connect_clicked(move |_| {
        manager.set_thumbnail_workers(crate::sandbox::browser::default_worker_limit())
    });
    content.append(&super::control_row(
        "Thumbnail workers",
        "Parallel thumbnail decoders across all windows. More workers use more CPU and memory. Click the number to reset to the recommended default.",
        &control,
    ));
}

#[derive(Clone, Copy)]
struct PreferenceSwitch {
    title: &'static str,
    description: &'static str,
    read: fn(&PreferenceManager) -> bool,
    write: fn(&PreferenceManager, bool),
}

fn append_browsing_options(content: &gtk::Box, manager: &Rc<PreferenceManager>) {
    let browsing = super::settings_group(content, "BROWSING");
    for switch in [
        PreferenceSwitch {
            title: "Folder peeking",
            description: "Preview folders automatically while moving through a pane.",
            read: PreferenceManager::folder_peeking,
            write: PreferenceManager::set_folder_peeking,
        },
        PreferenceSwitch {
            title: "Single-click file previews",
            description: "Show a quick preview when selecting a supported file.",
            read: PreferenceManager::single_click_previews,
            write: PreferenceManager::set_single_click_previews,
        },
        PreferenceSwitch {
            title: "Autoplay media previews",
            description: "Start playing video, audio, and GIF previews as soon as they open.",
            read: PreferenceManager::preview_autoplay,
            write: PreferenceManager::set_preview_autoplay,
        },
        PreferenceSwitch {
            title: "Render documents by default",
            description: "Open Markdown and HTML previews in the rendered view instead of source.",
            read: PreferenceManager::render_documents_by_default,
            write: PreferenceManager::set_render_documents_by_default,
        },
        PreferenceSwitch {
            title: "Keep arrows in file list",
            description: "Stop arrow keys from leaving the file list. Use Ctrl + Shift + B to focus the sidebar, or use the mouse.",
            read: PreferenceManager::arrow_navigation_scoped,
            write: PreferenceManager::set_arrow_navigation_scoped,
        },
        PreferenceSwitch {
            title: "Mirror columns selection",
            description: "Show the selected folder's contents in the next pane as you move with the keyboard.",
            read: PreferenceManager::columns_mirror_selection,
            write: PreferenceManager::set_columns_mirror_selection,
        },
        PreferenceSwitch {
            title: "10xer mode",
            description: crate::ui::tenxer_mode::MODE_DESCRIPTION,
            read: PreferenceManager::tenxer_mode,
            write: PreferenceManager::set_tenxer_mode,
        },
    ] {
        append_preference_switch(&browsing, manager, switch);
    }
    let search = super::settings_group(content, "SEARCH & FILTERING");
    for switch in [
        PreferenceSwitch {
            title: "Type to search",
            description: "Start filtering the active pane as soon as you type.",
            read: PreferenceManager::type_to_search,
            write: PreferenceManager::set_type_to_search,
        },
        PreferenceSwitch {
            title: "Include subfolders",
            description: "Also match items inside nested folders.",
            read: PreferenceManager::filter_include_subfolders,
            write: PreferenceManager::set_filter_include_subfolders,
        },
        PreferenceSwitch {
            title: "Open search results directly",
            description: "Launch files from search instead of showing the quick preview.",
            read: PreferenceManager::search_open_files_directly,
            write: PreferenceManager::set_search_open_files_directly,
        },
    ] {
        append_preference_switch(&search, manager, switch);
    }
}

fn append_layout_options(content: &gtk::Box, manager: &Rc<PreferenceManager>) {
    let layout = super::settings_group(content, "LAYOUT");
    for switch in [
        PreferenceSwitch {
            title: "Auto-hide sidebar",
            description: "Hide the sidebar until the pointer reaches the left edge of the window. Ctrl + B still shows it.",
            read: PreferenceManager::auto_hide_sidebar,
            write: PreferenceManager::set_auto_hide_sidebar,
        },
        PreferenceSwitch {
            title: "Auto-hide address bar",
            description: "Hide the address bar until the pointer reaches the top edge of the window. Ctrl + L still edits the location.",
            read: PreferenceManager::auto_hide_header,
            write: PreferenceManager::set_auto_hide_header,
        },
    ] {
        append_preference_switch(&layout, manager, switch);
    }
}

fn append_preference_switch(
    content: &gtk::Box,
    manager: &Rc<PreferenceManager>,
    switch: PreferenceSwitch,
) {
    let (row, toggle) = settings_option(switch.title, switch.description, (switch.read)(manager));
    bind_switch(manager, &toggle, switch.read, switch.write);
    if matches!(
        switch.title,
        "Type to search" | "Keep arrows in file list" | "Mirror columns selection"
    ) {
        bind_tenxer_unused_subtitle(&row, manager, switch.description);
    }
    if switch.title == "Include subfolders" {
        super::indent_row(&row);
    }
    if switch.title == "10xer mode" {
        append_experimental_label(&row, manager);
    }
    content.append(&row);
}

fn append_experimental_label(row: &gtk::Box, manager: &Rc<PreferenceManager>) {
    let Some(copy) = row.first_child().and_downcast::<gtk::Box>() else {
        return;
    };
    let experimental = gtk::Label::new(None);
    experimental.add_css_class("settings-option-description");
    experimental.add_css_class("tenxer-experimental");
    experimental.set_xalign(0.0);
    experimental.set_wrap(true);
    experimental.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    let label = experimental.clone();
    manager.bind_preference(
        &experimental,
        PreferenceManager::tenxer_mode,
        move |_, enabled| {
            label.set_text(if enabled {
                crate::ui::shortcut_reference::EXPERIMENTAL_LABEL
            } else {
                ""
            });
            label.set_visible(enabled);
        },
    );
    copy.append(&experimental);
}

fn bind_tenxer_unused_subtitle(
    row: &gtk::Box,
    manager: &Rc<PreferenceManager>,
    normal: &'static str,
) {
    let Some(description) = row
        .first_child()
        .and_then(|copy| copy.last_child())
        .and_downcast::<gtk::Label>()
    else {
        return;
    };
    let unused = crate::ui::tenxer_mode::UNUSED_SUBTITLE;
    manager.bind_preference(
        &description,
        PreferenceManager::tenxer_mode,
        move |widget, enabled| {
            if let Some(label) = widget.downcast_ref::<gtk::Label>() {
                label.set_text(if enabled { unused } else { normal });
                label.set_visible(true);
            }
        },
    );
}

fn append_default_directory_option(content: &gtk::Box, manager: &Rc<PreferenceManager>) {
    let choose = gtk::Button::with_label(&default_directory_text(manager.default_directory()));
    choose.set_valign(gtk::Align::Center);
    choose.add_css_class("form-control");
    choose.add_css_class("settings-choice");
    choose.set_tooltip_text(Some("Select default directory"));
    super::super::accessibility::set_label(&choose, "Default directory");

    let reset = gtk::Button::with_label("Reset");
    reset.add_css_class("form-control");
    reset.set_valign(gtk::Align::Center);
    reset.set_sensitive(manager.default_directory().is_some());
    reset.set_tooltip_text(Some("Restore the home directory as default"));

    let controls = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    controls.append(&choose);
    controls.append(&reset);

    let row = super::control_row(
        "Default directory",
        "Open this folder when launching Strata without a target.",
        &controls,
    );
    content.append(&row);

    manager.bind_preference(
        &choose,
        PreferenceManager::default_directory,
        move |widget, value| {
            if let Some(button) = widget.downcast_ref::<gtk::Button>() {
                button.set_label(&default_directory_text(value));
            }
        },
    );
    let manager_for_reset = manager.clone();
    reset.connect_clicked(move |_| {
        manager_for_reset.set_default_directory(None);
    });
    manager.bind_preference(
        &reset,
        PreferenceManager::default_directory,
        move |widget, value| {
            if let Some(button) = widget.downcast_ref::<gtk::Button>() {
                button.set_sensitive(value.is_some());
            }
        },
    );

    let manager = manager.clone();
    choose.connect_clicked(move |button| {
        let Some(window) = button.root().and_downcast::<gtk::Window>() else {
            return;
        };
        let dialog = gtk::FileDialog::builder()
            .title("Select default directory")
            .modal(true)
            .build();
        let manager = manager.clone();
        dialog.select_folder(Some(&window), gio::Cancellable::NONE, move |result| {
            let Ok(file) = result else {
                return;
            };
            if let Some(path) = file.path() {
                manager.set_default_directory(Some(path));
            }
        });
    });
}

fn default_directory_text(path: Option<std::path::PathBuf>) -> String {
    match path {
        Some(path) => abbreviate_home(&path),
        None => "Home directory".to_owned(),
    }
}

fn abbreviate_home(path: &std::path::Path) -> String {
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    if let Some(home) = home
        && let Ok(rest) = path.strip_prefix(&home)
    {
        format!("~/{}", rest.display())
    } else {
        path.display().to_string()
    }
}

fn append_sidebar_options(content: &gtk::Box, manager: &Rc<PreferenceManager>) {
    let sidebar = super::settings_group(content, "SIDEBAR");
    let chips = super::wrap::WrapRow::new(8);
    let row = super::control_row(
        "Items shown in sidebar",
        "Toggle which locations appear in the sidebar.",
        &chips,
    );
    row.add_css_class("settings-sidebar-places");
    row.set_orientation(gtk::Orientation::Vertical);
    row.set_spacing(18);
    sidebar.append(&row);
    use crate::assets::icons;
    let icons = [
        icons::HOME,
        icons::TRASH,
        icons::GLOBE,
        icons::CLOCK,
        icons::MONITOR,
        icons::DOCUMENTS,
        icons::DOWNLOADS,
        icons::PICTURES,
        icons::VIDEOS,
    ];
    for (switch, icon) in [
        PreferenceSwitch {
            title: "Show Home in sidebar",
            description: "Show the Home folder in the sidebar.",
            read: PreferenceManager::sidebar_show_home,
            write: PreferenceManager::set_sidebar_show_home,
        },
        PreferenceSwitch {
            title: "Show Trash in sidebar",
            description: "Show Trash in the sidebar.",
            read: PreferenceManager::sidebar_show_trash,
            write: PreferenceManager::set_sidebar_show_trash,
        },
        PreferenceSwitch {
            title: "Show Network in sidebar",
            description: "Show Network in the sidebar.",
            read: PreferenceManager::sidebar_show_network,
            write: PreferenceManager::set_sidebar_show_network,
        },
        PreferenceSwitch {
            title: "Show Recent in sidebar",
            description: "Show Recent files in the sidebar.",
            read: PreferenceManager::sidebar_show_recent,
            write: PreferenceManager::set_sidebar_show_recent,
        },
        PreferenceSwitch {
            title: "Show Desktop in sidebar",
            description: "Show the Desktop folder in the sidebar.",
            read: PreferenceManager::sidebar_show_desktop,
            write: PreferenceManager::set_sidebar_show_desktop,
        },
        PreferenceSwitch {
            title: "Show Documents in sidebar",
            description: "Show the Documents folder in the sidebar.",
            read: PreferenceManager::sidebar_show_documents,
            write: PreferenceManager::set_sidebar_show_documents,
        },
        PreferenceSwitch {
            title: "Show Downloads in sidebar",
            description: "Show the Downloads folder in the sidebar.",
            read: PreferenceManager::sidebar_show_downloads,
            write: PreferenceManager::set_sidebar_show_downloads,
        },
        PreferenceSwitch {
            title: "Show Pictures in sidebar",
            description: "Show the Pictures folder in the sidebar.",
            read: PreferenceManager::sidebar_show_pictures,
            write: PreferenceManager::set_sidebar_show_pictures,
        },
        PreferenceSwitch {
            title: "Show Videos in sidebar",
            description: "Show the Videos folder in the sidebar.",
            read: PreferenceManager::sidebar_show_videos,
            write: PreferenceManager::set_sidebar_show_videos,
        },
    ]
    .into_iter()
    .zip(icons)
    {
        let label = switch
            .title
            .trim_start_matches("Show ")
            .trim_end_matches(" in sidebar");
        let button = gtk::ToggleButton::new();
        button.add_css_class("sidebar-place-chip");
        let content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        content.append(&crate::assets::primary_icon(icon, 16));
        content.append(&gtk::Label::new(Some(label)));
        button.set_child(Some(&content));
        button.update_property(&[
            gtk::accessible::Property::Label(switch.title),
            gtk::accessible::Property::Description(switch.description),
        ]);
        super::bindings::bind_toggle(manager, &button, switch.read, switch.write);
        chips.append(&button);
    }
}

fn append_cross_volume_drop_option(content: &gtk::Box, manager: &Rc<PreferenceManager>) {
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
        PreferenceManager::cross_volume_drop_strategy,
        PreferenceManager::set_cross_volume_drop_strategy,
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

fn append_auto_refresh_option(content: &gtk::Box, manager: &Rc<PreferenceManager>) {
    let control = super::bindings::choice_menu(
        manager,
        "Auto-refresh folder",
        &[("Off", 0), ("1 min", 60), ("5 min", 300), ("10 min", 600)],
        PreferenceManager::auto_refresh_interval,
        PreferenceManager::set_auto_refresh_interval,
    );
    content.append(&super::control_row("Auto-refresh folder", "Reload the current folder on a timer. Useful for network shares where file monitors miss changes.", &control));
}

fn append_video_preview_option(content: &gtk::Box, manager: &Rc<PreferenceManager>) -> gtk::Box {
    let description = "Decode video thumbnails on the GPU.";
    let (video_row, acceleration, backend) = video_preview_option(manager, description);
    bind_switch(
        manager,
        &acceleration,
        PreferenceManager::hardware_accelerated_video_previews,
        PreferenceManager::set_hardware_accelerated_video_previews,
    );
    manager.bind_preference(
        &backend,
        PreferenceManager::hardware_accelerated_video_previews,
        |widget, enabled| widget.set_sensitive(video_preview_control_state(enabled).2),
    );
    content.append(&video_row);
    video_row
}

fn append_click_activation(
    content: &gtk::Box,
    manager: &Rc<PreferenceManager>,
) -> Vec<ResponsiveActivationRow> {
    let activation_options = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let mut responsive_activation_rows = Vec::new();
    activation_options.add_css_class("settings-group");
    activation_options.add_css_class("click-activation-options");
    super::search::tag(&activation_options, "Opening items");
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
    manager: &Rc<PreferenceManager>,
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
    manager: &Rc<PreferenceManager>,
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
        control.set_width_request(180);
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
    manager: &Rc<PreferenceManager>,
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
    super::search::tag(&row, "Hardware-accelerated video previews");
    row.append(&acceleration_row);
    row.append(&backend_row);
    (row, toggle, backend)
}

fn video_preview_backend_control(
    manager: &Rc<PreferenceManager>,
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
    manager: &Rc<PreferenceManager>,
    backend: &gtk::MenuButton,
    options: [(MediaPreviewBackend, gtk::Button, gtk::Image); 3],
) {
    manager.bind_preference(
        backend,
        PreferenceManager::video_preview_backend,
        |widget, selected| {
            if let Some(button) = widget.downcast_ref::<gtk::MenuButton>() {
                button.set_label(video_preview_backend_label(selected));
            }
        },
    );
    for (value, option, check) in options {
        manager.bind_preference(
            &check,
            PreferenceManager::video_preview_backend,
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
