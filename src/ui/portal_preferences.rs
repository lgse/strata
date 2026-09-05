// SPDX-License-Identifier: GPL-3.0-or-later

use std::{cell::Cell, rc::Rc, time::Duration};

use gtk::{gio, glib, prelude::*};

use super::{
    blur::BlurBin,
    browser::{dismiss_modal_layer, modal_layer},
    controls::{ModalLayout, ModalTone, message_dialog_layout},
};
use crate::{assets::icons, portal_setup};

#[cfg(test)]
mod tests;

thread_local! {
    static OFFER_SCHEDULED: Cell<bool> = const { Cell::new(false) };
    static SETUP_RUNNING: Cell<bool> = const { Cell::new(false) };
}

pub(super) fn schedule_offer(window: &gtk::ApplicationWindow) {
    if OFFER_SCHEDULED.replace(true) {
        return;
    }
    let window = window.downgrade();
    glib::timeout_add_local(Duration::from_secs(2), move || {
        let Some(window) = window.upgrade() else {
            OFFER_SCHEDULED.set(false);
            return glib::ControlFlow::Break;
        };
        if !window.is_active() || super::window::visible_modal_layer(&window).is_some() {
            return glib::ControlFlow::Continue;
        }
        let window = window.downgrade();
        glib::spawn_future_local(async move {
            match gio::spawn_blocking(portal_setup::take_prompt_offer).await {
                Ok(Ok(true)) => {
                    if let Some(window) = window.upgrade() {
                        show_dialog(window.upcast_ref(), true);
                    }
                }
                Ok(Err(error)) => {
                    tracing::warn!(%error, "Could not offer file chooser integration")
                }
                Err(error) => tracing::warn!(?error, "File chooser offer task failed"),
                Ok(Ok(false)) => {}
            }
        });
        glib::ControlFlow::Break
    });
}

pub(super) fn settings_row() -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 16);
    row.add_css_class("settings-option");
    let copy = gtk::Box::new(gtk::Orientation::Vertical, 2);
    copy.set_hexpand(true);
    let title = gtk::Label::new(Some("System file chooser"));
    title.set_xalign(0.0);
    title.add_css_class("settings-option-title");
    let description = gtk::Label::new(Some(
        "Use Strata for Open and Save dialogs in portal-aware apps, or restore your previous chooser. This is separate from your default file manager.",
    ));
    description.set_xalign(0.0);
    description.set_wrap(true);
    description.add_css_class("settings-option-description");
    let configure = gtk::Button::with_label("Configure…");
    configure.add_css_class("action-dialog-cancel");
    configure.set_halign(gtk::Align::Start);
    configure.set_valign(gtk::Align::Center);
    configure.connect_clicked(|button| {
        if let Some(window) = button.root().and_downcast::<gtk::Window>() {
            show_dialog(&window, false);
        }
    });
    copy.append(&title);
    copy.append(&description);
    row.append(&copy);
    row.append(&configure);
    row
}

struct Dialog {
    layer: glib::WeakRef<gtk::Box>,
    overlay: glib::WeakRef<gtk::Overlay>,
    root: Option<glib::WeakRef<BlurBin>>,
    confirm: glib::WeakRef<gtk::Button>,
    cancel: glib::WeakRef<gtk::Button>,
    close: glib::WeakRef<gtk::Button>,
    status: glib::WeakRef<gtk::Label>,
    description: glib::WeakRef<gtk::Label>,
    success: glib::WeakRef<gtk::Image>,
    loading: glib::WeakRef<gtk::Spinner>,
    busy: Rc<Cell<bool>>,
    enable: Cell<Option<bool>>,
    finished: Cell<bool>,
}

impl Dialog {
    fn dismiss(&self) {
        if !self.busy.get()
            && let (Some(layer), Some(overlay)) = (self.layer.upgrade(), self.overlay.upgrade())
        {
            let root = self.root.as_ref().and_then(glib::WeakRef::upgrade);
            dismiss_modal_layer(&layer, &overlay, root.as_ref());
        }
    }

