// SPDX-License-Identifier: MIT

use gtk::prelude::*;

pub(super) fn text(card: &gtk::Box, x: f64, y: f64) -> Option<String> {
    let (_, label) = super::parts(card)?;
    if !label.is_visible() || !crate::ui::pointer::hits_icon_card_content(card.upcast_ref(), x, y) {
        return None;
    }
    let name = label.text().filter(|text| !text.is_empty())?;
    let modified = super::modified_label(card).map(|label| label.text());
    crate::ui::entry_tooltip::name_text(
        &name,
        super::layout::caption_truncated(&label),
        modified.as_deref(),
    )
}

pub(super) fn install(card: &gtk::Box) {
    crate::ui::entry_tooltip::EntryTooltip::install(card, text);
}
