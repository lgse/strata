// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn resizing_refreshes_fallbacks_without_discarding_decoded_thumbnails() {
    crate::test_support::gtk_test(
        "ui::thumbnail::slot::tests::resizing_refreshes_fallbacks_without_discarding_decoded_thumbnails",
        || {
            let slot = ThumbnailSlot::new(64);
            slot.set_icon_context(crate::assets::IconContext::Grid);
            crate::ui::thumbnail::show_fallback_icon(&slot, crate::assets::icons::FOLDER, 64);
            let small = slot
                .imp()
                .fallback
                .borrow()
                .clone()
                .expect("small fallback");
            slot.set_slot(128);
            let large = slot
                .imp()
                .fallback
                .borrow()
                .clone()
                .expect("large fallback");
            assert_ne!(small, large, "resizing must rerender fallback artwork");
            slot.set_slot(64);
            assert_eq!(slot.imp().fallback.borrow().as_ref(), Some(&small));
            slot.set_texture(&large);
            slot.set_slot(128);
            assert_eq!(slot.texture(), Some(large));
        },
    );
}
