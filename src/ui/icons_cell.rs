// SPDX-License-Identifier: GPL-3.0-or-later

use gtk::prelude::*;

pub(super) const MIN_ICONS_THUMBNAIL_SIZE: i32 = 64;
pub(super) const MAX_ICONS_THUMBNAIL_SIZE: i32 = 256;
const FALLBACK_ICONS_COLUMN_WIDTH: i32 = 160;
pub(super) const ICONS_CARD_SPACING: i32 = 4;
const ICONS_CARD_LABEL_CHARS: i32 = 16;
const ICONS_CARD_LABEL_LINES: i32 = 2;
const ICONS_CARD_LABEL_LINE_PX: i32 = 18;
const ICONS_CARD_PAD_Y: i32 = 4;

pub(super) fn new_card(slot: i32) -> gtk::Box {
    let card = gtk::Box::new(gtk::Orientation::Vertical, 3);
    card.add_css_class("icons-card");
    card.set_overflow(gtk::Overflow::Hidden);
    card.set_halign(gtk::Align::Fill);
    card.set_valign(gtk::Align::Start);

    let icon = super::thumbnail::ThumbnailSlot::new(slot);
    icon.add_css_class("icons-card-icon");
    icon.set_halign(gtk::Align::Center);
    icon.set_valign(gtk::Align::Start);

    let label = gtk::Inscription::new(None);
    label.add_css_class("icons-card-label");
    label.add_css_class("alternate-rename-label");
    configure_label(&label);

    let labels = gtk::Overlay::new();
    labels.set_hexpand(true);
    labels.set_child(Some(&label));

    card.append(&icon);
    card.append(&labels);
    set_slot(&card, slot);
    card
}

pub(super) fn parts(
    card: &impl IsA<gtk::Widget>,
) -> Option<(super::thumbnail::ThumbnailSlot, gtk::Inscription)> {
    let icon = card
        .first_child()?
        .downcast::<super::thumbnail::ThumbnailSlot>()
        .ok()?;
    let labels = card.last_child()?.downcast::<gtk::Overlay>().ok()?;
    let label = labels.child()?.downcast::<gtk::Inscription>().ok()?;
    Some((icon, label))
}

pub(super) fn rename_field(card: &impl IsA<gtk::Widget>) -> Option<gtk::Entry> {
    let labels = card.last_child()?.downcast::<gtk::Overlay>().ok()?;
    let mut sibling = labels.first_child();
    while let Some(widget) = sibling {
        sibling = widget.next_sibling();
        if let Ok(entry) = widget.downcast::<gtk::Entry>() {
            return Some(entry);
        }
    }
    None
}

pub(super) fn ensure_rename_field(card: &impl IsA<gtk::Widget>) -> Option<gtk::Entry> {
    if let Some(field) = rename_field(card) {
        return Some(field);
    }
    let labels = card.last_child()?.downcast::<gtk::Overlay>().ok()?;
    let field = gtk::Entry::new();
    field.add_css_class("inline-rename");
    crate::ui::accessibility::set_label(&field, "Rename");
    field.set_width_chars(1);
    field.set_hexpand(true);
    field.set_visible(false);
    labels.add_overlay(&field);
    Some(field)
}

pub(super) fn set_slot(card: &gtk::Box, thumbnail_size: i32) {
    let slot = icons_card_icon_slot(thumbnail_size);
    let (width, height) = icons_card_extent(slot);
    if card.width_request() != width || card.height_request() != height {
        card.set_size_request(width, height);
    }
    if let Some((icon, _)) = parts(card) {
        icon.set_slot(slot);
    }
    if let Some(labels) = card.last_child() {
        labels.set_height_request(ICONS_CARD_LABEL_LINE_PX * ICONS_CARD_LABEL_LINES);
    }
}

pub(super) fn icons_card_icon_slot(thumbnail_size: i32) -> i32 {
    thumbnail_size.clamp(MIN_ICONS_THUMBNAIL_SIZE, MAX_ICONS_THUMBNAIL_SIZE)
}

pub(super) fn icons_card_extent(thumbnail_size: i32) -> (i32, i32) {
    let slot = icons_card_icon_slot(thumbnail_size);
    let width = slot.max(FALLBACK_ICONS_COLUMN_WIDTH - ICONS_CARD_SPACING);
    let height = slot + ICONS_CARD_LABEL_LINE_PX * ICONS_CARD_LABEL_LINES + ICONS_CARD_PAD_Y + 3;
    (width, height)
}

fn configure_label(label: &gtk::Inscription) {
    let chars = ICONS_CARD_LABEL_CHARS as u32;
    let lines = ICONS_CARD_LABEL_LINES as u32;
    label.set_min_chars(chars);
    label.set_nat_chars(chars);
    label.set_min_lines(lines);
    label.set_nat_lines(lines);
    label.set_xalign(0.5);
    label.set_yalign(0.0);
    label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    label.set_text_overflow(gtk::InscriptionOverflow::EllipsizeEnd);
}

#[cfg(test)]
mod tests;
