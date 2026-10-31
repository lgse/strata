// SPDX-License-Identifier: MIT

use super::*;

mod acceptance;
mod column_widths;
mod confined;
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
fn a_hidden_chooser_pauses_its_checks() {
    crate::test_support::gtk_test(
        "ui::chooser::tests::a_hidden_chooser_pauses_its_checks",
        || {
            crate::ui::prepare_portal_ui();
            let root = tempfile::tempdir().expect("fixture");
            let state = build_chooser(
                acceptance::request(root.path().to_path_buf()),
                Arc::new(AtomicBool::new(false)),
                |_| {},
            )
            .expect("chooser");
            let presence = || state.view.browser().freshness_input().presence;
            let shown = presence();
            state.window.set_visible(false);
            let hidden = presence();
            state.window.present();

            assert_ne!(shown, crate::app::Presence::Hidden);
            assert_eq!(hidden, crate::app::Presence::Hidden);
            assert_ne!(presence(), crate::app::Presence::Hidden);
            state.window.close();
        },
    );
}
