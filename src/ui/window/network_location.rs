// SPDX-License-Identifier: MIT

//! Connect-to-server dialog, which stays open until the location opens or fails.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

use gtk::prelude::*;

use crate::{
    adapters::{gio_file_for_location, location_for_file},
    assets, i18n,
    model::Location,
    ui::{
        browser::ConnectOutcome,
        controls::{
            form_check_button, form_entry, form_error_label, form_label, keep_words_whole,
            modal_layout, restart_modal_loading, set_modal_loading,
        },
        modal::{
            ModalHost, close_on_escape, dismiss_modal_layer, modal_layer, remember_modal_focus,
            set_modal_focus_restore, submit_on_enter,
        },
    },
};

use super::{PlaceMenuExtra, SidebarState};

#[cfg(test)]
mod tests;

/// A URI example, not prose, so it is never translated.
const ADDRESS_EXAMPLE: &str = "smb://server/share";

/// GTK's spinner fades in over about a second, so a retry that fails at once
/// would show no spinner and redisplay the same error, as if Connect did nothing.
pub(super) const MIN_CONNECTING: Duration = Duration::from_secs(1);

pub(super) fn menu_extra() -> PlaceMenuExtra {
    PlaceMenuExtra {
        icon: assets::icons::PLUS,
        label: "Connect to server…",
        action: show,
    }
}

/// The controls locked while a connection attempt is in flight.
struct ConnectForm {
    address: gtk::Entry,
    pin: gtk::CheckButton,
    confirm: gtk::Button,
    loading: RefCell<gtk::Spinner>,
    error: gtk::Label,
}

impl ConnectForm {
    fn set_connecting(&self, connecting: bool) {
        self.address.set_sensitive(!connecting);
        self.pin.set_sensitive(!connecting);
        self.confirm.set_sensitive(!connecting);
        if connecting {
            let fresh =
                restart_modal_loading(&self.loading.borrow(), Some(&i18n::tr("Connecting…")));
            self.loading.replace(fresh);
        } else {
            set_modal_loading(&self.loading.borrow(), false, None);
            self.address.grab_focus();
        }
    }

    /// `message` is already localized.
    fn show_error(&self, message: &str) {
        self.error.set_text(message);
        self.error.set_visible(true);
        self.address.add_css_class("error");
    }

    fn clear_error(&self) {
        self.address.remove_css_class("error");
        self.error.set_visible(false);
    }
}

