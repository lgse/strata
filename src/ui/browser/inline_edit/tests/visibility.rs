// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;

fn emit_scroll(view: &BrowserView) {
    let root = view.widget();
    view.install_inline_edit_dismissal(&root);
    let controllers = root.observe_controllers();
    let scroll = (0..controllers.n_items())
        .filter_map(|index| controllers.item(index))
        .find_map(|controller| controller.downcast::<gtk::EventControllerScroll>().ok())
        .expect("scroll dismissal controller");
    assert_eq!(scroll.propagation_phase(), gtk::PropagationPhase::Capture);
    assert!(!scroll.emit_by_name::<bool>("scroll", &[&0.0_f64, &1.0_f64]));
}

#[test]
fn scroll_before_creation_refresh_revokes_only_edit_authority() {
    gtk_test(
        "ui::browser::inline_edit::tests::visibility::scroll_before_creation_refresh_revokes_only_edit_authority",
        || {
            for mode in [BrowserMode::Columns, BrowserMode::List] {
                let fixture = tempfile::tempdir().expect("fixture");
                std::fs::write(fixture.path().join("anchor"), b"body").expect("anchor");
                let view = BrowserView::new(
                    Rc::new(crate::adapters::LocalFileSource),
                    PeekBehavior::default(),
                );
                view.set_view_mode(mode);
                view.set_operation_provider(Rc::new(crate::adapters::LocalOperationProvider));
                let window = gtk::Window::builder().child(&view.widget()).build();
                window.present();
                let browser = view.browser();
                let parent = Location::local(fixture.path());
                browser.navigate(parent.clone());
                wait_until(|| {
                    browser
                        .column_snapshot(0)
                        .is_some_and(|s| !s.loading && s.count == 1)
                });
                browser.select(0, 0);
                settle_frames();
                view.state.begin_new_entry(0, parent, false);
                assert!(view.state.pending_new_entry.borrow().is_some());
                emit_scroll(&view);
                wait_until(|| {
                    browser
                        .column_snapshot(0)
                        .is_some_and(|s| !s.loading && s.count == 2)
                });
                wait_until(|| view.state.pending_new_entry.borrow().is_none());
                settle_frames();
                assert!(fixture.path().join("new file").exists());
                assert!(!view.rename_is_active(), "{mode:?}");
                assert_eq!(
                    browser.selected_entries()[0].display_name,
                    "anchor",
                    "{mode:?}"
                );
                browser.clear_observer();
                window.destroy();
            }
        },
    );
}

