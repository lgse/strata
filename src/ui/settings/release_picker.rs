// SPDX-License-Identifier: MIT

use std::{
    cell::Cell,
    rc::Rc,
    sync::mpsc::{Receiver, TryRecvError},
    time::Duration,
};

use gtk::{glib, prelude::*};

use super::{
    InstallGuard, InstallLauncher, UpdateOffer, bindings::show_choice_value, build_update_dialog,
    control_row, default_install_launcher, release_date,
};
use crate::{
    assets::icons,
    services::{self, Channel, ReleaseChoice, ReleaseMetadata, UpdateMethod},
    ui::{accessibility, preferences::PreferenceManager},
};

pub(super) const RELEASE_PICKER_TITLE: &str = "Install a specific release";

type ReleaseList = Result<Vec<ReleaseChoice>, String>;

#[derive(Clone)]
pub(super) struct ReleaseSource {
    pub(super) list: Rc<dyn Fn(Channel) -> Receiver<ReleaseList>>,
    pub(super) resolve_commit: Rc<dyn Fn(ReleaseMetadata) -> Receiver<ReleaseMetadata>>,
    pub(super) launcher: InstallLauncher,
}

impl Default for ReleaseSource {
    fn default() -> Self {
        Self {
            list: Rc::new(services::fetch_releases),
            resolve_commit: Rc::new(services::resolve_release_commit),
            launcher: default_install_launcher(),
        }
    }
}

#[cfg_attr(
    not(test),
    expect(dead_code, reason = "tests open the menu and pick from it")
)]
pub(super) struct ReleasePicker {
    pub(super) row: gtk::Box,
    pub(super) button: gtk::MenuButton,
    pub(super) menu: gtk::Box,
}

/// Lists the releases the current channel offers, loaded when the menu first
/// opens and again after the channel changes, so Settings alone never calls GitHub.
pub(super) fn release_picker(
    manager: &Rc<PreferenceManager>,
    install_guard: InstallGuard,
    source: ReleaseSource,
) -> ReleasePicker {
    let menu = gtk::Box::new(gtk::Orientation::Vertical, 2);
    menu.add_css_class("column-menu");
    let scroll = gtk::ScrolledWindow::builder()
        .child(&menu)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .propagate_natural_height(true)
        .max_content_height(320)
        .min_content_width(240)
        .build();
    scroll.add_css_class("context-menu-scroll");
    let popover = gtk::Popover::builder()
        .child(&scroll)
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
    accessibility::set_label(&button, &crate::i18n::tr(RELEASE_PICKER_TITLE));
    show_choice_value(
        &button,
        &format!("v{}", crate::build_info::installed_version()),
    );
    let row = control_row(
        RELEASE_PICKER_TITLE,
        "Roll back or switch to another release your channel offers.",
        &button,
    );

    let listed = Rc::new(Cell::new(None::<Channel>));
    let generation = Rc::new(Cell::new(0u64));
    let load: Rc<dyn Fn()> = Rc::new({
        let manager = manager.clone();
        let menu = menu.downgrade();
        let button = button.downgrade();
        let listed = listed.clone();
        let generation = generation.clone();
        move || {
            let channel = manager.release_channel();
            let Some(menu_widget) = menu.upgrade() else {
                return;
            };
            if listed.get() == Some(channel) {
                return;
            }
            let current = generation.get().wrapping_add(1);
            generation.set(current);
            show_message(&menu_widget, &crate::i18n::tr("Loading releases…"));
            let receiver = (source.list)(channel);
            let menu = menu.clone();
            let button = button.clone();
            let listed = listed.clone();
            let generation = generation.clone();
            let source = source.clone();
            let install_guard = install_guard.clone();
            glib::timeout_add_local(Duration::from_millis(100), move || {
                if generation.get() != current {
                    return glib::ControlFlow::Break;
                }
                let result = match receiver.try_recv() {
                    Ok(result) => result.map_err(|message| {
                        rust_i18n::t!("Couldn't load releases: %{message}", message = message)
                            .into_owned()
                    }),
                    Err(TryRecvError::Empty) => return glib::ControlFlow::Continue,
                    Err(TryRecvError::Disconnected) => {
                        Err(crate::i18n::tr("Couldn't load releases"))
                    }
                };
                let (Some(menu), Some(button)) = (menu.upgrade(), button.upgrade()) else {
                    return glib::ControlFlow::Break;
                };
                match result {
                    Ok(choices) => {
                        listed.set(Some(channel));
                        fill_menu(&menu, &button, choices, &source, &install_guard);
                    }
                    Err(message) => show_message(&menu, &message),
                }
                glib::ControlFlow::Break
            });
        }
    });
    popover.connect_show({
        let load = load.clone();
        move |_| load()
    });
    let popover = popover.downgrade();
    manager.on_release_channel_changed(
        &row,
        Rc::new(move || {
            listed.set(None);
            generation.set(generation.get().wrapping_add(1));
            if popover
                .upgrade()
                .is_some_and(|popover| popover.is_visible())
            {
                load();
            }
        }),
    );
    ReleasePicker { row, button, menu }
}

