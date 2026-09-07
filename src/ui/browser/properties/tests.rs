// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;
use std::time::{Duration, Instant};

fn size_label(widget: &gtk::Widget) -> Option<gtk::Label> {
    if widget.has_css_class("properties-row")
        && widget
            .first_child()
            .and_downcast::<gtk::Label>()
            .is_some_and(|label| label.text() == "SIZE")
    {
        return widget.last_child().and_downcast::<gtk::Label>();
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        if let Some(label) = size_label(&widget) {
            return Some(label);
        }
        child = widget.next_sibling();
    }
    None
}

#[test]
fn folder_properties_loads_sizes_and_reports_unavailable_roots() {
    crate::test_support::gtk_test(
        "ui::browser::properties::tests::folder_properties_loads_sizes_and_reports_unavailable_roots",
        || {
            let root = tempfile::tempdir().expect("fixture");
            std::fs::create_dir(root.path().join("empty")).expect("empty folder");
            std::fs::create_dir(root.path().join("nested")).expect("nested folder");
            std::fs::write(root.path().join("nested/.hidden"), b"12345").expect("hidden file");
            let view = crate::ui::browser::BrowserView::new(
                Rc::new(crate::adapters::LocalFileSource),
                crate::ui::browser::PeekBehavior::default(),
            );
            let overlay = gtk::Overlay::new();
            overlay.set_child(Some(&view.widget()));
            let window = gtk::Window::builder().child(&overlay).build();
            window.present();

            for (path, expected) in [
                (root.path().to_path_buf(), "5 B"),
                (root.path().join("empty"), "0 B"),
                (root.path().join("missing"), "Unavailable"),
            ] {
                view.state.show_folder_properties(&Location::local(path));
                let size = size_label(overlay.upcast_ref()).expect("Properties SIZE row");
                assert_eq!(size.text(), "Calculating…");
                let deadline = Instant::now() + Duration::from_secs(5);
                while size.text() != expected {
                    assert!(Instant::now() < deadline, "SIZE stayed at {}", size.text());
                    glib::MainContext::default().iteration(false);
                    std::thread::sleep(Duration::from_millis(1));
                }
                let layer = overlay
                    .last_child()
                    .and_downcast::<gtk::Box>()
                    .expect("modal layer");
                dismiss_modal_layer(&layer, &overlay, None);
                while layer.parent().is_some() {
                    assert!(Instant::now() < deadline, "Properties did not close");
                    glib::MainContext::default().iteration(false);
                    std::thread::sleep(Duration::from_millis(1));
                }
            }
            window.destroy();
            view.browser().clear_observer();
        },
    );
}

#[test]
fn properties_permissions_are_formatted_symbolically_and_numerically() {
    assert_eq!(format_permissions(0o100774), "-rwxrwxr--  774");
    assert_eq!(format_permissions(0o040755), "drwxr-xr-x  755");
}

#[test]
fn individual_permission_bits_can_be_toggled_without_changing_file_type() {
    assert_eq!(toggled_permission(0o100644, 0o100), 0o100744);
    assert_eq!(toggled_permission(0o100744, 0o100), 0o100644);
}

#[test]
fn executable_toggle_changes_all_execute_bits_and_preserves_other_bits() {
    assert_eq!(with_execute_permissions(0o100644, true), 0o100755);
    assert_eq!(with_execute_permissions(0o100775, false), 0o100664);
}
