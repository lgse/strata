// SPDX-License-Identifier: MIT

//! Translations for [`strata_components::progress_dock::Words`], sourced from `strata_core::i18n`.

use std::rc::Rc;

use strata_components::progress_dock::Words;
use strata_core::i18n::{count, duration, file_size, integer, percent, tr, transfer_rate};

pub(super) fn translated() -> Words {
    Words {
        size: Rc::new(file_size),
        rate: Rc::new(transfer_rate),
        duration: Rc::new(duration),
        integer: Rc::new(integer),
        percent: Rc::new(|value| percent(value as usize)),
        speed_left: Rc::new(|speed, time| {
            rust_i18n::t!("%{speed} · %{time} left", speed = speed, time = time).into_owned()
        }),
        bytes_of: Rc::new(|done, total| format!("{done} / {total}")),
        count_of: Rc::new(|done, total| format!("{done}/{total}")),
        of_total: Rc::new(|done, total, files| {
            let kind = if files { "files" } else { "items" };
            rust_i18n::t!(
                "%{completed} of %{items}",
                completed = integer(done as u64),
                items = count(kind, total)
            )
            .into_owned()
        }),
        with_current: Rc::new(|items, name| format!("{items} · {name}")),
        calculating_speed: tr("Calculating speed…"),
        preparing_items: tr("Preparing items…"),
        preparing: tr("Preparing…"),
        transferring: tr("Transferring…"),
        busy_mark: String::from("…"),
        cancelling_transfer: tr("Cancelling transfer…"),
        cancelling_operation: tr("Cancelling operation…"),
        still_writing: tr("Device may still be writing. Do not unplug until the operation stops."),
        waiting_for_write: tr(
            "Waiting for the active write to stop. The device may still be writing.",
        ),
        waiting_for_device: tr("Waiting for device…"),
        not_responding: tr("Device not responding"),
        not_responding_detail: tr(
            "Cancellation is still pending. The device may still be writing; do not unplug it.",
        ),
        cancel_requested: tr("Cancellation requested"),
        cancel: tr("Cancel"),
        cancel_operation: tr("Cancel operation"),
        dismiss: tr("Dismiss notification"),
        keep: tr("Keep notification"),
        complete: tr("Complete"),
        complete_label: tr("Dismiss completed notification"),
        closing_in: Rc::new(|time| rust_i18n::t!("closing in %{time}", time = time).into_owned()),
        paused: tr("paused"),
        pinned: tr("pinned"),
        countdown_description: tr("Time remaining before notification closes"),
        region: tr("Operations"),
        transferred: tr("Transferred"),
    }
}
