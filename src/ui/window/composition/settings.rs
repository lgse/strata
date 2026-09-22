// SPDX-License-Identifier: MIT

use std::{cell::RefCell, rc::Rc};

use gtk::{glib, prelude::*};

use crate::{
    services::{BuildKind, InstallSource, ManagedInstall, ReleaseMetadata, UpdateMethod},
    ui::{
        blur::BlurBin,
        preferences::PreferenceManager,
        settings::{self, InstallGuard, UpdateNoticeHandler},
    },
};

use super::{
    super::{SidebarView, bind_update_notice_preferences, sidebar_update_label},
    WindowContent,
};

#[cfg(test)]
mod tests;

type AvailableUpdate = Rc<RefCell<Option<(ReleaseMetadata, String, UpdateMethod)>>>;
type RestoreFocus = Rc<dyn Fn()>;

pub(super) fn install(
    window: &gtk::ApplicationWindow,
    content: &WindowContent,
    preferences: &Rc<PreferenceManager>,
) -> (UpdateNoticeHandler, Rc<dyn Fn()>) {
    // The same process-wide guard covers this window's notice, lazy Settings
    // layer, and every other window's update and rollback controls.
    let guard = settings::install_guard();
    let notice = bind_update_notice(window, &content.sidebar, &guard);
    settings::register_update_notice(&notice);
    bind_update_notice_preferences(window, preferences, &notice);
    let restore_focus = content.browser.downgrade();
    let launcher = Rc::new(SettingsLauncher {
        layer: RefCell::new(None),
        button: content.header.settings.downgrade(),
        blurred_root: content.blurred_root.downgrade(),
        overlay: content.overlay.downgrade(),
        preferences: preferences.clone(),
        notice: notice.clone(),
        guard,
        capture_focus: Rc::new(move || {
            restore_focus
                .upgrade()
                .map(|view| view.modal_focus_restore())
                .unwrap_or_else(|| Rc::new(|| {}))
        }),
        restore_focus: Rc::default(),
    });
    let clicked_settings = launcher.clone();
    content
        .header
        .settings
        .connect_clicked(move |_| clicked_settings.show());
    let shortcut_launcher = launcher.clone();
    let shortcut = gtk::EventControllerKey::new();
    shortcut.connect_key_pressed(move |_, key, _, modifiers| {
        if key != gtk::gdk::Key::comma || !modifiers.contains(gtk::gdk::ModifierType::CONTROL_MASK)
        {
            return glib::Propagation::Proceed;
        }
        shortcut_launcher.show();
        glib::Propagation::Stop
    });
    window.add_controller(shortcut);
    let show = Rc::new(move || launcher.show()) as Rc<dyn Fn()>;
    (notice, show)
}

struct SettingsLauncher {
    layer: RefCell<Option<glib::WeakRef<gtk::Box>>>,
    button: glib::WeakRef<gtk::Button>,
    blurred_root: glib::WeakRef<BlurBin>,
    overlay: glib::WeakRef<gtk::Overlay>,
    preferences: Rc<PreferenceManager>,
    notice: UpdateNoticeHandler,
    guard: InstallGuard,
    capture_focus: Rc<dyn Fn() -> RestoreFocus>,
    restore_focus: Rc<RefCell<Option<RestoreFocus>>>,
}

impl SettingsLauncher {
    fn layer(&self) -> Option<gtk::Box> {
        if let Some(layer) = self
            .layer
            .borrow()
            .as_ref()
            .and_then(|layer| layer.upgrade())
        {
            return Some(layer);
        }
        let button = self.button.upgrade()?;
        let root = self.blurred_root.upgrade()?;
        let overlay = self.overlay.upgrade()?;
        let layer = settings::build_layer(
            &button,
            &root,
            self.preferences.clone(),
            self.notice.clone(),
            self.guard.clone(),
        );
        let restore_focus = self.restore_focus.clone();
        layer.connect_hide(move |_| {
            let restore = restore_focus.borrow_mut().take();
            if let Some(restore) = restore {
                restore();
            }
        });
        overlay.add_overlay(&layer);
        self.layer.borrow_mut().replace(layer.downgrade());
        Some(layer)
    }

    fn show(&self) {
        let (Some(overlay), Some(root), Some(button)) = (
            self.overlay.upgrade(),
            self.blurred_root.upgrade(),
            self.button.upgrade(),
        ) else {
            return;
        };
        let mut child = overlay.first_child();
        while let Some(widget) = child {
            child = widget.next_sibling();
            if widget.is_visible() && widget.has_css_class("app-modal-layer") {
                return;
            }
        }
        let restore = (self.capture_focus)();
        let Some(layer) = self.layer() else {
            return;
        };
        self.restore_focus.replace(Some(restore));
        root.set_blurred(true);
        layer.set_visible(true);
        layer.grab_focus();
        button.add_css_class("active");
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
    let parent = window.downgrade();
    let guard = guard.clone();
    sidebar.update_notice.connect_clicked(move |_| {
        let Some(parent) = parent.upgrade() else {
            return;
        };
        let Some((release, download_url, update_method)) = available_for_click.borrow().clone()
        else {
            return;
        };
        settings::show_update_dialog(
            parent.upcast_ref(),
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
