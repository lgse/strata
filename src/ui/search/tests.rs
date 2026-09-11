// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn only_rows_intersecting_the_viewport_are_visible() {
    assert!(intersects_viewport(100.0, 32.0, 90.0, 100.0));
    assert!(!intersects_viewport(58.0, 32.0, 90.0, 100.0));
    assert!(!intersects_viewport(190.0, 32.0, 90.0, 100.0));
}

#[test]
fn result_updates_preserve_entry_caret_selection_and_default_selection() {
    crate::test_support::gtk_test(
        "ui::search::tests::result_updates_preserve_entry_caret_selection_and_default_selection",
        || {
            let (dialog, window) = mapped_dialog(Rc::new(|_| {}));
            dialog.state.field.set_text("quarterly");
            assert!(dialog.state.field.grab_focus_without_selecting());
            dialog.state.field.set_position(7);
            dialog.state.field.select_region(2, 7);
            let caret = dialog.state.field.position();
            let selection = dialog.state.field.selection_bounds();

            let initial = search_items("result", 3);
            render_results(
                &dialog.state,
                initial.clone(),
                true,
                SearchCoverage::default(),
            );
            assert_eq!(
                dialog
                    .state
                    .list
                    .selected_row()
                    .expect("default selected row")
                    .index(),
                0
            );
            assert!(contains_keyboard_focus(dialog.state.field.upcast_ref()));
            assert_eq!(dialog.state.field.position(), caret);
            assert_eq!(dialog.state.field.selection_bounds(), selection);

            render_results(&dialog.state, initial, false, SearchCoverage::default());
            drain_main_context();
            assert!(contains_keyboard_focus(dialog.state.field.upcast_ref()));
            assert_eq!(dialog.state.field.position(), caret);
            assert_eq!(dialog.state.field.selection_bounds(), selection);
            window.destroy();
        },
    );
}

#[test]
fn query_changes_retain_rows_until_incremental_results_arrive() {
    crate::test_support::gtk_test(
        "ui::search::tests::query_changes_retain_rows_until_incremental_results_arrive",
        || {
            let (dialog, window) = mapped_dialog(Rc::new(|_| {}));
            dialog.state.field.set_text("le");
            render_results(
                &dialog.state,
                search_items("le-cat", 3),
                true,
                SearchCoverage::default(),
            );
            let retained = dialog
                .state
                .list
                .row_at_index(0)
                .expect("initial result row");
            let text = dialog
                .state
                .field
                .first_child()
                .and_downcast::<gtk::Text>()
                .expect("entry text");

            text.emit_by_name::<()>("insert-at-cursor", &[&"-"]);

            assert_eq!(dialog.state.list.row_at_index(0), Some(retained));
            window.destroy();
        },
    );
}

#[test]
fn refined_queries_select_the_new_best_result_without_rebuilding_it() {
    crate::test_support::gtk_test(
        "ui::search::tests::refined_queries_select_the_new_best_result_without_rebuilding_it",
        || {
            let (dialog, window) = mapped_dialog(Rc::new(|_| {}));
            dialog.state.field.set_text("l");
            let other = SearchItem::for_test(PathBuf::from("/search/older-log.txt"), false);
            let target =
                SearchItem::for_test(PathBuf::from("/Pictures/test/dsds/le-cat.jpeg"), false);
            render_results(
                &dialog.state,
                vec![other.clone(), target.clone()],
                false,
                SearchCoverage::default(),
            );
            let target_row = dialog
                .state
                .list
                .row_at_index(1)
                .expect("target result row");
            let text = dialog
                .state
                .field
                .first_child()
                .and_downcast::<gtk::Text>()
                .expect("entry text");
            text.emit_by_name::<()>("insert-at-cursor", &[&"e-cat"]);

            render_results(
                &dialog.state,
                vec![target, other],
                false,
                SearchCoverage::default(),
            );

            assert_eq!(dialog.state.list.row_at_index(0), Some(target_row.clone()));
            assert_eq!(dialog.state.list.selected_row(), Some(target_row));
            assert!(contains_keyboard_focus(dialog.state.field.upcast_ref()));
            window.destroy();
        },
    );
}