pub(super) fn show(state: &Rc<SidebarState>) {
    let Some(ModalHost {
        overlay: window_overlay,
        blurred_root,
    }) = ModalHost::blurred_for(&state.view.overlay())
    else {
        return;
    };

    let layout = modal_layout(
        assets::icons::NETWORK,
        &i18n::tr("Connect to server"),
        &i18n::tr("Connect to a share or remote server"),
        &i18n::tr("Connect"),
    );
    layout.content.add_css_class("wide");

    let fields = gtk::Box::new(gtk::Orientation::Vertical, 5);
    fields.add_css_class("authentication-fields");
    let address = form_entry();
    address.set_placeholder_text(Some(ADDRESS_EXAMPLE));
    let error = form_error_label();
    error.set_wrap(true);
    error.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    // Wrap within the dialog's width instead of widening it to fit the message.
    error.set_max_width_chars(1);
    keep_words_whole(&error);
    let address_label = form_label(&i18n::tr("Server address"));
    crate::ui::accessibility::set_labelled_by(&address, &[address_label.upcast_ref()]);
    fields.append(&address_label);
    fields.append(&address);
    fields.append(&error);
    layout.body.append(&fields);

    let pin = form_check_button(&i18n::tr("Pin to sidebar"));
    layout.body.append(&pin);

    let form = Rc::new(ConnectForm {
        address,
        pin,
        confirm: layout.confirm.clone(),
        loading: RefCell::new(layout.loading.clone()),
        error,
    });
    {
        let weak_form = Rc::downgrade(&form);
        form.address.connect_changed(move |_| {
            if let Some(form) = weak_form.upgrade() {
                form.clear_error();
            }
        });
    }

    let connecting = Rc::new(Cell::new(false));
    let block_dismiss = {
        let address = form.address.downgrade();
        let connecting = connecting.clone();
        Rc::new(move || {
            connecting.get()
                || address
                    .upgrade()
                    .is_some_and(|address| !address.text().trim().is_empty())
        })
    };
    let layer = modal_layer(
        &layout.content,
        &window_overlay,
        blurred_root.clone(),
        Some(block_dismiss),
    );
    remember_modal_focus(&layer, &window_overlay);
    window_overlay.add_overlay(&layer);
    submit_on_enter(&layout.body, &layout.confirm);
    form.address.grab_focus();

    let dismiss: Rc<dyn Fn()> = Rc::new({
        let layer = layer.clone();
        let overlay = window_overlay.clone();
        move || dismiss_modal_layer(&layer, &overlay, blurred_root.as_ref())
    });
    let cancel: Rc<dyn Fn()> = Rc::new({
        let weak_state = Rc::downgrade(state);
        let connecting = connecting.clone();
        let dismiss = dismiss.clone();
        move || {
            if connecting.replace(false)
                && let Some(state) = weak_state.upgrade()
            {
                state.view.cancel_connect();
            }
            dismiss();
        }
    });
    for button in [&layout.close, &layout.cancel] {
        let cancel = cancel.clone();
        button.connect_clicked(move |_| cancel());
    }
    close_on_escape(&layer, move || cancel());

    let weak_state = Rc::downgrade(state);
    let weak_layer = layer.downgrade();
    layout.confirm.connect_clicked(move |_| {
        let Some(state) = weak_state.upgrade() else {
            return;
        };
        if connecting.replace(true) {
            return;
        }
        form.clear_error();
        form.set_connecting(true);
        let finished = {
            let weak_state = weak_state.clone();
            let weak_form = Rc::downgrade(&form);
            let connecting = connecting.clone();
            let dismiss = dismiss.clone();
            let weak_layer = weak_layer.clone();
            let pin_requested = form.pin.is_active();
            let started = Instant::now();
            move |outcome| match outcome {
                ConnectOutcome::Opened(location) => {
                    connecting.set(false);
                    if let Some(state) = weak_state.upgrade() {
                        if pin_requested {
                            let location = normalized_location(location);
                            let label = pin_label(&location);
                            state.pin_location(location, label);
                        }
                        // Outcomes can arrive mid-dispatch of a browser event.
                        let browser = Rc::downgrade(&state.browser);
                        if let Some(layer) = weak_layer.upgrade() {
                            set_modal_focus_restore(
                                layer.upcast_ref(),
                                Rc::new(move || {
                                    if let Some(browser) = browser.upgrade() {
                                        browser.focus_active();
                                    }
                                }),
                            );
                        }
                    }
                    dismiss();
                }
                ConnectOutcome::Failed(message) => {
                    let connecting = connecting.clone();
                    let weak_form = weak_form.clone();
                    let show = move || {
                        connecting.set(false);
                        if let Some(form) = weak_form.upgrade() {
                            form.set_connecting(false);
                            if let Some(message) = &message {
                                form.show_error(message);
                            }
                        }
                    };
                    match MIN_CONNECTING.checked_sub(started.elapsed()) {
                        Some(remaining) => {
                            gtk::glib::timeout_add_local_once(remaining, show);
                        }
                        None => show(),
                    }
                }
            }
        };
        let input = form.address.text();
        if let Err(error) = state.view.connect_typed_location(input.trim(), finished) {
            connecting.set(false);
            form.set_connecting(false);
            // `message()` is already localized.
            form.show_error(&error.message());
        }
    });
}

fn pin_label(location: &Location) -> String {
    let name = location.display_name();
    if name.is_empty() {
        location.display_path()
    } else {
        name
    }
}

/// The form the bookmark parser reloads a pin in (`smb://host/share` gains a
/// trailing slash), so Unpin can match the row. Falls back to the original if
/// GIO rejects the URI.
fn normalized_location(location: Location) -> Location {
    location_for_file(&gio_file_for_location(&location)).unwrap_or(location)
}
