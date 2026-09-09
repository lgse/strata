// SPDX-License-Identifier: MIT

use super::*;
use crate::{app::BrowserEvent, ui::browser::columns::is_column_background};
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

fn assert_column_header_actions(view: &BrowserView, active_depth: usize) {
    for (depth, column) in view.state.columns.borrow().iter().enumerate() {
        assert_eq!(
            column.header_actions.is_child_visible(),
            depth == active_depth
        );
    }
}

#[test]
fn filter_follows_latest_pointer_or_keyboard_target() {
    crate::test_support::gtk_test(
        "ui::browser::tests::focus::filter_follows_latest_pointer_or_keyboard_target",
        || {
            let fixture = tempfile::tempdir().expect("directory fixture");
            std::fs::create_dir(fixture.path().join("Child")).expect("child folder");
            let view = BrowserView::new(
                Rc::new(crate::adapters::LocalFileSource),
                PeekBehavior::default(),
            );
            let browser = view.browser();
            let window = gtk::Window::builder()
                .child(&view.widget())
                .default_width(900)
                .default_height(500)
                .build();
            window.present();
            browser.navigate(Location::local(fixture.path()));
            wait_until(|| browser.column_snapshot(0).is_some_and(|s| !s.loading));
            browser.select(0, 0);
            view.state.hovered_column.set(Some(0));
            view.state.pointer_navigation();
            browser.enter_focused_directory();
            assert_eq!(view.state.hovered_column.get(), Some(0));
            assert_column_header_actions(&view, 0);
            wait_until(|| browser.column_snapshot(1).is_some_and(|s| !s.loading));
            browser.focus_active();
            assert_eq!(view.state.focused_column_depth(), Some(1));
            assert_eq!(view.state.hovered_column.get(), Some(0));
            assert_column_header_actions(&view, 0);

            assert!(view.show_filter());
            assert_eq!(view.state.focused_column_depth(), Some(0));
            assert!(view.filter_has_focus());
            assert!(view.state.columns.borrow()[0].filter_button.is_active());
            assert!(!view.state.columns.borrow()[1].filter_button.is_active());
            assert_column_header_actions(&view, 0);
            assert!(view.dismiss_focused_filter());

            browser.set_active_column(1);
            browser.focus_active();
            view.keyboard_navigation();
            assert_eq!(view.state.hovered_column.get(), Some(0));
            assert!(view.show_filter());
            assert_eq!(view.state.focused_column_depth(), Some(1));
            assert!(view.filter_has_focus());
            assert!(!view.state.columns.borrow()[0].filter_button.is_active());
            assert!(view.state.columns.borrow()[1].filter_button.is_active());
            assert_column_header_actions(&view, 1);
            window.close();
        },
    );
}

#[test]
fn hovering_another_column_preserves_keyboard_navigation_from_header() {
    crate::test_support::gtk_test(
        "ui::browser::tests::focus::hovering_another_column_preserves_keyboard_navigation_from_header",
        || {
            let fixture = tempfile::tempdir().expect("directory fixture");
            std::fs::create_dir(fixture.path().join("Child")).expect("child folder");
            std::fs::write(fixture.path().join("Child/item.txt"), "item").expect("file");
            let view = BrowserView::new(
                Rc::new(crate::adapters::LocalFileSource),
                PeekBehavior::default(),
            );
            let browser = view.browser();
            let window = gtk::Window::builder()
                .child(&view.widget())
                .default_width(900)
                .default_height(500)
                .build();
            window.present();
            browser.navigate(Location::local(fixture.path()));
            wait_until(|| browser.column_snapshot(0).is_some_and(|s| !s.loading));
            browser.select(0, 0);
            browser.enter_focused_directory();
            wait_until(|| browser.column_snapshot(1).is_some_and(|s| !s.loading));
            browser.select(1, 0);
            browser.focus_active();
            view.keyboard_navigation();
            assert!(view.focus_header_from_top_item());
            assert!(view.header_actions_have_focus());
            assert!(view.move_header_focus(gtk::DirectionType::Left));
            assert_eq!(view.state.focused_column_depth(), Some(0));
            assert_column_header_actions(&view, 0);
            for _ in 0..12 {
                if view.state.focused_column_depth() == Some(1) {
                    break;
                }
                assert!(view.move_header_focus(gtk::DirectionType::Right));
            }
            assert_eq!(view.state.focused_column_depth(), Some(1));
            assert_column_header_actions(&view, 1);

            view.state.hovered_column.set(Some(0));
            view.state.pointer_navigation();
            assert_column_header_actions(&view, 0);
            assert!(
                view.item_view_has_focus(),
                "hiding focused actions must return focus to the file list"
            );
            assert!(!view.filter_has_focus());
            view.keyboard_navigation();
            assert_column_header_actions(&view, 1);
            assert!(view.item_view_has_focus());
            window.close();
        },
    );
}