#[test]
fn arrow_keys_keep_entry_focus_and_use_a_logical_navigation_start() {
    crate::test_support::gtk_test(
        "ui::search::tests::arrow_keys_keep_entry_focus_and_use_a_logical_navigation_start",
        || {
            let (dialog, window) = mapped_dialog(Rc::new(|_| {}));
            dialog.state.field.set_text("navigation");
            render_results(
                &dialog.state,
                search_items("navigation", 3),
                false,
                SearchCoverage::default(),
            );
            assert!(dialog.state.field.grab_focus_without_selecting());
            dialog.state.field.set_position(6);
            dialog.state.field.select_region(2, 6);
            let selection = dialog.state.field.selection_bounds();

            assert!(emit_key(
                &dialog,
                gtk::gdk::Key::Down,
                gtk::gdk::ModifierType::empty()
            ));
            assert_selected(&dialog, 0);
            assert!(emit_key(
                &dialog,
                gtk::gdk::Key::Down,
                gtk::gdk::ModifierType::empty()
            ));
            assert_selected(&dialog, 1);
            assert!(emit_key(
                &dialog,
                gtk::gdk::Key::Down,
                gtk::gdk::ModifierType::empty()
            ));
            assert_selected(&dialog, 2);
            assert!(emit_key(
                &dialog,
                gtk::gdk::Key::Down,
                gtk::gdk::ModifierType::empty()
            ));
            assert_selected(&dialog, 2);
            assert!(emit_key(
                &dialog,
                gtk::gdk::Key::Up,
                gtk::gdk::ModifierType::empty()
            ));
            assert_selected(&dialog, 1);
            assert!(emit_key(
                &dialog,
                gtk::gdk::Key::Up,
                gtk::gdk::ModifierType::empty()
            ));
            assert!(emit_key(
                &dialog,
                gtk::gdk::Key::Up,
                gtk::gdk::ModifierType::empty()
            ));
            assert_selected(&dialog, 0);

            assert!(contains_keyboard_focus(dialog.state.field.upcast_ref()));
            assert_eq!(dialog.state.field.position(), 6);
            assert_eq!(dialog.state.field.selection_bounds(), selection);
            assert!(!emit_key(
                &dialog,
                gtk::gdk::Key::Down,
                gtk::gdk::ModifierType::SHIFT_MASK,
            ));
            assert_selected(&dialog, 0);
            window.destroy();
        },
    );
}

#[test]
fn typing_and_backspace_after_arrows_edit_the_query_at_the_caret() {
    crate::test_support::gtk_test(
        "ui::search::tests::typing_and_backspace_after_arrows_edit_the_query_at_the_caret",
        || {
            let (dialog, window) = mapped_dialog(Rc::new(|_| {}));
            dialog.state.field.set_text("quartely");
            let items = search_items("typing", 3);
            render_results(
                &dialog.state,
                items.clone(),
                false,
                SearchCoverage::default(),
            );
            assert!(dialog.state.field.grab_focus_without_selecting());
            dialog.state.field.set_position(6);
            assert!(emit_key(
                &dialog,
                gtk::gdk::Key::Down,
                gtk::gdk::ModifierType::empty()
            ));
            assert!(emit_key(
                &dialog,
                gtk::gdk::Key::Down,
                gtk::gdk::ModifierType::empty()
            ));

            let text = dialog
                .state
                .field
                .first_child()
                .and_downcast::<gtk::Text>()
                .expect("entry text");
            text.emit_by_name::<()>("insert-at-cursor", &[&"r"]);
            assert_eq!(dialog.state.field.text(), "quarterly");
            assert_eq!(dialog.state.field.position(), 7);
            assert_eq!(*dialog.state.visible_results.borrow(), items);
            assert!(!dialog.state.navigation_started.get());
            text.emit_by_name::<()>("backspace", &[]);
            assert_eq!(dialog.state.field.text(), "quartely");
            assert_eq!(dialog.state.field.position(), 6);
            assert_eq!(*dialog.state.visible_results.borrow(), items);
            assert!(contains_keyboard_focus(dialog.state.field.upcast_ref()));
            window.destroy();
        },
    );
}

