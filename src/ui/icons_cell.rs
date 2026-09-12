// SPDX-License-Identifier: MIT

use gtk::prelude::*;

mod layout;

pub(super) const MIN_ICONS_THUMBNAIL_SIZE: i32 = 32;
pub(super) const MAX_ICONS_THUMBNAIL_SIZE: i32 = 256;
const FALLBACK_ICONS_COLUMN_WIDTH: i32 = 120;
pub(super) const ICONS_CARD_SPACING: i32 = 4;
const ICONS_CARD_LABEL_CHARS: i32 = 12;
const ICONS_CARD_LABEL_LINES: i32 = 2;
const ICONS_CARD_LABEL_LINE_PX: i32 = 18;
const ICONS_CARD_PAD_Y: i32 = 4;
pub(super) const ICONS_CARD_ICON_PADDING: i32 = 6;
const ICONS_CARD_CAPTION_GAP: i32 = 4;

pub(super) fn new_card(slot: i32) -> gtk::Box {
    let card = gtk::Box::new(gtk::Orientation::Vertical, 0);
    card.add_css_class("icons-card");
    card.set_overflow(gtk::Overflow::Hidden);
    card.set_halign(gtk::Align::Fill);
    card.set_valign(gtk::Align::Start);

    let icon = super::thumbnail::ThumbnailSlot::new(slot);
    icon.set_content_inset(0);
    icon.set_margin_top(ICONS_CARD_ICON_PADDING);
    icon.set_margin_bottom(ICONS_CARD_ICON_PADDING);
    icon.set_margin_start(ICONS_CARD_ICON_PADDING);
    icon.set_margin_end(ICONS_CARD_ICON_PADDING);
    icon.limit_fallback_height_to_folder();
    icon.add_css_class("icons-card-icon");
    icon.set_halign(gtk::Align::Center);
    icon.set_valign(gtk::Align::Center);
    let icon_frame = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    icon_frame.add_css_class("icons-card-icon-frame");
    icon_frame.append(&icon);

    let label = gtk::Inscription::new(None);
    label.add_css_class("icons-card-label");
    label.add_css_class("alternate-rename-label");
    configure_label(&label);

    // GtkOverlay requires its own layout-child type; this caption has a custom layout.
    let labels = gtk::Box::new(gtk::Orientation::Vertical, 0);
    labels.add_css_class("icons-card-caption");
    labels.set_hexpand(true);
    labels.append(&label);

    card.append(&icon_frame);
    card.append(&labels);
    layout::install(&card, &label);
    set_slot(&card, slot);
    card
}

pub(super) fn parts(
    card: &impl IsA<gtk::Widget>,
) -> Option<(super::thumbnail::ThumbnailSlot, gtk::Inscription)> {
    let icon = card
        .first_child()?
        .first_child()?
        .downcast::<super::thumbnail::ThumbnailSlot>()
        .ok()?;
    let labels = card.last_child()?.downcast::<gtk::Box>().ok()?;
    let label = labels.first_child()?.downcast::<gtk::Inscription>().ok()?;
    Some((icon, label))
}

pub(super) fn rename_field(card: &impl IsA<gtk::Widget>) -> Option<gtk::Entry> {
    let labels = card.last_child()?.downcast::<gtk::Box>().ok()?;
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
    let labels = card.last_child()?.downcast::<gtk::Box>().ok()?;
    let field = gtk::Entry::new();
    field.add_css_class("inline-rename");
    crate::ui::accessibility::set_label(&field, "Rename");
    field.set_width_chars(1);
    field.set_hexpand(true);
    field.set_visible(false);
    labels.append(&field);
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
}

pub(super) fn icons_card_icon_slot(thumbnail_size: i32) -> i32 {
    thumbnail_size.clamp(MIN_ICONS_THUMBNAIL_SIZE, MAX_ICONS_THUMBNAIL_SIZE)
}

pub(super) fn icons_card_extent(thumbnail_size: i32) -> (i32, i32) {
    let slot = icons_card_icon_slot(thumbnail_size);
    let icon_extent = slot + ICONS_CARD_ICON_PADDING * 2;
    let width = icon_extent.max(FALLBACK_ICONS_COLUMN_WIDTH - ICONS_CARD_SPACING);
    let height = icon_extent
        + ICONS_CARD_CAPTION_GAP
        + ICONS_CARD_LABEL_LINE_PX * ICONS_CARD_LABEL_LINES
        + ICONS_CARD_PAD_Y;
    (width, height)
}

fn configure_label(label: &gtk::Inscription) {
    let lines = ICONS_CARD_LABEL_LINES as u32;
    label.set_min_chars(0);
    label.set_nat_chars(0);
    label.set_min_lines(1);
    label.set_nat_lines(lines);
    label.set_xalign(0.5);
    label.set_yalign(0.0);
    label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    label.set_text_overflow(gtk::InscriptionOverflow::EllipsizeEnd);
}

#[cfg(test)]
mod tests;