#[test]
fn context_menu_keeps_its_column_target_through_focus_and_hover_changes() {
    crate::test_support::gtk_test(
        "ui::browser::tests::focus::context_menu_keeps_its_column_target_through_focus_and_hover_changes",
        || {
            for chooser in [false, true] {
                let fixture = tempfile::tempdir().expect("directory fixture");
                std::fs::create_dir_all(fixture.path().join("Child/Grandchild"))
                    .expect("nested folders");
                let view = if chooser {
                    BrowserView::new_chooser(Rc::new(crate::adapters::LocalFileSource), true)
                } else {
                    BrowserView::new(
                        Rc::new(crate::adapters::LocalFileSource),
                        PeekBehavior::default(),
                    )
                };
                let browser = view.browser();
                let content = gtk::Box::new(gtk::Orientation::Horizontal, 0);
                let home = gtk::Button::with_label("Home");
                content.append(&home);
                content.append(&view.widget());
                let overlay = gtk::Overlay::builder().child(&content).build();
                let window = gtk::Window::builder()
                    .child(&overlay)
                    .default_width(1200)
                    .default_height(500)
                    .build();
                window.present();
                browser.navigate(Location::local(fixture.path()));
                wait_until(|| browser.column_snapshot(0).is_some_and(|s| !s.loading));
                for depth in 0..2 {
                    browser.select(depth, 0);
                    browser.enter_focused_directory();
                    wait_until(|| {
                        browser
                            .column_snapshot(depth + 1)
                            .is_some_and(|s| !s.loading)
                    });
                }
                let column = view.state.columns.borrow()[1].clone();
                wait_until(|| column.list.width() > 0 && !column.bound_rows.borrow().is_empty());
                for previous in [0, 2] {
                    for item in [false, true] {
                        browser.set_active_column(previous);
                        browser.focus_active();
                        view.state.hovered_column.set(Some(1));
                        view.state.pointer_navigation();
                        let (surface, x, y): (gtk::Widget, f64, f64) = if item {
                            let bounds = column
                                .bound_rows
                                .borrow()
                                .iter()
                                .find_map(|bound| bound.row.upgrade()?.compute_bounds(&column.list))
                                .expect("mapped row");
                            (
                                column.list.clone().upcast(),
                                f64::from(bounds.x() + 10.0),
                                f64::from(bounds.center().y()),
                            )
                        } else {
                            (
                                column.presentation.stack.clone().upcast(),
                                20.0,
                                f64::from(column.presentation.stack.height() - 30),
                            )
                        };
                        let controllers = surface.observe_controllers();
                        let gesture = (0..controllers.n_items())
                            .filter_map(|index| {
                                controllers.item(index).and_downcast::<gtk::GestureClick>()
                            })
                            .find(|gesture| gesture.button() == 3)
                            .expect("context gesture");
                        gesture.emit_by_name::<()>("pressed", &[&1i32, &x, &y]);
                        let popover = {
                            let mut child = overlay.first_child();
                            loop {
                                let widget = child.unwrap_or_else(|| panic!("open context menu: chooser={chooser}, previous={previous}, item={item}"));
                                child = widget.next_sibling();
                                if let Ok(popover) = widget.downcast::<gtk::Popover>() {
                                    break popover;
                                }
                            }
                        };
                        wait_until(|| popover.is_mapped());
                        for hovered in [None, Some(previous)] {
                            view.state.hovered_column.set(hovered);
                            view.state.refresh_destination_style();
                            assert_eq!(view.state.destination_depth(), Some(1));
                            assert_eq!(browser.active_depth(), Some(1));
                            assert_column_header_actions(&view, 1);
                        }
                        home.grab_focus();
                        assert!(home.has_focus());
                        popover.popdown();
                        assert!(
                            view.item_view_has_focus(),
                            "restore focus before the next key event"
                        );
                        assert_eq!(view.state.focused_column_depth(), Some(1));
                        wait_until(|| {
                            popover.parent().is_none()
                                && view.state.context_menu_column.get().is_none()
                        });
                        wait_until(|| view.item_view_has_focus());
                        view.keyboard_navigation();
                        assert_eq!(view.state.destination_depth(), Some(1));
                        assert!(view.item_view_has_focus());
                        view.state.hovered_column.set(Some(previous));
                        view.state.pointer_navigation();
                        assert_eq!(view.state.destination_depth(), Some(previous));
                    }
                }
                window.close();
            }
        },
    );
}