#[test]
fn arrow_navigation_reveals_offscreen_results_without_moving_focus() {
    crate::test_support::gtk_test(
        "ui::search::tests::arrow_navigation_reveals_offscreen_results_without_moving_focus",
        || {
            let (dialog, window) = mapped_dialog(Rc::new(|_| {}));
            render_results(
                &dialog.state,
                search_items("offscreen", 40),
                false,
                SearchCoverage::default(),
            );
            drain_main_context();
            assert!(dialog.state.field.grab_focus_without_selecting());
            for _ in 0..26 {
                assert!(emit_key(
                    &dialog,
                    gtk::gdk::Key::Down,
                    gtk::gdk::ModifierType::empty(),
                ));
            }
            assert_selected(&dialog, 25);
            let adjustment = dialog.state.scroller.vadjustment();
            assert!(adjustment.value() > 0.0);
            let selected = dialog.state.list.selected_row().expect("selected row");
            let bounds = selected
                .compute_bounds(&dialog.state.list)
                .expect("allocated selected row");
            assert!(f64::from(bounds.y()) >= adjustment.value());
            assert!(
                f64::from(bounds.y() + bounds.height())
                    <= adjustment.value() + adjustment.page_size()
            );
            assert!(contains_keyboard_focus(dialog.state.field.upcast_ref()));
            window.destroy();
        },
    );
}

#[test]
fn result_reorders_reveal_keyboard_selection_after_layout_and_user_scroll_wins() {
    crate::test_support::gtk_test(
        "ui::search::tests::result_reorders_reveal_keyboard_selection_after_layout_and_user_scroll_wins",
        || {
            let (dialog, window) = mapped_dialog(Rc::new(|_| {}));
            let mut items = search_items("reorder", 100);
            dialog
                .state
                .requested_thumbnails
                .borrow_mut()
                .extend(items.iter().map(|item| item.path.clone()));
            render_results(
                &dialog.state,
                items.clone(),
                true,
                SearchCoverage::default(),
            );
            settle_layout();
            assert!(dialog.state.field.grab_focus_without_selecting());
            dialog.state.reconciling_results.set(true);
            for _ in 0..21 {
                assert!(emit_key(
                    &dialog,
                    gtk::gdk::Key::Down,
                    gtk::gdk::ModifierType::empty(),
                ));
            }
            dialog.state.reconciling_results.set(false);
            let selected_path = items[20].path.clone();

            let selected = items.remove(20);
            items.insert(0, selected);
            dialog
                .state
                .requested_thumbnails
                .borrow_mut()
                .extend(items.iter().map(|item| item.path.clone()));
            render_results(
                &dialog.state,
                items.clone(),
                true,
                SearchCoverage::default(),
            );
            settle_layout();
            assert_eq!(
                dialog.state.list.selected_row().expect("selection").index(),
                0
            );
            assert_selected_is_visible(&dialog);
            assert!(contains_keyboard_focus(dialog.state.field.upcast_ref()));

            let selected = items.remove(0);
            items.insert(90, selected);
            dialog
                .state
                .requested_thumbnails
                .borrow_mut()
                .extend(items.iter().map(|item| item.path.clone()));
            render_results(
                &dialog.state,
                items.clone(),
                true,
                SearchCoverage::default(),
            );
            settle_layout();
            let selected_row = dialog.state.list.selected_row().expect("selection");
            assert_eq!(selected_row.index(), 90);
            assert_eq!(items[90].path, selected_path);
            assert_selected_is_visible(&dialog);
            assert!(contains_keyboard_focus(dialog.state.field.upcast_ref()));

            let selected = items.remove(90);
            items.insert(0, selected);
            dialog
                .state
                .requested_thumbnails
                .borrow_mut()
                .extend(items.iter().map(|item| item.path.clone()));
            render_results(&dialog.state, items, false, SearchCoverage::default());
            assert!(
                !scroll_controller(&dialog).emit_by_name::<bool>("scroll", &[&0.0_f64, &1.0_f64],)
            );
            let adjustment = dialog.state.scroller.vadjustment();
            adjustment.set_value(200.0);
            settle_layout();
            assert_eq!(adjustment.value(), 200.0);
            assert!(contains_keyboard_focus(dialog.state.field.upcast_ref()));
            window.destroy();
        },
    );
}

