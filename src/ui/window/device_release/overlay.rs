// SPDX-License-Identifier: MIT

//! The progress overlay a release shows over its window, and its pending sidebar row.

use std::{cell::Cell, rc::Rc, time::Duration};

use gtk::prelude::*;

use crate::ui::{
    controls::{message_dialog_description, modal_layout},
    modal::{ModalHost, close_on_escape, dismiss_modal_layer, modal_layer, remember_modal_focus},
};

use super::{
    RELEASE_BODY, RELEASE_ROW_TOOLTIP, ReleaseKey, ReleasePhase, keys_match, with_releases,
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
    spinner.set_hexpand(false);
    spinner.set_halign(gtk::Align::Center);
    spinner.set_valign(gtk::Align::Center);
    spinner.set_size_request(14, 14);
    spinner.start();
    let slot = gtk::CenterBox::new();
    slot.add_css_class("sidebar-device-action");
    slot.set_width_request(24);
    slot.set_center_widget(Some(&spinner));
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
    let snapshot = with_releases(|releases| {
        let entry = releases
            .iter_mut()
            .find(|entry| keys_match(&entry.key, key))?;
        entry.present_source = None;
        if entry.dismissed || entry.overlay.is_some() || entry.phase == ReleasePhase::FinishedOk {
            return None;
        }
        entry.parent = Some(parent.clone());
        Some((
            entry.kind,
            entry.key.display_name.clone(),
            entry.progress_message.clone(),
        ))
    });
    let Some((kind, display_name, progress)) = snapshot else {
        return;
    };
    let Some(host) = ModalHost::blurred_for(parent) else {
        return;
    };
    let title = kind.title();
    let layout = modal_layout(kind.icon(), &title, &display_name, &crate::i18n::tr("Hide"));
    layout.content.add_css_class("compact");
    layout.set_loading(true, Some(&title));
    layout.cancel.set_visible(false);
    let body = message_dialog_description(&body_for_progress(progress.as_deref()));
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
    let hide_key = key.clone();
    let layer_for_hide = layer.clone();
    let overlay_for_hide = host.overlay.clone();
    let root_for_hide = host.blurred_root.clone();
    let hide: Rc<dyn Fn()> = Rc::new({
        let mode = mode.clone();
        move || {
            if mode.get() == OverlayMode::Hide {
                hide_release(&hide_key);
            } else {
                dismiss_modal_layer(&layer_for_hide, &overlay_for_hide, root_for_hide.as_ref());
            }
        }
    });
    let hide_clicked = hide.clone();
    layout.confirm.connect_clicked(move |_| hide_clicked());
    let hide_closed = hide.clone();
    layout.close.connect_clicked(move |_| hide_closed());
    close_on_escape(&layer, move || hide());
    layout.confirm.grab_focus();

    let stored = with_releases(|releases| {
        let Some(entry) = releases
            .iter_mut()
            .find(|entry| keys_match(&entry.key, key))
        else {
            return false;
        };
        if entry.dismissed {
            return false;
        }
        entry.overlay = Some(ReleaseOverlay {
            layer: layer.clone(),
            host_overlay: host.overlay.clone(),
            blurred_root: host.blurred_root.clone(),
            title: layout.title,
            body,
            confirm: layout.confirm,
            loading: layout.loading,
            mode,
        });
        true
    });
    if !stored {
        dismiss_modal_layer(&layer, &host.overlay, host.blurred_root.as_ref());
    }
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
