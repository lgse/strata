// SPDX-License-Identifier: MIT

//! The progress overlay a release shows over its window, and its pending sidebar row.

use std::{cell::Cell, rc::Rc, time::Duration};

use gtk::prelude::*;

use crate::ui::{
    controls::{message_dialog_description, modal_layout},
    modal::{ModalHost, close_on_escape, dismiss_modal_layer, modal_layer, remember_modal_focus},
};

use super::{
    PendingRelease, RELEASE_BODY, RELEASE_ROW_TOOLTIP, ReleaseKey, ReleaseKind, ReleasePhase,
    keys_match, with_releases,
};

pub(super) const RELEASE_PROGRESS_DELAY: Duration = Duration::from_millis(350);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum OverlayMode {
    Hide,
    Dismiss,
}

pub(super) struct ReleaseOverlay {
    pub(super) layer: gtk::Box,
    pub(super) host_overlay: gtk::Overlay,
    pub(super) blurred_root: Option<crate::ui::blur::BlurBin>,
    pub(super) title: gtk::Label,
    pub(super) body: gtk::Label,
    pub(super) confirm: gtk::Button,
    pub(super) loading: gtk::Spinner,
    pub(super) mode: Rc<Cell<OverlayMode>>,
}

pub(in crate::ui::window) fn pending_device_shell(row: &gtk::Button) -> gtk::Box {
    row.set_sensitive(false);
    let busy_label = crate::i18n::tr(RELEASE_ROW_TOOLTIP);
    crate::ui::accessibility::set_own_label(row, &busy_label);
    let spinner = gtk::Spinner::new();
    spinner.add_css_class("sidebar-device-spinner");
    spinner.update_property(&[gtk::accessible::Property::Label(&busy_label)]);
    spinner.set_hexpand(true);
    spinner.set_halign(gtk::Align::End);
    spinner.set_valign(gtk::Align::Center);
    spinner.set_size_request(14, 14);
    spinner.start();
    let slot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    slot.add_css_class("sidebar-device-action");
    slot.set_width_request(24);
    slot.append(&spinner);
    let shell = super::super::sidebar_device_row(row, None, None);
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    actions.add_css_class("sidebar-device-actions");
    actions.set_hexpand(false);
    actions.set_halign(gtk::Align::End);
    actions.set_valign(gtk::Align::Center);
    actions.append(&slot);
    shell.append(&actions);
    shell
}

pub(in crate::ui::window) fn note_unmount_progress(
    key: &ReleaseKey,
    message: &str,
    _time_left: i64,
    _bytes_left: i64,
) {
    let trimmed = message.trim();
    with_releases(|releases| {
        let Some(entry) = releases
            .iter_mut()
            .find(|entry| keys_match(&entry.key, key))
        else {
            return;
        };
        if trimmed.is_empty() {
            return;
        }
        entry.progress_message = Some(trimmed.to_owned());
        if let Some(overlay) = entry.overlay.as_ref() {
            overlay
                .body
                .set_text(&body_for_progress(entry.progress_message.as_deref()));
        }
    });
}

pub(super) fn body_for_progress(message: Option<&str>) -> String {
    match message.map(str::trim).filter(|message| !message.is_empty()) {
        Some(message) => {
            rust_i18n::t!("%{message}\nDo not unplug it.", message = message).into_owned()
        }
        None => crate::i18n::tr(RELEASE_BODY),
    }
}

pub(in crate::ui::window) fn hide_release(key: &ReleaseKey) {
    let overlay = with_releases(|releases| {
        let entry = releases
            .iter_mut()
            .find(|entry| keys_match(&entry.key, key))?;
        entry.dismissed = true;
        if let Some(source) = entry.present_source.take() {
            source.remove();
        }
        entry.overlay.take()
    });
    if let Some(overlay) = overlay {
        dismiss_modal_layer(
            &overlay.layer,
            &overlay.host_overlay,
            overlay.blurred_root.as_ref(),
        );
    }
}

pub(in crate::ui::window) fn present_release_overlay(parent: &gtk::Widget, key: &ReleaseKey) {
    let Some(snapshot) = claim_overlay(parent, key) else {
        return;
    };
    let Some(host) = ModalHost::blurred_for(parent) else {
        return;
    };
    let overlay = build_overlay(&host, &snapshot, key);
    if let Some(unclaimed) = store_overlay(key, overlay) {
        dismiss_modal_layer(
            &unclaimed.layer,
            &unclaimed.host_overlay,
            unclaimed.blurred_root.as_ref(),
        );
    }
}

/// What the overlay shows, read while the release is claimed for it.
struct OverlaySnapshot {
    kind: ReleaseKind,
    display_name: String,
    progress: Option<String>,
}

