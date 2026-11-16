// SPDX-License-Identifier: MIT

#[cfg(test)]
use std::time::Duration;
use std::time::Instant;

use gtk::{glib, prelude::*};

#[cfg(test)]
pub(super) const REST_DELAY: Duration = crate::ui::entry_tooltip::REST_DELAY;

#[derive(Default)]
pub(super) struct NameTooltip {
    inner: crate::ui::entry_tooltip::EntryTooltip,
}

impl NameTooltip {
    /// Returns whether the pointer moved far enough to restart the rest delay.
    #[cfg(test)]
    pub(super) fn pointer_moved(&self, x: f64, y: f64, now: Instant) -> bool {
        self.inner.pointer_moved(x, y, now)
    }

    #[cfg(test)]
    pub(super) fn reset(&self) {
        self.inner.reset();
    }

    pub(super) fn text(
        &self,
        card: &gtk::Box,
        x: f64,
        y: f64,
        now: Instant,
    ) -> Option<glib::GString> {
        if !self.inner.is_rested(now) {
            return None;
        }
        let (_, label) = super::parts(card)?;
        if !label.is_visible()
            || !crate::ui::pointer::hits_icon_card_content(card.upcast_ref(), x, y)
        {
            return None;
        }
        let truncated = super::layout::caption_truncated(&label);
        let name = label.text().filter(|text| !text.is_empty())?;
        let modified = super::modified_label(card)
            .map(|l| l.label().to_string())
            .filter(|s| !s.is_empty());

        let text = match (truncated, modified) {
            (true, Some(date)) => format!("{name}\n{date}"),
            (true, None) => name.to_string(),
            (false, Some(date)) => format!("{name}\n{date}"),
            (false, None) => return None,
        };
        Some(text.into())
    }
}

pub(super) fn install(card: &gtk::Box) {
    let tooltip = NameTooltip::default();
    crate::ui::entry_tooltip::EntryTooltip::install(card, move |card, x, y| {
        tooltip
            .text(card, x, y, Instant::now())
            .map(|s| s.to_string())
    });
}
