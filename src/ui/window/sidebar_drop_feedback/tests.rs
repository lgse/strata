// SPDX-License-Identifier: MIT

use std::{cell::Cell, rc::Rc};

use super::*;
use crate::test_support::gtk_test;

#[test]
fn late_validation_and_leave_cannot_restore_or_clear_another_hover() {
    gtk_test(
        "ui::window::sidebar_drop_feedback::tests::late_validation_and_leave_cannot_restore_or_clear_another_hover",
        || {
            let feedback = SidebarDropFeedback::default();
            let old = gtk::Box::new(gtk::Orientation::Vertical, 0);
            let current = gtk::Box::new(gtk::Orientation::Vertical, 0);
            feedback.enter(old.upcast_ref());
            feedback.pin(old.upcast_ref());
            feedback.enter(current.upcast_ref());
            feedback.pin(current.upcast_ref());

            feedback.pin(old.upcast_ref());
            feedback.leave(old.upcast_ref());
            assert!(!old.has_css_class("eligible"));
            assert!(current.has_css_class("eligible"));
            assert!(feedback.is_current(current.upcast_ref()));

            feedback.reset();
            feedback.pin(current.upcast_ref());
            assert!(!current.has_css_class("eligible"));
        },
    );
}

#[test]
fn folder_and_pin_feedback_are_exclusive_and_switching_cancels_spring_loading() {
    gtk_test(
        "ui::window::sidebar_drop_feedback::tests::folder_and_pin_feedback_are_exclusive_and_switching_cancels_spring_loading",
        || {
            let sidebar = gtk::Box::new(gtk::Orientation::Vertical, 0);
            let folder = gtk::Button::new();
            let gap = gtk::Box::new(gtk::Orientation::Vertical, 0);
            let next = gtk::Button::new();
            let feedback = SidebarDropFeedback::new(&sidebar);
            let cancelled = Rc::new(Cell::new(0));
            let observed = cancelled.clone();
            feedback.enter_folder(folder.upcast_ref(), move || {
                observed.set(observed.get() + 1)
            });
            feedback.folder(folder.upcast_ref(), true);
            assert!(folder.has_css_class("drop-destination"));

            feedback.enter(gap.upcast_ref());
            feedback.pin(gap.upcast_ref());
            assert_eq!(cancelled.get(), 1);
            assert!(!folder.has_css_class("drop-destination"));
            feedback.folder(folder.upcast_ref(), true);
            feedback.leave(folder.upcast_ref());
            assert!(!folder.has_css_class("drop-destination"));
            assert!(gap.has_css_class("eligible"));
            assert!(sidebar.has_css_class("sidebar-drag-active"));
            assert!(sidebar.has_css_class("sidebar-inserting"));

            feedback.enter_folder(next.upcast_ref(), || {});
            feedback.folder(next.upcast_ref(), true);
            feedback.pin(gap.upcast_ref());
            assert!(!gap.has_css_class("eligible"));
            assert!(next.has_css_class("drop-destination"));
            assert!(!sidebar.has_css_class("sidebar-inserting"));
            feedback.reset();
            assert!(!next.has_css_class("drop-destination"));
            assert!(!sidebar.has_css_class("sidebar-drag-active"));
        },
    );
}

#[test]
fn crossing_a_pin_boundary_transfers_ownership_without_stale_cleanup() {
    gtk_test(
        "ui::window::sidebar_drop_feedback::tests::crossing_a_pin_boundary_transfers_ownership_without_stale_cleanup",
        || {
            let sidebar = gtk::Box::new(gtk::Orientation::Vertical, 0);
            let above = gtk::Button::new();
            let below = gtk::Button::new();
            for row in [&above, &below] {
                row.add_css_class("sidebar-pinned-row");
            }
            let gap = row_gap(None);
            gap.add_css_class("sidebar-pin-gap");
            sidebar.append(&above);
            sidebar.append(&gap);
            sidebar.append(&below);
            let feedback = SidebarDropFeedback::default();

            feedback.enter(above.upcast_ref());
            feedback.reorder(above.upcast_ref(), true);
            assert!(gap.has_css_class("eligible"));
            feedback.enter(gap.upcast_ref());
            feedback.pin(gap.upcast_ref());
            feedback.leave(above.upcast_ref());
            assert!(gap.has_css_class("eligible"));
            assert!(!above.has_css_class("reorder-after"));

            feedback.enter(below.upcast_ref());
            feedback.reorder(below.upcast_ref(), false);
            feedback.leave(gap.upcast_ref());
            assert!(gap.has_css_class("eligible"));
            feedback.leave(below.upcast_ref());
            assert!(!gap.has_css_class("eligible"));
        },
    );
}
