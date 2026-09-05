// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;
use std::time::{Duration, Instant};

#[track_caller]
fn wait_until(condition: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !condition() {
        assert!(Instant::now() < deadline, "browser did not settle");
        glib::MainContext::default().iteration(false);
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn press_column_background(view: &BrowserView, depth: usize) {
    let surface = view.state.columns.borrow()[depth]
        .presentation
        .stack
        .clone();
    wait_until(|| surface.height() > 100);
    let controllers = surface.observe_controllers();
    let gesture = (0..controllers.n_items())
        .filter_map(|index| controllers.item(index).and_downcast::<gtk::GestureClick>())
        .find(|gesture| gesture.button() == 1)
        .expect("background focus gesture");
    gesture.emit_by_name::<()>(
        "pressed",
        &[&1i32, &30.0f64, &(f64::from(surface.height()) - 40.0)],
    );
}

#[test]
fn horizontal_scrollbar_stays_below_destination_hints() {
    const CHILD: &str = "STRATA_DESTINATION_SCROLLBAR_GTK_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let sandbox = tempfile::tempdir().expect("isolated preferences");
        let status = std::process::Command::new(std::env::current_exe().expect("test executable"))
            .args([
                "--exact",
                "ui::browser::tests::focus::horizontal_scrollbar_stays_below_destination_hints",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .env("XDG_CONFIG_HOME", sandbox.path().join("config"))
            .env("XDG_CACHE_HOME", sandbox.path().join("cache"))
            .env("XDG_DATA_HOME", sandbox.path().join("data"))
            .status()
            .expect("GTK test starts");
        assert!(status.success());
        return;
    }
    if gtk::init().is_err() {
        return;
    }
    crate::assets::prepare().expect("bundled assets");
    crate::assets::register_icon_theme();
    let fixture = tempfile::tempdir().expect("directory fixture");
    std::fs::create_dir_all(fixture.path().join("Child/Grandchild")).expect("nested folders");
    let view = BrowserView::new(
        Rc::new(crate::adapters::LocalFileSource),
        PeekBehavior::default(),
    );
    let browser = view.browser();
    let window = gtk::Window::builder()
        .child(&view.widget())
        .default_width(640)
        .default_height(500)
        .resizable(false)
        .build();
    window.present();
    browser.navigate(Location::local(fixture.path()));
    let scroller = &view.state.scroller;
    let scrollbar = scroller.hscrollbar();
    let adjustment = scroller.hadjustment();
    wait_until(|| {
        browser.column_snapshot(0).is_some_and(|s| !s.loading) && adjustment.page_size() > 0.0
    });
    assert!(!scroller.is_overlay_scrolling());
    assert!(
        !scrollbar.is_mapped(),
        "no scrollbar is needed for a single fitting pane"
    );
    for depth in 0..2 {
        browser.select(depth, 0);
        browser.enter_focused_directory();
        wait_until(|| {
            browser
                .column_snapshot(depth + 1)
                .is_some_and(|s| !s.loading)
        });
    }
    view.keyboard_navigation();
    browser.focus_active();
    wait_until(|| {
        scrollbar.is_mapped()
            && scrollbar.height() > 0
            && view.state.columns.borrow()[2].destination_hint.height() > 0
    });
    assert_eq!(
        view.state.columns.borrow()[2].destination_hint.text(),
        "Keyboard · Paste here"
    );
    scrollbar.add_css_class("hovering");
    scrollbar.add_css_class("dragging");
    let extent = adjustment.upper() - adjustment.page_size();
    assert!(extent > 0.0);
    for value in [adjustment.lower(), extent / 2.0, extent] {
        adjustment.set_value(value);
        let bar = scrollbar
            .compute_bounds(scroller)
            .expect("scrollbar bounds");
        for column in view.state.columns.borrow().iter() {
            let hint = column
                .destination_hint
                .compute_bounds(scroller)
                .expect("hint bounds");
            assert!(
                hint.y() + hint.height() <= bar.y() + 0.5,
                "the scrollbar must not overlap a destination label"
            );
        }
    }
    browser.close_column(1);
    wait_until(|| !scrollbar.is_mapped());
    window.destroy();
    browser.clear_observer();
}

#[test]
fn pane_ownership_routes_commands_and_preserves_selection() {
    const CHILD: &str = "STRATA_FOCUS_GTK_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let sandbox = tempfile::tempdir().expect("isolated preferences");
        let status = std::process::Command::new(std::env::current_exe().expect("test executable"))
            .args([
                "--exact",
                "ui::browser::tests::focus::pane_ownership_routes_commands_and_preserves_selection",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .env("XDG_CONFIG_HOME", sandbox.path().join("config"))
            .env("XDG_CACHE_HOME", sandbox.path().join("cache"))
            .env("XDG_DATA_HOME", sandbox.path().join("data"))
            .status()
            .expect("isolated GTK test starts");
        assert!(status.success());
        return;
    }
    if gtk::init().is_err() {
        return;
    }
    crate::assets::prepare().expect("bundled assets");
    crate::assets::register_icon_theme();
    let fixture = tempfile::tempdir().expect("fixture directory");
    std::fs::create_dir(fixture.path().join("Child")).expect("child directory");
    std::fs::write(fixture.path().join("alpha.txt"), "alpha").expect("first file");
    std::fs::write(fixture.path().join("bravo.txt"), "bravo").expect("second file");
    let view = BrowserView::new(
        Rc::new(crate::adapters::LocalFileSource),
        PeekBehavior::default(),
    );
    let browser = view.browser();
    view.set_operation_provider(Rc::new(crate::adapters::LocalOperationProvider));
    let window = gtk::Window::builder()
        .child(&view.widget())
        .default_width(1000)
        .default_height(700)
        .build();
    window.present();
    browser.navigate(Location::local(fixture.path()));
    wait_until(|| {
        browser
            .column_snapshot(0)
            .is_some_and(|s| !s.loading && s.count == 3)
    });
    view.keyboard_navigation();
    browser.select(0, 1);
    browser.focus_active();
    assert!(view.copy_selection());
    browser.select(0, 0);
    browser.activate_focused();
    wait_until(|| browser.column_snapshot(1).is_some_and(|s| !s.loading));
    browser.focus_active();
    assert_eq!(view.state.focused_column_depth(), Some(1));
    assert!(view.item_view_has_focus());
    view.state.hovered_column.set(Some(0));
    assert_eq!(view.state.destination_depth(), Some(1));
    assert!(
        !view.copy_selection(),
        "empty child must not copy the parent folder"
    );
    assert!(
        !view.confirm_delete(false),
        "empty child must not delete its parent"
    );
    assert!(
        !view.confirm_delete(true),
        "permanent deletion needs an explicit selection too"
    );
    press_column_background(&view, 0);
    assert_eq!(browser.active_depth(), Some(0));
    press_column_background(&view, 1);
    assert_eq!(browser.active_depth(), Some(1));
    assert!(view.item_view_has_focus());
    view.paste();
    wait_until(|| fixture.path().join("Child/alpha.txt").exists());
    assert_eq!(
        std::fs::read_to_string(fixture.path().join("Child/alpha.txt")).expect("pasted file"),
        "alpha"
    );

    browser.focus_parent();
    view.select_all();
    assert_eq!(browser.selected_positions(0), [0, 1, 2]);
    press_column_background(&view, 1);
    press_column_background(&view, 0);
    assert_eq!(browser.active_depth(), Some(0));
    assert!(browser.column_snapshot(1).is_some());
    assert_eq!(browser.selected_positions(0), [0, 1, 2]);
    assert_eq!(
        view.state.columns.borrow()[0].selection.selection().size(),
        3
    );

    browser.set_active_column(1);
    browser.focus_active();
    view.state.handle(&BrowserEvent::SelectionSetChanged {
        depth: 0,
        positions: vec![0, 1, 2],
        focused: 2,
        take_focus: false,
    });
    assert_eq!(view.state.focused_column_depth(), Some(1));
    assert_eq!(browser.active_depth(), Some(1));

    view.state.input_ownership.borrow_mut().pointer_action();
    view.state.hovered_column.set(Some(0));
    view.state.refresh_destination_style();
    assert_eq!(view.state.destination_depth(), Some(0));
    assert_eq!(
        view.state.columns.borrow()[0].destination_hint.text(),
        "Pointer · Paste here"
    );
    view.keyboard_navigation();
    wait_until(|| {
        browser
            .column_snapshot(1)
            .is_some_and(|snapshot| snapshot.count == 1)
    });
    browser.select(1, 0);
    browser.focus_active();
    wait_until(|| {
        view.state.columns.borrow()[1]
            .bound_rows
            .borrow()
            .iter()
            .any(|bound| {
                bound
                    .row
                    .upgrade()
                    .is_some_and(|row| row.has_css_class("keyboard-cursor"))
            })
    });
    let cursors = view
        .state
        .columns
        .borrow()
        .iter()
        .map(|column| {
            column
                .bound_rows
                .borrow()
                .iter()
                .filter(|bound| {
                    bound
                        .row
                        .upgrade()
                        .is_some_and(|row| row.has_css_class("keyboard-cursor"))
                })
                .count()
        })
        .sum::<usize>();
    assert_eq!(cursors, 1);
    assert_eq!(view.state.destination_depth(), Some(1));
    assert_eq!(
        view.state.columns.borrow()[1].destination_hint.text(),
        "Keyboard · Paste here"
    );

    let surface = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let button = gtk::Button::with_label("Retry");
    let field = gtk::Entry::new();
    let scrollbar = gtk::Scrollbar::new(gtk::Orientation::Vertical, None::<&gtk::Adjustment>);
    surface.append(&button);
    surface.append(&field);
    surface.append(&scrollbar);
    for control in [
        button.upcast::<gtk::Widget>(),
        field.upcast(),
        scrollbar.upcast(),
    ] {
        assert!(!is_column_background(surface.upcast_ref(), &control));
    }

    for mode in [BrowserMode::Grid, BrowserMode::Explorer] {
        view.set_view_mode(mode);
        assert_eq!(view.state.destination_depth(), browser.active_depth());
    }
    window.destroy();
    browser.clear_observer();
}
