// SPDX-License-Identifier: MIT

use super::*;

mod acceptance;
mod column_widths;
mod filtered_preview;
mod image_conversion;
mod keyboard;
mod sizing;

pub(super) fn widget_with_class(widget: &gtk::Widget, class: &str) -> Option<gtk::Widget> {
    if widget.has_css_class(class) {
        return Some(widget.clone());
    }
    let mut child = widget.first_child();
    while let Some(current) = child {
        if let Some(found) = widget_with_class(&current, class) {
            return Some(found);
        }
        child = current.next_sibling();
    }
    None
}

#[test]
fn close_button_follows_the_live_decoration_layout() {
    crate::test_support::gtk_test(
        "ui::chooser::tests::close_button_follows_the_live_decoration_layout",
        || {
            crate::ui::prepare_portal_ui();
            let root = tempfile::tempdir().expect("fixture");
            let state = build_chooser(
                acceptance::request(root.path().to_path_buf()),
                Arc::new(AtomicBool::new(false)),
                |_| {},
            )
            .expect("chooser");
            let leading = widget_with_class(state.window.upcast_ref(), "window-controls-start")
                .and_downcast::<gtk::Box>()
                .expect("leading window controls");
            let settings = state.window.settings();
            settings.set_gtk_decoration_layout(Some("close:appmenu"));
            assert!(leading.first_child().is_some());
            settings.set_gtk_decoration_layout(Some("appmenu:close"));
            assert!(leading.first_child().is_none());
            state.window.close();
        },
    );
}