#[test]
fn partial_updates_preserve_selected_path_entry_focus_and_navigation_progress() {
    crate::test_support::gtk_test(
        "ui::search::tests::partial_updates_preserve_selected_path_entry_focus_and_navigation_progress",
        || {
            let (dialog, window) = mapped_dialog(Rc::new(|_| {}));
            dialog.state.field.set_text("stable");
            let mut items = search_items("stable", 5);
            render_results(
                &dialog.state,
                items.clone(),
                true,
                SearchCoverage::default(),
            );
            assert!(dialog.state.field.grab_focus_without_selecting());
            dialog.state.field.set_position(3);
            assert!(emit_key(
                &dialog,
                gtk::gdk::Key::Down,
                gtk::gdk::ModifierType::empty()
            ));
            assert!(emit_key(
                &dialog,
                gtk::gdk::Key::Down,
                gtk::gdk::ModifierType::empty()
            ));
            let selected_path = items[1].path.clone();

            items.insert(
                0,
                SearchItem::for_test(PathBuf::from("/search/inserted.txt"), false),
            );
            items.swap(2, 4);
            render_results(
                &dialog.state,
                items.clone(),
                true,
                SearchCoverage::default(),
            );
            let selected = dialog
                .state
                .list
                .selected_row()
                .expect("restored selection");
            assert_eq!(items[selected.index() as usize].path, selected_path);
            assert!(contains_keyboard_focus(dialog.state.field.upcast_ref()));
            assert_eq!(dialog.state.field.position(), 3);

            assert!(emit_key(
                &dialog,
                gtk::gdk::Key::Down,
                gtk::gdk::ModifierType::empty()
            ));
            let advanced = dialog
                .state
                .list
                .selected_row()
                .expect("advanced selection");
            assert_eq!(advanced.index(), selected.index() + 1);
            let removed_index = advanced.index() as usize;
            items.remove(removed_index);
            let fallback_index = removed_index.min(items.len() - 1);
            render_results(&dialog.state, items, false, SearchCoverage::default());
            assert_eq!(
                dialog
                    .state
                    .list
                    .selected_row()
                    .expect("stable fallback")
                    .index(),
                fallback_index as i32
            );
            assert!(contains_keyboard_focus(dialog.state.field.upcast_ref()));
            assert_eq!(dialog.state.field.position(), 3);
            window.destroy();
        },
    );
}

#[test]
fn arrows_with_empty_results_preserve_the_entry_caret() {
    crate::test_support::gtk_test(
        "ui::search::tests::arrows_with_empty_results_preserve_the_entry_caret",
        || {
            let (dialog, window) = mapped_dialog(Rc::new(|_| {}));
            dialog.state.field.set_text("missing");
            dialog.state.field.set_position(3);
            dialog.state.field.grab_focus_without_selecting();

            assert!(emit_key(
                &dialog,
                gtk::gdk::Key::Down,
                gtk::gdk::ModifierType::empty()
            ));
            assert!(dialog.state.list.selected_row().is_none());
            assert!(contains_keyboard_focus(dialog.state.field.upcast_ref()));
            assert_eq!(dialog.state.field.position(), 3);
            assert!(!dialog.state.navigation_started.get());
            window.destroy();
        },
    );
}

#[test]
fn enter_focuses_results_before_opening_and_ctrl_k_resumes_editing() {
    crate::test_support::gtk_test(
        "ui::search::tests::enter_focuses_results_before_opening_and_ctrl_k_resumes_editing",
        || {
            let activated = Rc::new(RefCell::new(None));
            let observed = activated.clone();
            let (dialog, window) = mapped_dialog(Rc::new(move |item| {
                observed.replace(Some(item));
            }));
            dialog.state.field.set_text("activation");
            dialog.state.field.set_position(4);
            let items = search_items("activation", 3);
            render_results(
                &dialog.state,
                items.clone(),
                false,
                SearchCoverage::default(),
            );
            assert!(dialog.state.field.grab_focus_without_selecting());

            assert!(key_controller(&dialog).emit_by_name::<bool>(
                "key-pressed",
                &[
                    &gtk::gdk::Key::Return,
                    &0u32,
                    &gtk::gdk::ModifierType::empty(),
                ],
            ));
            assert!(activated.borrow().is_none());
            assert!(contains_keyboard_focus(dialog.state.list.upcast_ref()));
            assert!(emit_key(&dialog, gdk::Key::j, gdk::ModifierType::empty()));
            assert_eq!(
                dialog
                    .state
                    .list
                    .selected_row()
                    .expect("next result")
                    .index(),
                1
            );
            assert!(emit_key(&dialog, gdk::Key::k, gdk::ModifierType::empty()));
            assert_eq!(
                dialog
                    .state
                    .list
                    .selected_row()
                    .expect("previous result")
                    .index(),
                0
            );
            assert!(emit_key(
                &dialog,
                gdk::Key::k,
                gdk::ModifierType::CONTROL_MASK
            ));
            assert!(contains_keyboard_focus(dialog.state.field.upcast_ref()));
            assert_eq!(dialog.state.field.text(), "activation");
            assert_eq!(dialog.state.field.position(), 4);
            for key in [gdk::Key::h, gdk::Key::j, gdk::Key::k, gdk::Key::l] {
                assert!(!emit_key(&dialog, key, gdk::ModifierType::empty()));
            }
            assert!(emit_key(
                &dialog,
                gdk::Key::KP_Enter,
                gdk::ModifierType::empty()
            ));
            assert!(activated.borrow().is_none());
            assert!(emit_key(&dialog, gdk::Key::j, gdk::ModifierType::empty()));
            assert!(emit_key(
                &dialog,
                gdk::Key::Return,
                gdk::ModifierType::empty()
            ));
            assert_eq!(
                activated.borrow().as_ref().expect("activated item").path,
                items[1].path
            );
            window.destroy();
        },
    );
}

