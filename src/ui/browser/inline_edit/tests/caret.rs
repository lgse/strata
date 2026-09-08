// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;

fn settle_frames(widget: &impl IsA<gtk::Widget>) {
    let frames = Rc::new(std::cell::Cell::new(0));
    let counted = frames.clone();
    widget.add_tick_callback(move |_, _| {
        counted.set(counted.get() + 1);
        if counted.get() == 3 {
            glib::ControlFlow::Break
        } else {
            glib::ControlFlow::Continue
        }
    });
    wait_until(|| frames.get() == 3);
}

fn assert_caret_visible(context: &str, field: &gtk::Entry, root: &gtk::Widget) {
    settle_frames(field);
    let text = field
        .delegate()
        .expect("rename editable delegate")
        .downcast::<gtk::Text>()
        .expect("GtkText delegate");
    assert!(field.is_mapped() && field.width() > 0 && text.width() > 0);
    let name_width = text
        .create_pango_layout(Some(field.text().as_str()))
        .pixel_size()
        .0;
    assert!(
        name_width > text.width(),
        "{context}: the name must overflow the editor"
    );
    let bounds = text.compute_bounds(root).expect("text bounds in browser");
    assert!(
        bounds.x() >= 0.0 && bounds.x() + bounds.width() <= root.width() as f32,
        "{context}: the text allocation {bounds:?} must fit inside the browser"
    );
    let (strong, _) = text.compute_cursor_extents(field.position() as usize);
    let x = bounds.x() + strong.x();
    assert!(
        strong.x() >= 0.0
            && strong.x() <= text.width() as f32
            && x >= 0.0
            && x <= root.width() as f32,
        "{context}: caret x={x}, local x={}, text width={}, browser width={}",
        strong.x(),
        text.width(),
        root.width()
    );
}

#[test]
fn long_filenames_keep_the_rename_caret_visible_in_every_view_mode() {
    gtk_test(
        "ui::browser::inline_edit::tests::caret::long_filenames_keep_the_rename_caret_visible_in_every_view_mode",
        || {
            let fixture = tempfile::tempdir().expect("directory fixture");
            let name = "synthetic-quarterly-report-with-a-very-long-descriptive-basename-2026.txt";
            std::fs::write(fixture.path().join(name), b"body").expect("fixture file");

            for (mode, width) in [
                (BrowserMode::Columns, 420),
                (BrowserMode::Columns, 210),
                (BrowserMode::List, 420),
                (BrowserMode::Icons, 420),
            ] {
                let view = BrowserView::new(
                    Rc::new(crate::adapters::LocalFileSource),
                    PeekBehavior::default(),
                );
                view.set_view_mode(mode);
                let root = view.widget();
                let window = gtk::Window::builder()
                    .child(&root)
                    .default_width(width)
                    .default_height(300)
                    .build();
                window.present();
                let browser = view.browser();
                browser.navigate(Location::local(fixture.path()));
                wait_until(|| {
                    browser
                        .column_snapshot(0)
                        .is_some_and(|snapshot| !snapshot.loading && snapshot.count == 1)
                });
                browser.select(0, 0);
                wait_until(|| view.state.begin_rename());
                let field = view
                    .state
                    .active_rename
                    .borrow()
                    .as_ref()
                    .map(|rename| rename.field.clone())
                    .or_else(|| view.state.mode_views.borrow().active_rename_field())
                    .unwrap_or_else(|| panic!("{mode:?} did not open an inline rename field"));
                wait_until(|| field.is_mapped() && field.width() > 0);
                assert_eq!(field.selection_bounds(), Some((0, rename_stem_end(name))));
                assert_caret_visible(&format!("{mode:?} after opening rename"), &field, &root);
                assert_eq!(root.width(), width, "the fixture must stay constrained");

                field.set_position(-1);
                assert_eq!(field.position(), name.chars().count() as i32);
                assert_caret_visible(&format!("{mode:?} at the filename end"), &field, &root);

                let mut position = field.position();
                field.insert_text(" (final)", &mut position);
                field.set_position(position);
                assert_caret_visible(&format!("{mode:?} after insertion"), &field, &root);

                field.delete_text(position - 4, position);
                field.set_position(position - 4);
                assert_caret_visible(&format!("{mode:?} after deletion"), &field, &root);

                field.set_position(20);
                assert_caret_visible(&format!("{mode:?} at an interior position"), &field, &root);

                if mode == BrowserMode::Columns {
                    window.set_default_size(width + 120, 300);
                    assert_caret_visible("after growing the viewport", &field, &root);
                    window.set_default_size(width, 300);
                    field.set_position(-1);
                    assert_caret_visible("after shrinking the viewport", &field, &root);
                }
                assert!(view.state.cancel_rename());
                assert_eq!(field.margin_start(), 0);
                assert_eq!(field.margin_end(), 0);
                browser.clear_observer();
                window.destroy();
            }
        },
    );
}
