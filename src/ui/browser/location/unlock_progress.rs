// SPDX-License-Identifier: MIT

use super::devices::{DeviceKeys, unlock_target_matches};
use super::dialogs::LocationModal;
use crate::ui::browser::ViewState;
use crate::ui::controls::{message_dialog_description, modal_layout};
use crate::ui::modal::{ModalHost, close_on_escape};
use gtk::glib;
use gtk::prelude::*;
use std::rc::Rc;
use std::time::Duration;

pub(super) const UNLOCK_PROGRESS_DELAY: Duration = Duration::from_millis(350);

pub(in crate::ui::browser) struct UnlockProgressSlot {
    pub(super) keys: DeviceKeys,
    pub(super) view: Option<LocationModal>,
    pub(super) pending: Option<glib::SourceId>,
    pub(super) dismissed: bool,
    pub(super) in_flight: bool,
}

pub(super) fn begin_unlock_slot(slots: &mut Vec<UnlockProgressSlot>, keys: &DeviceKeys) -> bool {
    if let Some(slot) = slots
        .iter_mut()
        .find(|slot| unlock_target_matches(&slot.keys, keys))
    {
        if slot.in_flight {
            return false;
        }
        slot.in_flight = true;
        slot.dismissed = false;
        return true;
    }
    slots.push(UnlockProgressSlot {
        keys: keys.clone(),
        view: None,
        pending: None,
        dismissed: false,
        in_flight: true,
    });
    true
}

pub(super) fn unlock_progress_dismissed_for(
    slots: &[UnlockProgressSlot],
    keys: &DeviceKeys,
) -> bool {
    slots
        .iter()
        .find(|slot| unlock_target_matches(&slot.keys, keys))
        .is_some_and(|slot| slot.dismissed)
}

fn unlock_progress_layout(volume_name: &str) -> crate::ui::controls::ModalLayout {
    let layout = modal_layout(
        crate::assets::icons::LOCK,
        &crate::i18n::tr("Unlocking volume"),
        volume_name,
        &crate::i18n::tr("Hide"),
    );
    layout.content.add_css_class("compact");
    layout.set_loading(true, Some(&crate::i18n::tr("Unlocking volume")));
    layout.cancel.set_visible(false);
    layout
        .body
        .append(&message_dialog_description(&crate::i18n::tr(
            "You can hide this and keep working. Unlocking will continue in the background.",
        )));
    layout
}

impl ViewState {
    /// Runs `update` on the slot tracking `keys`, if one does.
    fn with_unlock_slot<T>(
        &self,
        keys: &DeviceKeys,
        update: impl FnOnce(&mut UnlockProgressSlot) -> T,
    ) -> Option<T> {
        self.unlock_slots
            .borrow_mut()
            .iter_mut()
            .find(|slot| unlock_target_matches(&slot.keys, keys))
            .map(update)
    }

    fn unlock_progress_dismissed(&self, keys: &DeviceKeys) -> bool {
        unlock_progress_dismissed_for(&self.unlock_slots.borrow(), keys)
    }

    pub(super) fn begin_unlock_progress(&self, keys: &DeviceKeys) -> bool {
        begin_unlock_slot(&mut self.unlock_slots.borrow_mut(), keys)
    }

    pub(super) fn schedule_device_mount_chrome(
        self: &Rc<Self>,
        keys: &DeviceKeys,
        volume_name: &str,
        encrypted: bool,
    ) {
        if !encrypted {
            return;
        }
        self.schedule_unlock_progress(keys, volume_name);
    }

    pub(super) fn schedule_unlock_progress(self: &Rc<Self>, keys: &DeviceKeys, volume_name: &str) {
        self.dismiss_unlock_progress(keys);
        if self.unlock_progress_dismissed(keys) {
            return;
        }
        let weak = Rc::downgrade(self);
        let volume_name = volume_name.to_owned();
        let keys = keys.clone();
        let source = glib::timeout_add_local_once(UNLOCK_PROGRESS_DELAY, {
            let keys = keys.clone();
            move || {
                if let Some(state) = weak.upgrade() {
                    state.with_unlock_slot(&keys, |slot| slot.pending = None);
                    state.present_unlock_progress(&keys, &volume_name);
                }
            }
        });
        let mut source = Some(source);
        self.with_unlock_slot(&keys, |slot| {
            if let Some(previous) = slot.pending.take() {
                previous.remove();
            }
            slot.pending = source.take();
        });
        if let Some(source) = source {
            source.remove();
        }
    }

    fn hide_unlock_progress_on(self: &Rc<Self>, keys: &DeviceKeys) -> impl Fn() + 'static {
        let weak = Rc::downgrade(self);
        let keys = keys.clone();
        move || {
            if let Some(state) = weak.upgrade() {
                state.hide_unlock_progress(&keys);
            }
        }
    }

    pub(super) fn present_unlock_progress(self: &Rc<Self>, keys: &DeviceKeys, volume_name: &str) {
        self.dismiss_other_unlock_views(keys);
        self.dismiss_unlock_progress(keys);
        if self.unlock_progress_dismissed(keys) {
            return;
        }
        let Some(host) = ModalHost::blurred_for(&self.overlay) else {
            return;
        };
        let layout = unlock_progress_layout(volume_name);
        let hide = layout.confirm;
        let view = LocationModal::present(&layout.content, host, Rc::new(|| true));
        let hide_on_click = self.hide_unlock_progress_on(keys);
        hide.connect_clicked(move |_| hide_on_click());
        let close_on_click = self.hide_unlock_progress_on(keys);
        layout.close.connect_clicked(move |_| close_on_click());
        close_on_escape(&view.layer, self.hide_unlock_progress_on(keys));
        hide.grab_focus();
        let mut view = Some(view);
        self.with_unlock_slot(keys, |slot| slot.view = view.take());
        if let Some(view) = view {
            view.dismiss();
        }
    }

    fn hide_unlock_progress(&self, keys: &DeviceKeys) {
        self.with_unlock_slot(keys, |slot| slot.dismissed = true);
        self.dismiss_unlock_progress(keys);
    }

    fn dismiss_other_unlock_views(&self, keys: &DeviceKeys) {
        let others: Vec<DeviceKeys> = self
            .unlock_slots
            .borrow()
            .iter()
            .filter(|slot| !unlock_target_matches(&slot.keys, keys))
            .map(|slot| slot.keys.clone())
            .collect();
        for other in others {
            self.dismiss_unlock_progress(&other);
        }
    }

    pub(super) fn dismiss_unlock_progress(&self, keys: &DeviceKeys) {
        let view = self.with_unlock_slot(keys, |slot| {
            if let Some(source) = slot.pending.take() {
                source.remove();
            }
            slot.view.take()
        });
        if let Some(Some(view)) = view {
            view.dismiss();
        }
    }

    pub(super) fn finish_unlock_slot(&self, keys: &DeviceKeys) {
        self.dismiss_unlock_progress(keys);
        self.unlock_slots
            .borrow_mut()
            .retain(|slot| !unlock_target_matches(&slot.keys, keys));
    }
}
