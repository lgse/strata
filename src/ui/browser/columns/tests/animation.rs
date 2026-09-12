// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn column_entry_moves_right_from_the_left_without_resizing_the_column() {
    crate::test_support::gtk_test(
        "ui::browser::columns::tests::animation::column_entry_moves_right_from_the_left_without_resizing_the_column",
        || {
            crate::ui::window::load_styles();
            crate::ui::motion::set_reduce_motion(false);
            gtk::Settings::default()
                .expect("GTK settings")
                .set_gtk_enable_animations(true);
            let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
            column.add_css_class("directory-column");
            column.set_size_request(COLUMN_WIDTH, 100);
            let shell = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            shell.set_overflow(gtk::Overflow::Hidden);
            shell.append(&column);
            let window = gtk::Window::builder().child(&shell).build();
            window.present();
            wait_until(|| column.width() >= COLUMN_WIDTH);
            let width = column.width();
            let generation = Rc::new(Cell::new(0));
            animate_column_entry(&column, &generation);
            wait_until(|| {
                column
                    .compute_bounds(&shell)
                    .is_some_and(|bounds| bounds.x() < -1.0)
            });
            assert_eq!(column.width(), width);
            assert_eq!(column.margin_start(), 0);
            wait_until(|| !column.has_css_class("column-entering"));
            wait_until(|| {
                column
                    .compute_bounds(&shell)
                    .is_some_and(|bounds| bounds.x().abs() < 0.1)
            });
            assert_eq!(column.width(), width);
            window.destroy();
        },
    );
}

fn wait_until(condition: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !condition() {
        assert!(Instant::now() < deadline, "column animation did not settle");
        glib::MainContext::default().iteration(false);
        std::thread::sleep(Duration::from_millis(1));
    }
}
