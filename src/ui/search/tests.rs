// SPDX-License-Identifier: MIT

use super::*;

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
fn arrow_keys_advance_from_preselection_and_keep_entry_focus() {
    crate::test_support::gtk_test(
        "ui::search::tests::arrow_keys_advance_from_preselection_and_keep_entry_focus",
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
            assert_selected(&dialog, 0);
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
    let dialog = SearchDialog::new(activate, Rc::new(|_| {}), Rc::new(|| {}));
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
    settle_layout();
}

fn settle_layout() {
    let Some(window) = gtk::Window::list_toplevels()
        .into_iter()
        .find(|window| window.is_mapped())
    else {
        while glib::MainContext::default().iteration(false) {}
        return;
    };
    // Ready GLib sources alone are not a barrier for GTK's frame-clock layout.
    let frames = Rc::new(Cell::new(0));
    let observed = frames.clone();
    window.add_tick_callback(move |_, _| {
        observed.set(observed.get() + 1);
        if observed.get() >= 3 {
            glib::ControlFlow::Break
        } else {
            glib::ControlFlow::Continue
        }
    });
    wait_until(|| frames.get() >= 3);
    wait_until(|| !glib::MainContext::default().iteration(false));
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
                Rc::new(|_| {}),
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
            assert_eq!(
                dialog.state.truncated_hint.tooltip_text().as_deref(),
                Some(coverage.message().as_str())
            );
            render_results(&dialog.state, Vec::new(), false, SearchCoverage::default());
            assert!(!dialog.state.truncated_hint.is_visible());

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
