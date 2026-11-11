//! [`Words`]: every phrase the dock, its cards and the progress dialog write, as data. The
//! defaults are Strata's English; Strata passes its own translations.

use std::fmt;
use std::rc::Rc;

use super::format;

/// A phrase built from numbers or other phrases.
pub type Phrase<T> = Rc<dyn Fn(T) -> String>;

/// A phrase built from two others: "1.5 MB/s · 2m 5s left".
pub type Phrase2 = Rc<dyn Fn(&str, &str) -> String>;

/// The dock's and the progress dialog's words. Components carry no translations: Strata
/// fills these from its catalog (`i18n::file_size`, `tr("Calculating speed…")`, …).
#[derive(Clone)]
pub struct Words {
    /// A byte count: "64 MB" ([`format::size`]).
    pub size: Phrase<u64>,
    /// A byte rate: "1.5 MB/s" ([`format::rate`]).
    pub rate: Phrase<u64>,
    /// A duration in whole seconds: "2m 5s" ([`format::duration`]).
    pub duration: Phrase<u64>,
    /// A whole number: "270,325" ([`format::integer`]).
    pub integer: Phrase<u64>,
    /// A whole percentage: "42%".
    pub percent: Phrase<u32>,
    /// The speed and the time left: "1.5 MB/s · 2m 5s left".
    pub speed_left: Phrase2,
    /// Bytes done of the total: "12 MB / 40 MB".
    pub bytes_of: Phrase2,
    /// Done of a total, as the card's count: "3/10".
    pub count_of: Phrase2,
    /// The dialog's item line: "3 of 10 files", or "items" when `files` is false.
    pub of_total: Rc<dyn Fn(usize, usize, bool) -> String>,
    /// The item line and the file being copied: "3 of 10 files · photo.jpg".
    pub with_current: Phrase2,
    pub calculating_speed: String,
    pub preparing_items: String,
    pub preparing: String,
    pub transferring: String,
    /// The right label while the bar pulses.
    pub busy_mark: String,
    pub cancelling_transfer: String,
    pub cancelling_operation: String,
    /// The card's line while a transfer cancels.
    pub still_writing: String,
    /// The dialog's subtitle while a transfer cancels.
    pub waiting_for_write: String,
    /// The speed's place while a transfer cancels.
    pub waiting_for_device: String,
    pub not_responding: String,
    pub not_responding_detail: String,
    /// The dialog's Cancel once pressed.
    pub cancel_requested: String,
    pub cancel: String,
    /// The card's X, running: its tooltip.
    pub cancel_operation: String,
    /// The card's X once it has finished.
    pub dismiss: String,
    pub keep: String,
    pub complete: String,
    pub complete_label: String,
    /// The countdown's words: "closing in 4s".
    pub closing_in: Rc<dyn Fn(&str) -> String>,
    pub paused: String,
    pub pinned: String,
    pub countdown_description: String,
    /// The dock's accessible name.
    pub region: String,
    /// The dialog's bytes heading.
    pub transferred: String,
}

impl fmt::Debug for Words {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Words").finish_non_exhaustive()
    }
}

impl Default for Words {
    fn default() -> Self {
        Words {
            size: Rc::new(format::size),
            rate: Rc::new(format::rate),
            duration: Rc::new(format::duration),
            integer: Rc::new(format::integer),
            percent: Rc::new(format::percent),
            speed_left: Rc::new(|speed, time| format!("{speed} · {time} left")),
            bytes_of: Rc::new(|done, total| format!("{done} / {total}")),
            count_of: Rc::new(|done, total| format!("{done}/{total}")),
            of_total: Rc::new(|done, total, files| {
                let noun = match (files, total) {
                    (true, 1) => "file",
                    (true, _) => "files",
                    (false, 1) => "item",
                    (false, _) => "items",
                };
                format!(
                    "{} of {} {noun}",
                    format::integer(done as u64),
                    format::integer(total as u64)
                )
            }),
            with_current: Rc::new(|items, name| format!("{items} · {name}")),
            calculating_speed: "Calculating speed…".into(),
            preparing_items: "Preparing items…".into(),
            preparing: "Preparing…".into(),
            transferring: "Transferring…".into(),
            busy_mark: "…".into(),
            cancelling_transfer: "Cancelling transfer…".into(),
            cancelling_operation: "Cancelling operation…".into(),
            still_writing: "Device may still be writing. Do not unplug until the operation stops."
                .into(),
            waiting_for_write:
                "Waiting for the active write to stop. The device may still be writing.".into(),
            waiting_for_device: "Waiting for device…".into(),
            not_responding: "Device not responding".into(),
            not_responding_detail:
                "Cancellation is still pending. The device may still be writing; do not unplug it."
                    .into(),
            cancel_requested: "Cancellation requested".into(),
            cancel: "Cancel".into(),
            cancel_operation: "Cancel operation".into(),
            dismiss: "Dismiss notification".into(),
            keep: "Keep notification".into(),
            complete: "Complete".into(),
            complete_label: "Dismiss completed notification".into(),
            closing_in: Rc::new(|time| format!("closing in {time}")),
            paused: "paused".into(),
            pinned: "pinned".into(),
            countdown_description: "Time remaining before notification closes".into(),
            region: "Operations".into(),
            transferred: "Transferred".into(),
        }
    }
}