    fn set_busy(&self, busy: bool) {
        self.busy.set(busy);
        for button in [&self.confirm, &self.cancel, &self.close] {
            if let Some(button) = button.upgrade() {
                button.set_sensitive(!busy);
            }
        }
        if let Some(loading) = self.loading.upgrade() {
            loading.set_visible(busy);
            loading.set_spinning(busy);
        }
    }

    fn message(&self, message: &str, error: bool) {
        if let Some(status) = self.status.upgrade() {
            status.set_text(
                &message
                    .lines()
                    .map(|line| {
                        super::controls::wrap_dialog_text(
                            line,
                            super::controls::MESSAGE_DIALOG_WIDTH_CHARS,
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n"),
            );
            if error {
                status.add_css_class("error");
            } else {
                status.remove_css_class("error");
            }
        }
    }

    fn complete(&self, message: &str) {
        self.finished.set(true);
        self.message(message, false);
        if let Some(description) = self.description.upgrade() {
            description.set_visible(false);
        }
        if let Some(success) = self.success.upgrade() {
            success.set_visible(true);
        }
        if let Some(status) = self.status.upgrade() {
            status.set_xalign(0.5);
            status.set_justify(gtk::Justification::Center);
        }
        if let Some(confirm) = self.confirm.upgrade() {
            confirm.set_label("Done");
            confirm.grab_focus();
        }
        if let Some(cancel) = self.cancel.upgrade() {
            cancel.set_visible(false);
        }
    }

    fn load(self: &Rc<Self>) {
        self.set_busy(true);
        let dialog = self.clone();
        glib::spawn_future_local(async move {
            let result = gio::spawn_blocking(|| {
                portal_setup::dismiss_prompt()?;
                portal_setup::status()
            })
            .await;
            dialog.set_busy(false);
            match result {
                Ok(Ok(status)) => {
                    dialog.enable.set(Some(!status.configured));
                    dialog.message(if status.configured {
                        "Strata is currently configured as your preferred file chooser. Restoring removes its integration and preserves unrelated configuration edits."
                    } else if status.has_installation {
                        "Strata integration exists, but it is not your preferred chooser. You can update its configuration below."
                    } else {
                        "Your current file chooser has not been changed. You can enable Strata now or later in Settings → General → System file chooser."
                    }, false);
                    if let Some(confirm) = dialog.confirm.upgrade() {
                        confirm.set_label(if status.configured {
                            "Restore previous chooser"
                        } else {
                            "Use Strata"
                        });
                    }
                }
                Ok(Err(error)) => dialog.load_failed(&error),
                Err(error) => dialog.load_failed(&format!(
                    "Could not read file chooser configuration: {error:?}"
                )),
            }
            if let Some(cancel) = dialog.cancel.upgrade() {
                cancel.grab_focus();
            }
        });
    }

    fn load_failed(&self, error: &str) {
        self.message(error, true);
        if let Some(confirm) = self.confirm.upgrade() {
            confirm.set_label("Retry");
        }
    }

    fn apply(self: &Rc<Self>) {
        if self.busy.get() {
            return;
        }
        if self.finished.get() {
            self.dismiss();
            return;
        }
        let Some(enable) = self.enable.get() else {
            self.load();
            return;
        };
        if SETUP_RUNNING.replace(true) {
            self.message(
                "Another file chooser configuration is running. Try again when it finishes.",
                true,
            );
            return;
        }
        self.set_busy(true);
        self.message(
            if enable {
                "Configuring Strata and restarting the portal service…"
            } else {
                "Restoring your previous chooser and restarting the portal service…"
            },
            false,
        );
        let hold = self
            .confirm
            .upgrade()
            .and_then(|button| button.root().and_downcast::<gtk::Window>())
            .and_then(|window| window.application())
            .map(|application| application.hold());
        let dialog = self.clone();
        glib::spawn_future_local(async move {
            let _hold = hold;
            let result = gio::spawn_blocking(move || {
                if enable {
                    portal_setup::install()
                } else {
                    portal_setup::uninstall()
                }
            })
            .await;
            SETUP_RUNNING.set(false);
            dialog.set_busy(false);
            match result {
                Ok(Ok(message)) => dialog.complete(&message),
                Ok(Err(error)) => dialog.message(&error, true),
                Err(error) => {
                    dialog.message(&format!("File chooser setup failed: {error:?}"), true)
                }
            }
        });
    }
}

fn show_dialog(parent: &gtk::Window, offer: bool) {
    if let Some(dialog) = build_dialog(parent, offer) {
        dialog.load();
    }
}

fn build_dialog(parent: &gtk::Window, offer: bool) -> Option<Rc<Dialog>> {
    let overlay = parent.child().and_downcast::<gtk::Overlay>()?;
    let root = overlay.child().and_downcast::<BlurBin>();
    let layout: ModalLayout = message_dialog_layout(
        icons::FOLDER,
        if offer {
            "Use Strata as your file chooser?"
        } else {
            "System file chooser"
        },
        "Open and Save dialogs, with your familiar Strata views.",
        "Use Strata",
        ModalTone::Accent,
    );
    layout.content.add_css_class("portal-setup-dialog");
    layout
        .cancel
        .set_label(if offer { "Not now" } else { "Cancel" });
    let description = gtk::Label::new(Some(&super::controls::wrap_dialog_text(
        "Only apps using the desktop FileChooser portal are affected. Requires xdg-desktop-portal. Changing this setting restarts the portal service; close any open file dialogs first. Your default file manager and other portals are unchanged.",
        super::controls::MESSAGE_DIALOG_WIDTH_CHARS,
    )));
    description.set_xalign(0.0);
    description.set_wrap(true);
    description.set_max_width_chars(super::controls::MESSAGE_DIALOG_WIDTH_CHARS as i32);
    description.add_css_class("settings-option-description");
    layout.body.append(&description);
    let success = crate::assets::primary_icon(icons::CIRCLE_CHECK, 48);
    success.set_halign(gtk::Align::Center);
    success.set_visible(false);
    layout.body.append(&success);
    let status = gtk::Label::new(Some("Checking your current chooser…"));
    status.set_xalign(0.0);
    status.set_wrap(true);
    status.set_max_width_chars(super::controls::MESSAGE_DIALOG_WIDTH_CHARS as i32);
    status.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    status.add_css_class("form-message");
    layout.body.append(&status);
    let busy = Rc::new(Cell::new(false));
    let blocked = busy.clone();
    let layer = modal_layer(
        &layout.content,
        &overlay,
        root.clone(),
        Some(Rc::new(move || blocked.get())),
    );
    let dialog = Rc::new(Dialog {
        layer: layer.downgrade(),
        overlay: overlay.downgrade(),
        root: root.as_ref().map(ObjectExt::downgrade),
        confirm: layout.confirm.downgrade(),
        cancel: layout.cancel.downgrade(),
        close: layout.close.downgrade(),
        status: status.downgrade(),
        description: description.downgrade(),
        success: success.downgrade(),
        loading: layout.loading.downgrade(),
        busy,
        enable: Cell::new(None),
        finished: Cell::new(false),
    });
    for button in [&layout.cancel, &layout.close] {
        let dialog = dialog.clone();
        button.connect_clicked(move |_| dialog.dismiss());
    }
    let action = dialog.clone();
    layout.confirm.connect_clicked(move |_| action.apply());
    let escaped = dialog.clone();
    let keys = gtk::EventControllerKey::new();
    keys.connect_key_pressed(move |_, key, _, _| {
        if key == gtk::gdk::Key::Escape {
            escaped.dismiss();
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    layer.add_controller(keys);
    if let Some(root) = root {
        root.set_blurred(true);
    }
    overlay.add_overlay(&layer);
    Some(dialog)
}
