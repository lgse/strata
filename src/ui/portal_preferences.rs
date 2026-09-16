// SPDX-License-Identifier: MIT

use std::{cell::Cell, rc::Rc, time::Duration};

use gtk::{gio, glib, prelude::*};

use crate::{assets::icons, portal_setup};

#[cfg(test)]
mod tests;

thread_local! {
    static SETUP_RUNNING: Cell<bool> = const { Cell::new(false) };
}

pub(super) fn settings_row() -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Vertical, 2);
    row.add_css_class("settings-option");
    let content = gtk::Box::new(gtk::Orientation::Vertical, 2);
    content.set_hexpand(true);
    let title = gtk::Label::new(Some("System file manager"));
    title.set_xalign(0.0);
    title.add_css_class("settings-option-title");
    let description = gtk::Label::new(Some(
        "Use Strata for Open and Save dialogs, opening folders, and Reveal in File Manager. On Omarchy, this also sets file-manager keyboard shortcuts. Close open file dialogs before changing setup.",
    ));
    description.set_xalign(0.0);
    description.set_wrap(true);
    description.add_css_class("settings-option-description");
    content.append(&title);
    content.append(&description);
    let summary = SettingsIntegrationStatus::new(&content);
    row.append(&content);
    let mapped_summary = summary.clone();
    row.connect_map(move |_| mapped_summary.reload(None));
    let weak_row = row.downgrade();
    glib::timeout_add_local(Duration::from_secs(2), move || {
        let Some(row) = weak_row.upgrade() else {
            return glib::ControlFlow::Break;
        };
        if row.is_mapped() {
            summary.set_sensitive(!SETUP_RUNNING.get() && !summary.busy.get());
            if !SETUP_RUNNING.get() {
                summary.reload(None);
            }
        }
        glib::ControlFlow::Continue
    });
    row
}

struct SettingsIntegrationStatus {
    indicators: [IntegrationIndicator; 4],
    message: glib::WeakRef<gtk::Label>,
    complete: glib::WeakRef<gtk::Button>,
    restore: glib::WeakRef<gtk::Button>,
    busy: Cell<bool>,
    known: Cell<bool>,
}

impl SettingsIntegrationStatus {
    fn new(parent: &gtk::Box) -> Rc<Self> {
        let list = gtk::Box::new(gtk::Orientation::Vertical, 4);
        list.set_margin_top(8);
        list.set_margin_bottom(8);
        let indicators = [
            "Open and Save dialogs",
            "Opening folders",
            "Reveal in File Manager",
            "Keyboard shortcuts",
        ]
        .map(|name| IntegrationIndicator::new(&list, name));
        let message = gtk::Label::new(None);
        message.set_visible(false);
        message.set_xalign(0.0);
        message.set_wrap(true);
        message.add_css_class("settings-option-description");
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        actions.add_css_class("settings-integration-actions");
        actions.set_halign(gtk::Align::End);
        actions.set_valign(gtk::Align::Start);
        let restore = gtk::Button::with_label("Restore default");
        restore.add_css_class("action-dialog-cancel");
        restore.set_sensitive(false);
        restore.set_visible(false);
        let complete = gtk::Button::with_label("Checking status…");
        complete.add_css_class("action-dialog-confirm");
        complete.set_sensitive(false);
        actions.append(&restore);
        actions.append(&complete);
        parent.append(&list);
        parent.append(&message);
        parent.append(&actions);
        let summary = Rc::new(Self {
            indicators,
            message: message.downgrade(),
            complete: complete.downgrade(),
            restore: restore.downgrade(),
            busy: Cell::new(false),
            known: Cell::new(false),
        });
        let setup = summary.clone();
        complete.connect_clicked(move |_| setup.apply(true));
        let reset = summary.clone();
        restore.connect_clicked(move |_| reset.apply(false));
        summary
    }

    fn set_sensitive(&self, sensitive: bool) {
        for button in [&self.complete, &self.restore] {
            if let Some(button) = button.upgrade() {
                button.set_sensitive(sensitive && self.known.get());
            }
        }
    }

    fn message(&self, text: &str, error: bool) {
        if let Some(message) = self.message.upgrade() {
            message.set_text(text);
            message.set_visible(!text.is_empty());
            if error {
                message.add_css_class("error");
            } else {
                message.remove_css_class("error");
            }
        }
    }

    fn reload(self: &Rc<Self>, failure: Option<String>) {
        if self.busy.replace(true) {
            return;
        }
        let summary = self.clone();
        glib::spawn_future_local(async move {
            let result = gio::spawn_blocking(|| {
                Ok::<_, String>((
                    portal_setup::status()?,
                    portal_setup::file_manager_status()?,
                ))
            })
            .await;
            summary.busy.set(false);
            summary.show_result(match result {
                Ok(result) => result,
                Err(error) => Err(format!("Could not check integration status: {error:?}")),
            });
            if let Some(failure) = failure {
                summary.message(&failure, true);
            }
        });
    }