#[test]
fn enter_before_results_arrive_preserves_navigation_through_updates() {
    crate::test_support::gtk_test(
        "ui::search::tests::enter_before_results_arrive_preserves_navigation_through_updates",
        || {
            let (dialog, window) = mapped_dialog(Rc::new(|_| panic!("must not open")));
            dialog.state.field.set_text("pending");
            assert!(emit_key(
                &dialog,
                gdk::Key::Return,
                gdk::ModifierType::empty()
            ));
            assert!(dialog.state.layer.has_focus());
            let mut items = search_items("pending", 4);
            render_results(
                &dialog.state,
                items.clone(),
                true,
                SearchCoverage::default(),
            );
            assert!(contains_keyboard_focus(dialog.state.list.upcast_ref()));
            assert!(emit_key(&dialog, gdk::Key::j, gdk::ModifierType::empty()));
            let selected = dialog.state.list.selected_row().expect("selected result");
            let path = items[1].path.clone();
            items.swap(1, 3);
            render_results(
                &dialog.state,
                items.clone(),
                false,
                SearchCoverage::default(),
            );
            assert_eq!(dialog.state.list.selected_row(), Some(selected.clone()));
            assert!(contains_keyboard_focus(selected.upcast_ref()));
            assert_eq!(items[selected.index() as usize].path, path);
            assert_eq!(dialog.state.field.text(), "pending");
            window.destroy();
        },
    );
}

#[test]
fn unchanged_results_retain_exact_row_descendant_focus_and_scroll() {
    crate::test_support::gtk_test(
        "ui::search::tests::unchanged_results_retain_exact_row_descendant_focus_and_scroll",
        || {
            let (dialog, window) = mapped_dialog(Rc::new(|_| {}));
            let items = search_items("unchanged", 40);
            render_results(
                &dialog.state,
                items.clone(),
                true,
                SearchCoverage::default(),
            );
            drain_main_context();
            let row = dialog
                .state
                .list
                .row_at_index(12)
                .expect("selected result row");
            dialog.state.list.select_row(Some(&row));
            let descendant = row.child().expect("result row content");
            descendant.set_focusable(true);
            assert!(descendant.grab_focus());
            let adjustment = dialog.state.scroller.vadjustment();
            adjustment.set_value(180.0);
            let scroll = adjustment.value();

            render_results(&dialog.state, items, false, SearchCoverage::default());
            drain_main_context();
            assert_eq!(dialog.state.list.selected_row(), Some(row));
            assert_eq!(gtk::prelude::GtkWindowExt::focus(&window), Some(descendant));
            assert_eq!(adjustment.value(), scroll);
            window.destroy();
        },
    );
}

