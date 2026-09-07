// SPDX-License-Identifier: GPL-3.0-or-later

use super::{ensure_rename_field, icons_card_extent, new_card, parts, rename_field, set_slot};
use crate::test_support::gtk_test;
use gtk::prelude::*;

#[test]
fn card_keeps_a_fixed_size_request() {
    gtk_test(
        "ui::icons_cell::tests::card_keeps_a_fixed_size_request",
        || {
            let card = new_card(64);
            let (width, height) = icons_card_extent(64);
            assert_eq!(card.width_request(), width);
            assert_eq!(card.height_request(), height);
            let (icon, _) = parts(&card).expect("icon and label");
            icon.set_slot(512);
            set_slot(&card, 64);
            assert_eq!(card.width_request(), width);
            assert_eq!(card.height_request(), height);
        },
    );
}

#[test]
fn new_card_has_no_rename_entry_until_needed() {
    gtk_test(
        "ui::icons_cell::tests::new_card_has_no_rename_entry_until_needed",
        || {
            let card = new_card(64);
            let (width, height) = icons_card_extent(64);
            assert!(rename_field(&card).is_none());
            let field = ensure_rename_field(&card).expect("rename field");
            assert!(field.has_css_class("inline-rename"));
            assert!(!gtk::prelude::WidgetExt::is_visible(&field));
            assert_eq!(card.width_request(), width);
            assert_eq!(card.height_request(), height);
            assert!(rename_field(&card).is_some());
        },
    );
}
