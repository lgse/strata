// SPDX-License-Identifier: MIT

use super::*;

fn wait_for_allocation(widget: &gtk::Widget) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while widget.width() <= 0 || widget.height() <= 0 {
        assert!(
            std::time::Instant::now() < deadline,
            "widget was not allocated"
        );
        glib::MainContext::default().iteration(false);
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

fn middle_gesture(scroll: &gtk::ScrolledWindow) -> gtk::GestureClick {
    let controllers = scroll.observe_controllers();
    (0..controllers.n_items())
        .filter_map(|index| controllers.item(index).and_downcast::<gtk::GestureClick>())
        .find(|click| click.button() == gtk::gdk::BUTTON_MIDDLE)
        .expect("middle-click autoscroll gesture")
}

fn press(gesture: &gtk::GestureClick, x: f64, y: f64) {
    gesture.emit_by_name::<()>("pressed", &[&1i32, &x, &y]);
}

#[test]
fn middle_click_autoscroll_skips_rows_and_anchors_on_background() {
    crate::test_support::gtk_test(
        "ui::scrolling::tests::middle_click_autoscroll_skips_rows_and_anchors_on_background",
        || {
            let scroll = gtk::ScrolledWindow::new();
            scroll.set_hexpand(true);
            scroll.set_vexpand(true);
            let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            row.add_css_class("file-row");
            row.set_height_request(40);
            row.append(&gtk::Label::new(Some("a.txt")));
            content.append(&row);
            scroll.set_child(Some(&content));
            let overlay = gtk::Overlay::new();
            overlay.set_child(Some(&scroll));
            let window = gtk::Window::new();
            window.set_child(Some(&overlay));
            window.set_default_size(240, 200);
            window.present();
            wait_for_allocation(scroll.upcast_ref());

            // Make the view scrollable so `AutoScroll::start` accepts the anchor.
            let adjustment = scroll.vadjustment();
            adjustment.set_upper(adjustment.lower() + adjustment.page_size() + 400.0);
            install_autoscroll(&scroll, &overlay);
            let gesture = middle_gesture(&scroll);

            let bounds = row
                .compute_bounds(&scroll)
                .expect("row is allocated within the scroll");
            press(
                &gesture,
                f64::from(bounds.x()) + 4.0,
                f64::from(bounds.y()) + f64::from(bounds.height()) / 2.0,
            );
            assert!(
                !autoscroll_is_running(),
                "a middle press over a row must not anchor autoscroll"
            );

            press(&gesture, 4.0, scroll.height() as f64 - 4.0);
            assert!(
                autoscroll_is_running(),
                "a middle press over the view background must anchor autoscroll"
            );

            assert!(stop_autoscroll());
            window.destroy();
        },
    );
}

#[test]
fn a_sideways_grid_pages_by_whole_columns_and_scrolls_sideways() {
    crate::test_support::gtk_test(
        "ui::scrolling::tests::a_sideways_grid_pages_by_whole_columns_and_scrolls_sideways",
        || {
            let names: Vec<String> = (0..200).map(|index| format!("{index}")).collect();
            let names: Vec<&str> = names.iter().map(String::as_str).collect();
            let model = gtk::StringList::new(&names);
            let factory = gtk::SignalListItemFactory::new();
            factory.connect_setup(|_, object| {
                let item = object.downcast_ref::<gtk::ListItem>().expect("list item");
                let label = gtk::Label::new(Some("Item"));
                label.set_size_request(80, 40);
                item.set_child(Some(&label));
            });
            let grid = gtk::GridView::new(Some(gtk::NoSelection::new(Some(model))), Some(factory));
            grid.set_orientation(gtk::Orientation::Horizontal);
            grid.set_max_columns(4);
            let scroll = gtk::ScrolledWindow::builder()
                .child(&grid)
                .vscrollbar_policy(gtk::PolicyType::Never)
                .build();
            let window = gtk::Window::new();
            window.set_child(Some(&scroll));
            window.set_default_size(400, 200);
            window.present();
            wait_for_allocation(grid.upcast_ref());
            let view = grid.upcast_ref::<gtk::Widget>();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while grid_cell_pitch(view).is_none() || scroll.hadjustment().upper() <= 400.0 {
                assert!(std::time::Instant::now() < deadline, "grid never filled");
                glib::MainContext::default().iteration(false);
            }

            let page = page(view, &scroll);
            let (column_width, _) = grid_cell_pitch(view).expect("cell size");
            let columns = rows_per_page(scroll.hadjustment().page_size(), column_width);
            assert_eq!(
                page.items,
                columns * 4,
                "a page is whole columns of four rows"
            );
            reveal_selection(view, &scroll, 1, &page);
            assert_eq!(scroll.hadjustment().value(), page.distance);
            assert_eq!(scroll.vadjustment().value(), 0.0);
            window.close();
        },
    );
}