fn settle_frames() {
    let deadline = Instant::now() + Duration::from_millis(150);
    while Instant::now() < deadline {
        while glib::MainContext::default().pending() {
            glib::MainContext::default().iteration(false);
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn accepting_an_unchanged_name_preserves_the_visible_viewport() {
    gtk_test(
        "ui::browser::inline_edit::tests::visibility::accepting_an_unchanged_name_preserves_the_visible_viewport",
        || {
            for mode in [BrowserMode::Columns, BrowserMode::List] {
                let fixture = tempfile::tempdir().expect("fixture");
                for index in 0..100 {
                    std::fs::write(fixture.path().join(format!("item-{index:03}.txt")), b"")
                        .expect("fixture file");
                }
                let view = BrowserView::new(
                    Rc::new(crate::adapters::LocalFileSource),
                    PeekBehavior::default(),
                );
                view.set_view_mode(mode);
                let window = gtk::Window::builder()
                    .child(&view.widget())
                    .default_width(800)
                    .default_height(400)
                    .build();
                window.present();
                let browser = view.browser();
                browser.navigate(Location::local(fixture.path()));
                wait_until(|| {
                    browser
                        .column_snapshot(0)
                        .is_some_and(|s| !s.loading && s.count == 100)
                });
                let list = if mode == BrowserMode::Columns {
                    view.state.columns.borrow()[0].list.clone()
                } else {
                    view.state
                        .mode_views
                        .borrow()
                        .list_rename_view(0)
                        .expect("List view")
                };
                let scroll = list
                    .ancestor(gtk::ScrolledWindow::static_type())
                    .and_downcast::<gtk::ScrolledWindow>()
                    .expect("scroller");
                settle_frames();
                let adjustment = scroll.vadjustment();
                assert!(adjustment.upper() > adjustment.page_size());
                browser.select(0, 2);
                wait_until(|| view.state.begin_rename());
                let field = view
                    .state
                    .active_rename
                    .borrow()
                    .as_ref()
                    .map(|rename| rename.field.clone())
                    .or_else(|| view.state.mode_views.borrow().active_rename_field())
                    .expect("editor");
                settle_frames();
                let before = adjustment.value();
                let values = Rc::new(RefCell::new(Vec::new()));
                let observed = values.clone();
                let listener = adjustment.connect_value_changed(move |adjustment| {
                    observed.borrow_mut().push(adjustment.value());
                });
                field.emit_activate();
                wait_until(|| !view.rename_is_active());
                settle_frames();
                adjustment.disconnect(listener);
                assert_eq!(adjustment.value(), before, "{mode:?}");
                assert!(
                    values.borrow().iter().all(|value| *value == before),
                    "{mode:?}: {:?}",
                    values.borrow()
                );
                assert!(!view.state.rename_operation_pending());
                assert_eq!(browser.selected_entries()[0].display_name, "item-002.txt");
                assert!(fixture.path().join("item-002.txt").exists());
                browser.clear_observer();
                window.destroy();
            }
        },
    );
}

#[test]
fn active_editor_yields_reveal_after_deliberate_scroll() {
    gtk_test(
        "ui::browser::inline_edit::tests::visibility::active_editor_yields_reveal_after_deliberate_scroll",
        || {
            for mode in [BrowserMode::Columns, BrowserMode::List] {
                let fixture = tempfile::tempdir().expect("fixture");
                for index in 0..100 {
                    std::fs::write(fixture.path().join(format!("item-{index:03}.txt")), b"")
                        .expect("fixture file");
                }
                let view = BrowserView::new(
                    Rc::new(crate::adapters::LocalFileSource),
                    PeekBehavior::default(),
                );
                view.set_view_mode(mode);
                let window = gtk::Window::builder()
                    .child(&view.widget())
                    .default_width(800)
                    .default_height(400)
                    .build();
                window.present();
                let browser = view.browser();
                browser.navigate(Location::local(fixture.path()));
                wait_until(|| {
                    browser
                        .column_snapshot(0)
                        .is_some_and(|s| !s.loading && s.count == 100)
                });
                let list = if mode == BrowserMode::Columns {
                    view.state.columns.borrow()[0].list.clone()
                } else {
                    view.state
                        .mode_views
                        .borrow()
                        .list_rename_view(0)
                        .expect("List view")
                };
                let scroll = list
                    .ancestor(gtk::ScrolledWindow::static_type())
                    .and_downcast::<gtk::ScrolledWindow>()
                    .expect("scroller");
                settle_frames();
                browser.select(0, 2);
                wait_until(|| view.state.begin_rename());
                settle_frames();
                emit_scroll(&view);
                let adjustment = scroll.vadjustment();
                adjustment.set_value(80.0);
                let requested = adjustment.value();
                assert!(requested > 0.0);
                settle_frames();
                assert!(view.rename_is_active(), "{mode:?}");
                assert_eq!(adjustment.value(), requested, "{mode:?}");
                browser.clear_observer();
                window.destroy();
            }
        },
    );
}

#[test]
fn reveal_rename_row_leaves_visible_rows_and_horizontal_scroll_unchanged() {
    gtk_test(
        "ui::browser::inline_edit::tests::visibility::reveal_rename_row_leaves_visible_rows_and_horizontal_scroll_unchanged",
        || {
            let content = gtk::Fixed::new();
            content.set_size_request(1200, 1600);
            let row = gtk::Label::new(Some("rename target"));
            row.set_size_request(200, 30);
            content.put(&row, 300.0, 650.0);
            let scroll = gtk::ScrolledWindow::builder().child(&content).build();
            let window = gtk::Window::builder()
                .child(&scroll)
                .default_width(400)
                .default_height(300)
                .build();
            window.present();
            settle_frames();
            scroll.vadjustment().set_value(600.0);
            scroll.hadjustment().set_value(200.0);
            settle_frames();
            let vertical = scroll.vadjustment().value();
            let horizontal = scroll.hadjustment().value();
            for _ in 0..3 {
                assert!(reveal_rename_row(&row, &scroll, None).is_some());
                assert_eq!(scroll.vadjustment().value(), vertical);
                assert_eq!(scroll.hadjustment().value(), horizontal);
                settle_frames();
            }
            for requested in [700.0, 300.0] {
                scroll.vadjustment().set_value(requested);
                settle_frames();
                let bounds = row.compute_bounds(&scroll).expect("row bounds");
                let bottom = scroll.height() as f32;
                let delta = if bounds.y() < 0.0 {
                    bounds.y()
                } else {
                    (bounds.y() + bounds.height() - bottom).max(0.0)
                };
                assert_ne!(delta, 0.0, "fixture row must be clipped");
                let before = scroll.vadjustment().value();
                assert!(reveal_rename_row(&row, &scroll, None).is_none());
                assert_eq!(scroll.vadjustment().value(), before + f64::from(delta));
                assert_eq!(scroll.hadjustment().value(), horizontal);
            }
            window.destroy();
        },
    );
}
