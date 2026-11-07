// SPDX-License-Identifier: MIT

use std::rc::Rc;

use gtk::prelude::*;

use crate::{
    sandbox::MediaPreviewBackend,
    ui::{
        controls::{menu_option, number_stepper},
        preferences::PreferenceManager,
    },
};

use super::super::{
    bindings::{bind_number, bind_switch, show_choice_value},
    control_row, indent_row, search, settings_option,
};

pub(super) fn append_video_start_option(content: &gtk::Box, manager: &Rc<PreferenceManager>) {
    let control = number_stepper(0, crate::ui::preferences::MAX_PREVIEW_VIDEO_START, 5);
    crate::ui::accessibility::set_label(&control, &crate::i18n::tr("Video preview start"));
    bind_number(
        manager,
        &control,
        |manager| f64::from(manager.preview_video_start()),
        |manager, value| manager.set_preview_video_start(value as u32),
    );
    control.set_halign(gtk::Align::Start);
    let controls = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    controls.set_valign(gtk::Align::Center);
    controls.append(&control);
    let row = control_row(
        "Video preview start",
        "Percent of a video's length to open it at; 0 starts at the beginning. A video you already previewed resumes where you left it.",
        &controls,
    );
    row.add_css_class("settings-text-size-row");
    content.append(&row);
}

pub(super) fn append_video_preview_option(
    content: &gtk::Box,
    manager: &Rc<PreferenceManager>,
) -> gtk::Box {
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

fn video_preview_option(
    manager: &Rc<PreferenceManager>,
    description: &str,
) -> (gtk::Box, gtk::Switch, gtk::MenuButton) {
    let (active, toggle_sensitive, backend_sensitive) =
        video_preview_control_state(manager.hardware_accelerated_video_previews());
    let (acceleration_row, toggle) =
        settings_option("Hardware-accelerated video previews", description, active);
    let backend = video_preview_backend_control(manager, backend_sensitive);
    backend.add_css_class("settings-choice");
    toggle.set_sensitive(toggle_sensitive);
    let backend_row = control_row("Decoding backend", "", &backend);
    indent_row(&backend_row);
    let row = gtk::Box::new(gtk::Orientation::Vertical, 0);
    search::tag(&row, "Hardware-accelerated video previews");
    row.append(&acceleration_row);
    row.append(&backend_row);
    (row, toggle, backend)
}

fn video_preview_backend_control(
    manager: &Rc<PreferenceManager>,
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
        let (option, check) = menu_option(&crate::i18n::tr(label), selected_backend == value);
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
        .label(crate::i18n::tr(video_preview_backend_label(
            selected_backend,
        )))
        .always_show_arrow(true)
        .popover(&popover)
        .build();
    backend.add_css_class("form-control");
    backend.set_sensitive(backend_sensitive);
    backend.set_valign(gtk::Align::Center);
    crate::ui::accessibility::set_label(
        &backend,
        &crate::i18n::tr("Video preview hardware backend"),
    );
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
                show_choice_value(
                    button,
                    &crate::i18n::tr(video_preview_backend_label(selected)),
                );
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