    fn show_result(
        &self,
        result: Result<(portal_setup::PortalStatus, portal_setup::FileManagerStatus), String>,
    ) {
        if let Some(button) = self.complete.upgrade() {
            button.set_label("Complete setup");
        }
        if let Some(button) = self.restore.upgrade() {
            button.set_label("Restore default");
        }
        match result {
            Ok((chooser, manager)) => {
                self.known.set(true);
                for (indicator, configured) in self.indicators.iter().zip([
                    Some(chooser.configured),
                    Some(manager.default),
                    Some(manager.has_service),
                    manager.shortcuts,
                ]) {
                    indicator.update(configured);
                }
                let configured = chooser.configured
                    && manager.default
                    && manager.has_service
                    && manager.shortcuts != Some(false);
                let installed = chooser.configured
                    || chooser.has_installation
                    || manager.default
                    || manager.has_installation
                    || manager.shortcuts == Some(true);
                if let Some(complete) = self.complete.upgrade() {
                    complete.set_visible(!configured);
                }
                if let Some(restore) = self.restore.upgrade() {
                    restore.set_visible(installed);
                }
                if let Some(message) = self.message.upgrade()
                    && !message.has_css_class("error")
                {
                    message.set_visible(false);
                }
                self.set_sensitive(!SETUP_RUNNING.get());
            }
            Err(error) => {
                self.known.set(false);
                for indicator in &self.indicators {
                    indicator.update(None);
                }
                self.set_sensitive(false);
                self.message(&format!("Integration status unavailable: {error}"), true);
            }
        }
    }

    fn show_progress(&self, enable: bool) {
        self.set_sensitive(false);
        self.message("", false);
        let action = if enable {
            &self.complete
        } else {
            &self.restore
        };
        if let Some(button) = action.upgrade() {
            button.set_label(if enable {
                "Completing setup…"
            } else {
                "Restoring defaults…"
            });
        }
    }

    fn apply(self: &Rc<Self>, enable: bool) {
        if self.busy.get() || !self.known.get() {
            return;
        }
        if SETUP_RUNNING.replace(true) {
            self.message(
                "Another integration change is running. Try again when it finishes.",
                true,
            );
            return;
        }
        self.busy.set(true);
        self.show_progress(enable);
        let hold = self
            .complete
            .upgrade()
            .and_then(|button| button.root().and_downcast::<gtk::Window>())
            .and_then(|window| window.application())
            .map(|application| application.hold());
        let summary = self.clone();
        glib::spawn_future_local(async move {
            let _hold = hold;
            let result = gio::spawn_blocking(move || {
                if enable {
                    portal_setup::install()?;
                    portal_setup::install_file_manager()?;
                } else {
                    portal_setup::uninstall_file_manager()?;
                    portal_setup::uninstall()?;
                }
                Ok::<_, String>(())
            })
            .await;
            SETUP_RUNNING.set(false);
            summary.busy.set(false);
            let failure = match result {
                Ok(Ok(())) => None,
                Ok(Err(error)) => Some(error),
                Err(error) => Some(format!("Integration change failed: {error:?}")),
            };
            summary.reload(failure);
        });
    }
}

struct IntegrationIndicator {
    row: glib::WeakRef<gtk::Box>,
    icon: glib::WeakRef<gtk::Image>,
    name: &'static str,
}

impl IntegrationIndicator {
    fn new(parent: &gtk::Box, name: &'static str) -> Self {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let icon = crate::assets::primary_icon(icons::X, 16);
        let label = gtk::Label::new(Some(name));
        label.set_xalign(0.0);
        label.set_wrap(true);
        row.append(&icon);
        row.append(&label);
        row.set_visible(false);
        parent.append(&row);
        Self {
            row: row.downgrade(),
            icon: icon.downgrade(),
            name,
        }
    }

    fn update(&self, configured: Option<bool>) {
        if let Some(row) = self.row.upgrade() {
            row.set_visible(configured.is_some());
            if let Some(configured) = configured {
                let status = if configured {
                    "Configured"
                } else {
                    "Not configured"
                };
                let description = format!("{}: {status}", self.name);
                row.set_tooltip_text(Some(&description));
                row.update_property(&[gtk::accessible::Property::Label(&description)]);
                if let Some(icon) = self.icon.upgrade() {
                    crate::assets::set_primary_icon(
                        &icon,
                        if configured { icons::CHECK } else { icons::X },
                    );
                }
            }
        }
    }
}
