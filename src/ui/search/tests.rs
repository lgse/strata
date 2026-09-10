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
fn entry_down_up_down_cycles_through_the_first_result() {
    crate::test_support::gtk_test(
        "ui::search::tests::entry_down_up_down_cycles_through_the_first_result",
        || {
            let (dialog, window) = mapped_dialog(Rc::new(|_| {}));
            render_results(
                &dialog.state,
                search_items("navigation", 3),
                false,
                SearchCoverage::default(),
            );
            assert!(dialog.state.field.grab_focus_without_selecting());

            move_selection(&dialog.state, 1);
            let first = dialog.state.list.row_at_index(0).expect("first result row");
            assert!(contains_keyboard_focus(first.upcast_ref()));
            move_selection(&dialog.state, -1);
            assert!(contains_keyboard_focus(dialog.state.field.upcast_ref()));
            assert_eq!(dialog.state.list.selected_row(), Some(first.clone()));
            move_selection(&dialog.state, 1);
            assert!(contains_keyboard_focus(first.upcast_ref()));
            assert_eq!(dialog.state.list.selected_row(), Some(first));
            window.destroy();
        },
    );
}

#[test]
fn enter_activates_the_default_result_directly_from_the_entry() {
    crate::test_support::gtk_test(
        "ui::search::tests::enter_activates_the_default_result_directly_from_the_entry",
        || {
            let activated = Rc::new(RefCell::new(None));
            let observed = activated.clone();
            let (dialog, window) = mapped_dialog(Rc::new(move |item| {
                observed.replace(Some(item));
            }));
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
            assert_eq!(
                activated.borrow().as_ref().expect("activated item").path,
                items[0].path
            );
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
fn removed_focused_result_focuses_the_fallback_then_the_entry_when_empty() {
    crate::test_support::gtk_test(
        "ui::search::tests::removed_focused_result_focuses_the_fallback_then_the_entry_when_empty",
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
            assert!(contains_keyboard_focus(dialog.state.field.upcast_ref()));
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
            render_results(&dialog.state, items, true, SearchCoverage::default());
            begin_query(&dialog.state, "different");
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