fn press_column_background(view: &BrowserView, depth: usize) {
    let surface = view.state.columns.borrow()[depth]
        .presentation
        .stack
        .clone();
    wait_until(|| surface.height() > 100);
    let x = 30.0;
    let y = f64::from(surface.height()) - 40.0;
    let gesture = background_gesture(surface.upcast_ref());
    gesture.emit_by_name::<()>("pressed", &[&1i32, &x, &y]);
    gesture.emit_by_name::<()>("released", &[&1i32, &x, &y]);
}

fn background_gesture(surface: &gtk::Widget) -> gtk::GestureClick {
    let controllers = surface.observe_controllers();
    let click = (0..controllers.n_items())
        .filter_map(|index| controllers.item(index).and_downcast::<gtk::GestureClick>())
        .find(|gesture| gesture.button() == 1)
        .expect("background focus gesture");
    assert!(
        (0..controllers.n_items())
            .filter_map(|index| controllers.item(index).and_downcast::<gtk::GestureDrag>())
            .any(|drag| drag.is_grouped_with(&click)),
        "background focus must share the surface's marquee gesture"
    );
    click
}

#[test]
fn background_and_header_clicks_focus_and_reveal_without_changing_selection() {
    crate::test_support::gtk_test(
        "ui::browser::tests::focus::background_and_header_clicks_focus_and_reveal_without_changing_selection",
        || {
            let fixture = tempfile::tempdir().expect("directory fixture");
            std::fs::create_dir_all(fixture.path().join("Child/Grandchild"))
                .expect("nested folders");
            let view = BrowserView::new(
                Rc::new(crate::adapters::LocalFileSource),
                PeekBehavior::default(),
            );
            let browser = view.browser();
            let window = gtk::Window::builder()
                .child(&view.widget())
                .default_width(640)
                .default_height(500)
                .build();
            window.present();
            browser.navigate(Location::local(fixture.path()));
            wait_until(|| browser.column_snapshot(0).is_some_and(|s| !s.loading));
            for depth in 0..2 {
                browser.select(depth, 0);
                browser.enter_focused_directory();
                wait_until(|| {
                    browser
                        .column_snapshot(depth + 1)
                        .is_some_and(|s| !s.loading)
                });
            }
            assert_column_header_actions(&view, 2);
            let widths = || {
                view.state
                    .columns
                    .borrow()
                    .iter()
                    .map(|column| column.shell.measure(gtk::Orientation::Horizontal, -1))
                    .collect::<Vec<_>>()
            };
            let initial_widths = widths();
            for hovered in [Some(0), Some(1), None] {
                view.state.hovered_column.set(hovered);
                view.state.pointer_navigation();
                assert_column_header_actions(&view, hovered.unwrap_or(2));
                assert_eq!(widths(), initial_widths);
                assert_eq!(browser.active_depth(), Some(2));
                view.keyboard_navigation();
                assert_column_header_actions(&view, 2);
                view.state.pointer_navigation();
                assert_column_header_actions(&view, hovered.unwrap_or(2));
            }
            view.state.rebuild_columns();
            assert_column_header_actions(&view, 2);
            let adjustment = view.state.scroller.hadjustment();
            wait_until(|| adjustment.value() > 100.0);
            for header in [false, true] {
                for depth in [0, 2, 1] {
                    if header {
                        let surface = view.state.columns.borrow()[depth]
                            .header_actions_stack
                            .parent()
                            .expect("column header");
                        let gesture = background_gesture(&surface);
                        let x = 20.0f64;
                        let y = f64::from(surface.height()) / 2.0;
                        gesture.emit_by_name::<()>("pressed", &[&1i32, &x, &y]);
                        gesture.emit_by_name::<()>("released", &[&1i32, &x, &y]);
                    } else {
                        press_column_background(&view, depth);
                    }
                    assert_eq!(browser.active_depth(), Some(depth));
                    assert_eq!(view.state.focused_column_depth(), Some(depth));
                    assert_column_header_actions(&view, depth);
                    assert_eq!(browser.selected_positions(0), [0]);
                    assert_eq!(browser.selected_positions(1), [0]);
                    assert!(browser.selected_positions(2).is_empty());
                    assert_eq!(view.state.columns.borrow().len(), 3);
                    wait_until(|| {
                        let columns = view.state.columns.borrow();
                        let bounds = columns[depth]
                            .shell
                            .compute_bounds(&view.state.columns_widget)
                            .expect("column bounds");
                        f64::from(bounds.x()) >= adjustment.value() - 1.0
                            && f64::from(bounds.x() + bounds.width())
                                <= adjustment.value() + adjustment.page_size() + 1.0
                    });
                }
            }
            let surface = view.state.columns.borrow()[0].presentation.stack.clone();
            let gesture = background_gesture(surface.upcast_ref());
            let y = f64::from(surface.height()) - 40.0;
            gesture.emit_by_name::<()>("pressed", &[&1i32, &30.0f64, &y]);
            assert_eq!(
                browser.active_depth(),
                Some(1),
                "press must not interrupt a drag"
            );
            gesture.emit_by_name::<()>("released", &[&1i32, &130.0f64, &y]);
            assert_eq!(browser.active_depth(), Some(1));
            gesture.emit_by_name::<()>("pressed", &[&1i32, &30.0f64, &y]);
            gesture.emit_by_name::<()>("stopped", &[]);
            gesture.emit_by_name::<()>("released", &[&1i32, &30.0f64, &y]);
            assert_eq!(browser.active_depth(), Some(1));
            window.destroy();
            browser.clear_observer();
        },
    );
}

