//! [`Flash`]: a short footer message, such as "Unknown chord" or "No matches for “x”", that
//! goes 2 s after it shows (Strata's `show_feedback`, `FEEDBACK_FLASH`).

use std::time::Duration;

use rune::{El, LiveRegion, Tx, row, text};
use strata_kit::look::Look;

/// How long a flash shows (Strata's 2 s).
pub const FLASH_FOR: f64 = 2.0;

/// The footer's flash, held by the host. A new flash replaces the one showing and starts its
/// 2 s again.
///
/// [`Flash::show`] asks for a wake when the flash is due to go, so it goes in a window at rest
/// too, and the host's poll calls [`Flash::tick`] to end it.
///
/// Used by: hosts, C19 (which places it).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Flash {
    /// The words and when they showed, on `Tx::clock`.
    shown: Option<(String, f64)>,
}

impl Flash {
    /// Nothing showing.
    pub fn new() -> Self {
        Self::default()
    }

    /// The words showing.
    pub fn text(&self) -> Option<&str> {
        self.shown.as_ref().map(|(t, _)| t.as_str())
    }

    /// Shows `words` for [`FLASH_FOR`] from now, and asks for a wake on `tx` when they're due
    /// to go. Empty words clear it.
    pub fn show(&mut self, words: impl Into<String>, tx: &mut Tx) {
        let words = words.into();
        self.shown = (!words.is_empty()).then_some((words, tx.clock()));
        if self.shown.is_some() {
            tx.wake_at(tx.clock() + FLASH_FOR);
        }
    }

    /// Clears it at once.
    pub fn clear(&mut self) {
        self.shown = None;
    }

    /// One frame at `now` seconds on the window's clock: asks again for the time left, and ends
    /// the flash once due. True when it ended.
    pub fn tick(&mut self, now: f64, tx: &mut Tx) -> bool {
        match self.left(now) {
            Some(left) if left <= 1e-6 => {
                self.shown = None;
                true
            }
            Some(left) => {
                tx.wake_after(Duration::from_secs_f64(left));
                false
            }
            None => false,
        }
    }

    /// The seconds left at `now`: `None` with nothing showing.
    fn left(&self, now: f64) -> Option<f64> {
        let (_, at) = self.shown.as_ref()?;
        Some(FLASH_FOR - (now - at))
    }
}

/// The flash's element for the footer, or `None` with nothing showing: the text colour, as
/// Strata's `.shortcut-footer-feedback`, and a polite live region.
pub fn flash(look: &Look, key: &str, state: &Flash) -> Option<El> {
    let words = state.text()?;
    Some(
        row()
            .key(key.to_owned())
            .role("status")
            .label(words.to_owned())
            .live_region(LiveRegion::Polite)
            .center_y()
            .child(
                text(words.to_owned())
                    .font(look.kit.ty.small)
                    .color(look.kit.c.text_base)
                    .truncate()
                    .a11y_hidden(),
            ),
    )
}