/// Claims the pending release for an overlay unless it was hidden, already
/// shows one or has finished.
fn claim_overlay(parent: &gtk::Widget, key: &ReleaseKey) -> Option<OverlaySnapshot> {
    with_releases(|releases| {
        let entry = releases
            .iter_mut()
            .find(|entry| keys_match(&entry.key, key))?;
        entry.present_source = None;
        if !wants_overlay(entry) {
            return None;
        }
        entry.parent = Some(parent.clone());
        Some(OverlaySnapshot {
            kind: entry.kind,
            display_name: entry.key.display_name.clone(),
            progress: entry.progress_message.clone(),
        })
    })
}

fn wants_overlay(entry: &PendingRelease) -> bool {
    if entry.dismissed || entry.overlay.is_some() {
        return false;
    }
    entry.phase != ReleasePhase::FinishedOk
}

fn build_overlay(host: &ModalHost, snapshot: &OverlaySnapshot, key: &ReleaseKey) -> ReleaseOverlay {
    let title = snapshot.kind.title();
    let layout = modal_layout(
        snapshot.kind.icon(),
        &title,
        &snapshot.display_name,
        &crate::i18n::tr("Hide"),
    );
    layout.content.add_css_class("compact");
    layout.set_loading(true, Some(&title));
    layout.cancel.set_visible(false);
    let body = message_dialog_description(&body_for_progress(snapshot.progress.as_deref()));
    layout.body.append(&body);
    let layer = modal_layer(
        &layout.content,
        &host.overlay,
        host.blurred_root.clone(),
        Some(Rc::new(|| true)),
    );
    remember_modal_focus(&layer, &host.overlay);
    host.overlay.add_overlay(&layer);

    let mode = Rc::new(Cell::new(OverlayMode::Hide));
    let hide = hide_action(key, &mode, &layer, host);
    let hide_clicked = hide.clone();
    layout.confirm.connect_clicked(move |_| hide_clicked());
    let hide_closed = hide.clone();
    layout.close.connect_clicked(move |_| hide_closed());
    close_on_escape(&layer, move || hide());
    layout.confirm.grab_focus();
    ReleaseOverlay {
        layer,
        host_overlay: host.overlay.clone(),
        blurred_root: host.blurred_root.clone(),
        title: layout.title,
        body,
        confirm: layout.confirm,
        loading: layout.loading,
        mode,
    }
}

/// Hide keeps the release running in the background; once it is safe to
/// remove, the same controls just close the overlay.
fn hide_action(
    key: &ReleaseKey,
    mode: &Rc<Cell<OverlayMode>>,
    layer: &gtk::Box,
    host: &ModalHost,
) -> Rc<dyn Fn()> {
    let hide_key = key.clone();
    let mode = mode.clone();
    let layer = layer.clone();
    let overlay = host.overlay.clone();
    let root = host.blurred_root.clone();
    Rc::new(move || {
        if mode.get() == OverlayMode::Hide {
            hide_release(&hide_key);
        } else {
            dismiss_modal_layer(&layer, &overlay, root.as_ref());
        }
    })
}

/// Hands the overlay back when the release was hidden or finished meanwhile.
fn store_overlay(key: &ReleaseKey, overlay: ReleaseOverlay) -> Option<ReleaseOverlay> {
    with_releases(|releases| {
        let Some(entry) = releases
            .iter_mut()
            .find(|entry| keys_match(&entry.key, key))
        else {
            return Some(overlay);
        };
        if entry.dismissed {
            return Some(overlay);
        }
        entry.overlay = Some(overlay);
        None
    })
}
pub(super) fn schedule_present(parent: gtk::Widget, key: ReleaseKey) {
    let present_key = key.clone();
    let source = glib::timeout_add_local_once(RELEASE_PROGRESS_DELAY, move || {
        present_release_overlay(&parent, &present_key);
    });
    with_releases(|releases| {
        if let Some(entry) = releases
            .iter_mut()
            .find(|entry| keys_match(&entry.key, &key))
        {
            if let Some(previous) = entry.present_source.replace(source) {
                previous.remove();
            }
        } else {
            source.remove();
        }
    });
}

pub(super) fn show_safe_to_remove(overlay: &ReleaseOverlay, display_name: &str) {
    overlay.mode.set(OverlayMode::Dismiss);
    overlay.title.set_text(&crate::i18n::tr("Safe to remove"));
    overlay.loading.stop();
    overlay.loading.set_visible(false);
    crate::ui::accessibility::set_description(&overlay.loading, None);
    overlay.body.set_text(&rust_i18n::t!(
        "%{display_name} can be unplugged.",
        display_name = display_name
    ));
    overlay.confirm.set_label(&crate::i18n::tr("Close"));
}

pub(in crate::ui::window) fn mount_operation(
    parent: &gtk::Widget,
    key: &ReleaseKey,
) -> gtk::MountOperation {
    let window = parent.root().and_downcast::<gtk::Window>();
    let operation = gtk::MountOperation::new(window.as_ref());
    let key = key.clone();
    operation.connect_show_unmount_progress(move |operation, message, time_left, bytes_left| {
        operation.stop_signal_emission_by_name("show-unmount-progress");
        note_unmount_progress(&key, message, time_left, bytes_left);
    });
    operation
}