#[test]
fn removed_focused_result_focuses_the_fallback_then_waits_when_empty() {
    crate::test_support::gtk_test(
        "ui::search::tests::removed_focused_result_focuses_the_fallback_then_waits_when_empty",
        || {
            let (dialog, window) = mapped_dialog(Rc::new(|_| {}));
            dialog.state.field.set_text("remaining");
            dialog.state.field.set_position(4);
            let mut items = search_items("removed", 4);
            render_results(
                &dialog.state,
                items.clone(),
                true,
                SearchCoverage::default(),
            );
            drain_main_context();
            let removed = dialog
                .state
                .list
                .row_at_index(1)
                .expect("focused result row");
            dialog.state.list.select_row(Some(&removed));
            assert!(removed.grab_focus());

            items.remove(1);
            render_results(
                &dialog.state,
                items.clone(),
                false,
                SearchCoverage::default(),
            );
            let fallback = dialog
                .state
                .list
                .row_at_index(1)
                .expect("fallback result row");
            assert_eq!(dialog.state.list.selected_row(), Some(fallback.clone()));
            assert!(contains_keyboard_focus(fallback.upcast_ref()));

            render_results(&dialog.state, Vec::new(), false, SearchCoverage::default());
            assert!(dialog.state.list.selected_row().is_none());
            assert!(dialog.state.layer.has_focus());
            assert_eq!(dialog.state.field.position(), 4);
            window.destroy();
        },
    );
}

#[test]
fn reconciliation_retains_rows_and_invalidates_changed_thumbnail_requests() {
    crate::test_support::gtk_test(
        "ui::search::tests::reconciliation_retains_rows_and_invalidates_changed_thumbnail_requests",
        || {
            let (dialog, window) = mapped_dialog(Rc::new(|_| {}));
            let initial = search_items("result", 40);
            render_results(
                &dialog.state,
                initial.clone(),
                true,
                SearchCoverage::default(),
            );
            drain_main_context();
            let selected = dialog
                .state
                .list
                .row_at_index(1)
                .expect("selected result row");
            dialog.state.list.select_row(Some(&selected));
            let descendant = selected.child().expect("result row content");
            descendant.set_focusable(true);
            assert!(descendant.grab_focus());
            let thumbnail = descendant.first_child().expect("result thumbnail");
            let selected_path = initial[1].path.clone();

            let mut reordered = initial;
            reordered.swap(0, 1);
            render_results(
                &dialog.state,
                reordered.clone(),
                true,
                SearchCoverage::default(),
            );
            let moved = dialog
                .state
                .list
                .selected_row()
                .expect("reordered selected row");
            assert_eq!(reordered[moved.index() as usize].path, selected_path);
            assert_eq!(moved, selected);
            assert_eq!(
                moved
                    .child()
                    .expect("moved row content")
                    .first_child()
                    .expect("moved row thumbnail"),
                thumbnail
            );
            assert_eq!(gtk::prelude::GtkWindowExt::focus(&window), Some(descendant));

            let changed_path = reordered[0].path.clone();
            dialog
                .state
                .requested_thumbnails
                .borrow_mut()
                .insert(changed_path.clone());
            let old_row = dialog
                .state
                .list
                .row_at_index(0)
                .expect("original changed row");
            reordered[0].is_directory = true;
            render_results(&dialog.state, reordered, false, SearchCoverage::default());
            assert_ne!(
                dialog
                    .state
                    .list
                    .row_at_index(0)
                    .expect("replacement changed row"),
                old_row
            );
            assert!(
                !dialog
                    .state
                    .requested_thumbnails
                    .borrow()
                    .contains(&changed_path)
            );
            window.destroy();
        },
    );
}

