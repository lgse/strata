// SPDX-License-Identifier: MIT
use super::*;

#[test]
fn pointer_selection_preserves_drag_groups_and_applies_ranges_and_toggles() {
    crate::test_support::gtk_test(
        "ui::collection_interaction::tests::pointer_selection_preserves_drag_groups_and_applies_ranges_and_toggles",
        || {
            let plain = gdk::ModifierType::empty();
            let control = gdk::ModifierType::CONTROL_MASK;
            let shift = gdk::ModifierType::SHIFT_MASK;
            let first = pointer_selection(&gtk::Bitset::new_empty(), 2, None, true, plain);
            let both = pointer_selection(&first.selected, 4, first.anchor, true, control);
            assert!(both.selected.contains(2) && both.selected.contains(4));
            let drag = pointer_selection(&both.selected, 2, both.anchor, true, plain);
            assert!(drag.preserved_group);
            assert!(drag.selected.equals(&both.selected));
            let range = pointer_selection(&drag.selected, 5, drag.anchor, true, shift);
            assert!(range.selected.equals(&gtk::Bitset::new_range(2, 4)));
            let toggle = pointer_selection(&range.selected, 3, range.anchor, true, control);
            assert!(!toggle.selected.contains(3));
            assert!(toggle.selected.contains(2) && toggle.selected.contains(5));
            for modifiers in [plain, control, shift, control | shift] {
                let single =
                    pointer_selection(&toggle.selected, 4, toggle.anchor, false, modifiers);
                assert!(single.selected.equals(&gtk::Bitset::new_range(4, 1)));
                assert!(!single.preserved_group);
            }
        },
    );
}

#[test]
fn modified_sequence_defers_claim_and_cancellation_retires_activation() {
    crate::test_support::gtk_test(
        "ui::collection_interaction::tests::modified_sequence_defers_claim_and_cancellation_retires_activation",
        || {
            let gesture = gtk::GestureClick::new();
            let sequence = PointerSequence::default();
            sequence.install(&gesture);
            sequence.press(gdk::ModifierType::CONTROL_MASK);
            assert_eq!(sequence.activation(), Some(false));
            gesture.emit_by_name::<()>("cancel", &[&None::<gdk::EventSequence>]);
            assert_eq!(sequence.activation(), None);
            sequence.press(gdk::ModifierType::empty());
            gesture.emit_by_name::<()>("released", &[&1i32, &0f64, &0f64]);
            assert_eq!(sequence.activation(), Some(true));
            sequence.press(gdk::ModifierType::SHIFT_MASK);
            while glib::MainContext::default().iteration(false) {}
            assert_eq!(
                sequence.activation(),
                Some(false),
                "old release must not retire the new gesture"
            );
        },
    );
}
