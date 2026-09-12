// SPDX-License-Identifier: MIT

use std::{cell::RefCell, rc::Rc};

use gtk::{glib, prelude::*};

use crate::{
    services::{BuildKind, InstallSource, ManagedInstall, ReleaseMetadata, UpdateMethod},
    ui::{
        blur::BlurBin,
        settings::{self, InstallGuard, UpdateNoticeHandler},
        theme::ThemeManager,
    },
};

use super::{
    super::{SidebarView, bind_update_notice_preferences, sidebar_update_label},
    WindowContent,
};

#[cfg(test)]
mod tests;

type AvailableUpdate = Rc<RefCell<Option<(ReleaseMetadata, String, UpdateMethod)>>>;

pub(super) fn install(
    window: &gtk::ApplicationWindow,
    content: &WindowContent,
    preferences: &Rc<ThemeManager>,
) -> UpdateNoticeHandler {
    // The same process-wide guard covers this window's notice, lazy Settings
    // layer, and every other window's update and rollback controls.
    let guard = settings::install_guard();
    let notice = bind_update_notice(window, &content.sidebar, &guard);
    bind_update_notice_preferences(window, preferences, &notice);
    let launcher = Rc::new(SettingsLauncher {
        layer: RefCell::new(None),
        button: content.header.settings.clone(),
        blurred_root: content.blurred_root.clone(),
        overlay: content.overlay.clone(),
        preferences: preferences.clone(),
        notice: notice.clone(),
        guard,
    });
    let clicked_settings = launcher.clone();
    content
        .header
        .settings
        .connect_clicked(move |_| clicked_settings.show());
    let shortcut = gtk::EventControllerKey::new();
    shortcut.connect_key_pressed(move |_, key, _, modifiers| {
        if key != gtk::gdk::Key::comma || !modifiers.contains(gtk::gdk::ModifierType::CONTROL_MASK)
        {
            return glib::Propagation::Proceed;
        }
        launcher.show();
        glib::Propagation::Stop
    });
    window.add_controller(shortcut);
    notice
}

struct SettingsLauncher {
    layer: RefCell<Option<gtk::Box>>,
    button: gtk::Button,
    blurred_root: BlurBin,
    overlay: gtk::Overlay,
    preferences: Rc<ThemeManager>,
    notice: UpdateNoticeHandler,
    guard: InstallGuard,
}

impl SettingsLauncher {
    fn layer(&self) -> gtk::Box {
        if let Some(layer) = self.layer.borrow().clone() {
            return layer;
        }
        let layer = settings::build_layer(
            &self.button,
            &self.blurred_root,
            self.preferences.clone(),
            self.notice.clone(),
            self.guard.clone(),
        );
        self.overlay.add_overlay(&layer);
        self.layer.borrow_mut().replace(layer.clone());
        layer
    }

    fn show(&self) {
        let mut child = self.overlay.first_child();
        while let Some(widget) = child {
            child = widget.next_sibling();
            if widget.is_visible() && widget.has_css_class("app-modal-layer") {
                return;
            }
        }
        let layer = self.layer();
        self.blurred_root.set_blurred(true);
        layer.set_visible(true);
        layer.grab_focus();
        self.button.add_css_class("active");
        crate::ui::browser::animate_in(&layer);
    }
}

fn bind_update_notice(
    window: &gtk::ApplicationWindow,
    sidebar: &SidebarView,
    guard: &InstallGuard,
) -> UpdateNoticeHandler {
    let available: AvailableUpdate = Rc::new(RefCell::new(None));
    let available_for_click = available.clone();
    let parent = window.clone().upcast::<gtk::Window>();
    let guard = guard.clone();
    sidebar.update_notice.connect_clicked(move |_| {
        let Some((release, download_url, update_method)) = available_for_click.borrow().clone()
        else {
            return;
        };
        settings::show_update_dialog(
            &parent,
            &release,
            download_url,
            guard.clone(),
            update_method,
        );
    });
    notice_handler(sidebar, available)
}

fn notice_handler(sidebar: &SidebarView, available: AvailableUpdate) -> UpdateNoticeHandler {
    let button = sidebar.update_notice.clone();
    let label = sidebar.update_label.clone();
    let area = sidebar.update_area.clone();
    Rc::new(move |release| {
        if let Some((release, download_url, update_method)) = release {
            button.set_tooltip_text(Some(&update_tooltip(&release, update_method)));
            label.set_text(&sidebar_update_label(&release));
            if release.kind == BuildKind::Stable {
                button.remove_css_class("preview");
            } else {
                button.add_css_class("preview");
            }
            *available.borrow_mut() = Some((release, download_url, update_method));
            area.set_visible(true);
        } else {
            available.borrow_mut().take();
            area.set_visible(false);
        }
    })
}

fn update_tooltip(release: &ReleaseMetadata, method: UpdateMethod) -> String {
    match method {
        UpdateMethod::InPlace => format!("Install Strata v{}", release.version),
        UpdateMethod::Aur => format!(
            "Strata v{} is available through {}",
            release.version,
            InstallSource::detect()
                .managed()
                .map(ManagedInstall::manager)
                .unwrap_or("your package manager")
        ),
        UpdateMethod::Omarchy => {
            format!("Strata v{} is available through Omarchy", release.version)
        }
        UpdateMethod::Pacman => format!("Strata v{} is available through pacman", release.version),
    }
}