#[test]
fn deferred_scroll_restoration_yields_to_updates_wheel_scrollbar_and_query_reset() {
    crate::test_support::gtk_test(
        "ui::search::tests::deferred_scroll_restoration_yields_to_updates_wheel_scrollbar_and_query_reset",
        || {
            let (dialog, window) = mapped_dialog(Rc::new(|_| {}));
            let fixture = tempfile::tempdir().expect("scroll fixture");
            let mut items = Vec::new();
            for position in 0..80 {
                let path = fixture.path().join(format!("scroll-{position:03}"));
                std::fs::create_dir(&path).expect("scroll fixture directory");
                items.push(SearchItem::for_test(path, true));
            }
            render_results(
                &dialog.state,
                items.clone(),
                true,
                SearchCoverage::default(),
            );
            drain_main_context();
            let adjustment = dialog.state.scroller.vadjustment();
            assert!(adjustment.upper() > adjustment.page_size() + 300.0);

            adjustment.set_value(120.0);
            items.swap(0, 1);
            render_results(
                &dialog.state,
                items.clone(),
                true,
                SearchCoverage::default(),
            );
            adjustment.set_value(180.0);
            items.swap(1, 2);
            render_results(
                &dialog.state,
                items.clone(),
                true,
                SearchCoverage::default(),
            );
            drain_main_context();
            assert_eq!(adjustment.value(), 180.0);

            items.swap(2, 3);
            render_results(
                &dialog.state,
                items.clone(),
                true,
                SearchCoverage::default(),
            );
            assert!(
                !scroll_controller(&dialog).emit_by_name::<bool>("scroll", &[&0.0_f64, &1.0_f64],)
            );
            adjustment.set_value(240.0);
            drain_main_context();
            assert_eq!(adjustment.value(), 240.0);
            assert!(!dialog.state.requested_thumbnails.borrow().is_empty());

            items.swap(3, 4);
            render_results(
                &dialog.state,
                items.clone(),
                true,
                SearchCoverage::default(),
            );
            let scroller_bounds = dialog
                .state
                .scroller
                .compute_bounds(&dialog.state.layer)
                .expect("mapped scroller bounds");
            let scrollbar_x = f64::from(scroller_bounds.x() + scroller_bounds.width()) - 1.0;
            let scrollbar_y = f64::from(scroller_bounds.y() + scroller_bounds.height() / 2.0);
            click_controller(&dialog)
                .emit_by_name::<()>("pressed", &[&1i32, &scrollbar_x, &scrollbar_y]);
            adjustment.set_value(280.0);
            drain_main_context();
            assert_eq!(adjustment.value(), 280.0);
            assert!(dialog.state.layer.is_visible());

            items.swap(4, 5);
            render_results(
                &dialog.state,
                items.clone(),
                true,
                SearchCoverage::default(),
            );
            dialog.state.field.set_text("different");
            adjustment.set_value(300.0);
            drain_main_context();
            assert_eq!(adjustment.value(), 300.0);
            assert_eq!(*dialog.state.visible_results.borrow(), items);
            assert!(!dialog.state.navigation_started.get());

            items.swap(5, 6);
            render_results(&dialog.state, items, true, SearchCoverage::default());
            dialog.state.field.set_text("");
            drain_main_context();
            assert_eq!(adjustment.value(), 0.0);
            assert!(dialog.state.visible_results.borrow().is_empty());
            window.destroy();
        },
    );
}

fn mapped_dialog(activate: Rc<dyn Fn(SearchItem)>) -> (SearchDialog, gtk::Window) {
    let dialog = SearchDialog::new(activate, Rc::new(|| {}));
    let window = gtk::Window::builder()
        .default_width(900)
        .default_height(600)
        .child(&dialog.widget())
        .build();
    dialog.state.layer.set_visible(true);
    window.present();
    drain_main_context();
    (dialog, window)
}

fn key_controller(dialog: &SearchDialog) -> gtk::EventControllerKey {
    controller::<gtk::EventControllerKey>(&dialog.state.layer)
}

fn emit_key(dialog: &SearchDialog, key: gtk::gdk::Key, modifiers: gtk::gdk::ModifierType) -> bool {
    key_controller(dialog).emit_by_name::<bool>("key-pressed", &[&key, &0u32, &modifiers])
}

fn assert_selected(dialog: &SearchDialog, position: i32) {
    assert_eq!(
        dialog
            .state
            .list
            .selected_row()
            .expect("selected result")
            .index(),
        position
    );
}

fn assert_selected_is_visible(dialog: &SearchDialog) {
    let adjustment = dialog.state.scroller.vadjustment();
    let selected = dialog.state.list.selected_row().expect("selected result");
    let bounds = selected
        .compute_bounds(&dialog.state.list)
        .expect("allocated selected row");
    assert!(f64::from(bounds.y()) >= adjustment.value());
    assert!(f64::from(bounds.y() + bounds.height()) <= adjustment.value() + adjustment.page_size());
}

fn scroll_controller(dialog: &SearchDialog) -> gtk::EventControllerScroll {
    controller::<gtk::EventControllerScroll>(&dialog.state.layer)
}

fn click_controller(dialog: &SearchDialog) -> gtk::GestureClick {
    controller::<gtk::GestureClick>(&dialog.state.layer)
}

fn controller<
    T: glib::object::IsA<gtk::EventController>
        + glib::object::IsA<glib::Object>
        + glib::object::ObjectType,
>(
    widget: &impl IsA<gtk::Widget>,
) -> T {
    let controllers = widget.observe_controllers();
    (0..controllers.n_items())
        .filter_map(|index| controllers.item(index))
        .find_map(|controller| controller.downcast::<T>().ok())
        .expect("expected input controller")
}