fn clear_menu(menu: &gtk::Box) {
    while let Some(child) = menu.first_child() {
        menu.remove(&child);
    }
}

fn show_message(menu: &gtk::Box, message: &str) {
    clear_menu(menu);
    let label = gtk::Label::new(Some(message));
    label.set_xalign(0.0);
    label.set_wrap(true);
    label.set_max_width_chars(36);
    label.add_css_class("settings-option-description");
    menu.append(&label);
}

fn fill_menu(
    menu: &gtk::Box,
    button: &gtk::MenuButton,
    choices: Vec<ReleaseChoice>,
    source: &ReleaseSource,
    install_guard: &InstallGuard,
) {
    if choices.is_empty() {
        show_message(
            menu,
            &crate::i18n::tr("No releases are available on this channel."),
        );
        return;
    }
    clear_menu(menu);
    let installed = crate::build_info::installed_version().to_string();
    for choice in choices {
        let is_installed = choice.release.version == installed;
        let option = release_option(&choice.release, is_installed);
        let button = button.downgrade();
        let source = source.clone();
        let install_guard = install_guard.clone();
        option.connect_clicked(move |_| {
            let Some(button) = button.upgrade() else {
                return;
            };
            button.popdown();
            if !is_installed {
                open_release(&button, choice.clone(), &source, install_guard.clone());
            }
        });
        menu.append(&option);
    }
}

fn release_option(release: &ReleaseMetadata, installed: bool) -> gtk::Button {
    let name = format!("v{}", release.version);
    let kind = release.kind.localized_label();
    let detail = match release.published_at.as_deref() {
        Some(published_at) => rust_i18n::t!(
            "%{kind} · %{date}",
            kind = kind,
            date = release_date(published_at)
        )
        .into_owned(),
        None => kind,
    };
    let title = gtk::Label::new(Some(&name));
    title.set_xalign(0.0);
    let description = gtk::Label::new(Some(&detail));
    description.set_xalign(0.0);
    description.add_css_class("settings-option-description");
    let copy = gtk::Box::new(gtk::Orientation::Vertical, 2);
    copy.set_hexpand(true);
    copy.append(&title);
    copy.append(&description);
    let check = crate::assets::primary_icon(icons::CHECK, 16);
    check.set_visible(installed);
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    content.append(&copy);
    content.append(&check);
    let option = gtk::Button::builder()
        .child(&content)
        .accessible_role(gtk::AccessibleRole::MenuItemRadio)
        .build();
    option.add_css_class("column-menu-option");
    option.set_has_frame(false);
    accessibility::set_label(&option, &name);
    accessibility::set_description(&option, Some(&detail));
    accessibility::sync_checked_with_icon(&option, &check);
    option
}

/// Resolves the release's commit for the dialog's identity block, then asks the
/// user to confirm the install.
fn open_release(
    button: &gtk::MenuButton,
    choice: ReleaseChoice,
    source: &ReleaseSource,
    install_guard: InstallGuard,
) {
    button.set_sensitive(false);
    let receiver = (source.resolve_commit)(choice.release.clone());
    let button = button.downgrade();
    let launcher = source.launcher.clone();
    glib::timeout_add_local(Duration::from_millis(50), move || {
        let release = match receiver.try_recv() {
            Ok(release) => release,
            Err(TryRecvError::Empty) => return glib::ControlFlow::Continue,
            Err(TryRecvError::Disconnected) => choice.release.clone(),
        };
        let Some(button) = button.upgrade() else {
            return glib::ControlFlow::Break;
        };
        button.set_sensitive(true);
        if let Some(window) = button.root().and_downcast::<gtk::Window>() {
            let _dialog = build_update_dialog(
                &window,
                &release,
                choice.install.clone(),
                install_guard.clone(),
                UpdateMethod::InPlace,
                UpdateOffer::Chosen,
                launcher.clone(),
            );
        }
        glib::ControlFlow::Break
    });
}