#[test]
#[ignore = "requires a mapped GTK window; run this test alone"]
fn horizontal_scrollbar_stays_below_destination_hints() {
    const CHILD: &str = "STRATA_DESTINATION_SCROLLBAR_GTK_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let sandbox = tempfile::tempdir().expect("isolated preferences");
        let status = std::process::Command::new(std::env::current_exe().expect("test executable"))
            .args([
                "--exact",
                "ui::browser::tests::focus::horizontal_scrollbar_stays_below_destination_hints",
                "--nocapture",
                "--ignored",
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
#[ignore = "requires a mapped GTK window; run this test alone"]
fn pane_ownership_routes_commands_and_preserves_selection() {
    const CHILD: &str = "STRATA_FOCUS_GTK_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let sandbox = tempfile::tempdir().expect("isolated preferences");
        let status = std::process::Command::new(std::env::current_exe().expect("test executable"))
            .args([
                "--exact",
                "ui::browser::tests::focus::pane_ownership_routes_commands_and_preserves_selection",
                "--nocapture",
                "--ignored",
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
    assert_column_header_actions(&view, 0);
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
    assert_column_header_actions(&view, 0);
    assert_eq!(
        view.state.columns.borrow()[0].destination_hint.text(),
        "Pointer · Paste here"
    );
    view.keyboard_navigation();
    assert_column_header_actions(&view, 1);
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

    for mode in [BrowserMode::Icons, BrowserMode::List] {
        view.set_view_mode(mode);
        assert_eq!(view.state.destination_depth(), browser.active_depth());
    }
    window.destroy();
    browser.clear_observer();
}