fn search_items(prefix: &str, count: usize) -> Vec<SearchItem> {
    (0..count)
        .map(|position| {
            SearchItem::for_test(
                PathBuf::from(format!("/search/{prefix}-{position:03}.txt")),
                false,
            )
        })
        .collect()
}

fn drain_main_context() {
    while glib::MainContext::default().iteration(false) {}
}

fn settle_layout() {
    for _ in 0..20 {
        glib::MainContext::default().iteration(false);
    }
}

#[test]
fn global_search_combines_home_and_drives_and_refreshes_mounts() {
    crate::test_support::gtk_test(
        "ui::search::tests::global_search_combines_home_and_drives_and_refreshes_mounts",
        || {
            let fixture = tempfile::tempdir().expect("search fixture");
            let home = fixture.path().join("Home");
            let usb = fixture.path().join("USB");
            for root in [&home, &usb] {
                std::fs::create_dir(root).expect("search root");
                std::fs::write(root.join("needle.txt"), b"fixture").expect("search file");
            }
            let activated = Rc::new(RefCell::new(None));
            let observed = activated.clone();
            let dialog = SearchDialog::new(
                Rc::new(move |item| {
                    observed.replace(Some(item));
                }),
                Rc::new(|| {}),
            );
            let window = gtk::Window::builder().child(&dialog.widget()).build();
            window.present();
            dialog.show(vec![home.clone(), usb.clone()], false);
            assert_eq!(
                dialog
                    .state
                    .field
                    .parent()
                    .expect("search bar")
                    .next_sibling(),
                Some(dialog.state.results.clone().upcast())
            );
            assert_eq!(
                dialog.state.status.text(),
                "Type to search Home and mounted local drives"
            );
            let tooltip = dialog.state.field.tooltip_text().expect("scope locations");
            assert!(tooltip.contains("Remote shares are not included."));
            assert!(tooltip.contains(&home.display().to_string()));
            assert!(tooltip.contains(&usb.display().to_string()));
            dialog.state.field.set_text("needle");
            wait_until(|| dialog.state.visible_results.borrow().len() == 2);
            for (position, item) in dialog.state.visible_results.borrow().iter().enumerate() {
                let row = dialog
                    .state
                    .list
                    .row_at_index(position as i32)
                    .expect("result row");
                assert_eq!(
                    row.tooltip_text().as_deref(),
                    Some(item.path.to_string_lossy().as_ref())
                );
            }

            dialog.show(vec![home.clone()], false);
            assert!(dialog.state.visible_results.borrow().is_empty());
            let tooltip = dialog
                .state
                .field
                .tooltip_text()
                .expect("updated scope locations");
            assert!(tooltip.contains(&home.display().to_string()));
            assert!(!tooltip.contains(&usb.display().to_string()));
            dialog.state.field.set_text("needle");
            wait_until(|| {
                !dialog.state.indexing_spinner.is_visible()
                    && dialog.state.visible_results.borrow().len() == 1
            });
            assert_eq!(
                dialog.state.visible_results.borrow()[0].path,
                home.join("needle.txt")
            );

            let coverage = SearchCoverage {
                unreadable: true,
                time_limit: true,
                ..Default::default()
            };
            render_results(&dialog.state, Vec::new(), false, coverage);
            assert!(dialog.state.truncated_hint.is_visible());
            assert_eq!(dialog.state.truncated_hint.text(), coverage.message());

            dialog.show(vec![home, usb.clone()], false);
            dialog.state.field.set_text("needle");
            wait_until(|| dialog.state.visible_results.borrow().len() == 2);
            let usb_position = dialog
                .state
                .visible_results
                .borrow()
                .iter()
                .position(|item| item.path.starts_with(&usb))
                .expect("USB result");
            activate_position(&dialog.state, usb_position as i32);
            assert_eq!(
                activated.borrow().as_ref().expect("activation").path,
                usb.join("needle.txt")
            );
            assert!(dialog.state.search.borrow().is_none());
            window.destroy();
        },
    );
}

fn wait_until(condition: impl Fn() -> bool) {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !condition() {
        assert!(
            std::time::Instant::now() < deadline,
            "search results timed out"
        );
        glib::MainContext::default().iteration(false);
        std::thread::sleep(Duration::from_millis(1));
    }
}
