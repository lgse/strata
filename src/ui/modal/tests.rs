// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;
use gtk::subclass::prelude::ObjectSubclassIsExt;
use std::time::Instant;

#[test]
fn modal_hosts_preserve_nested_blur_and_support_plain_overlays() {
    crate::test_support::gtk_test(
        "ui::modal::tests::modal_hosts_preserve_nested_blur_and_support_plain_overlays",
        || {
            let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
            assert!(ModalHost::blurred_for(&content).is_none());
            let root = BlurBin::new(&content);
            let overlay = gtk::Overlay::new();
            overlay.set_child(Some(&root));
            let window = gtk::Window::builder().child(&overlay).build();
            window.present();
            assert!(!root.imp().blurred.get());
            let host = ModalHost::blurred_for(&content).expect("modal host");
            assert_eq!(host.overlay, overlay);
            assert_eq!(host.blurred_root.as_ref(), Some(&root));
            assert!(root.imp().blurred.get());

            let first = modal_layer(&gtk::Label::new(None), &overlay, Some(root.clone()), None);
            let second = modal_layer(&gtk::Label::new(None), &overlay, Some(root.clone()), None);
            overlay.add_overlay(&first);
            overlay.add_overlay(&second);
            dismiss_modal_layer(&first, &overlay, Some(&root));
            dismiss_modal_layer(&first, &overlay, Some(&root));
            wait_until(|| first.parent().is_none());
            assert!(root.imp().blurred.get(), "remaining modal must retain blur");
            dismiss_modal_layer(&second, &overlay, Some(&root));
            wait_until(|| second.parent().is_none());
            assert!(!root.imp().blurred.get());
            window.destroy();

            let plain = gtk::Overlay::new();
            let label = gtk::Label::new(None);
            plain.set_child(Some(&label));
            let window = gtk::Window::builder().child(&plain).build();
            let host = ModalHost::blurred_for(&label).expect("plain overlay host");
            assert_eq!(host.overlay, plain);
            assert!(host.blurred_root.is_none());
            window.destroy();
        },
    );
}

#[test]
fn enter_in_a_single_line_field_invokes_the_primary_action() {
    crate::test_support::gtk_test(
        "ui::modal::tests::enter_in_a_single_line_field_invokes_the_primary_action",
        || {
            let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
            let nested = gtk::Box::new(gtk::Orientation::Vertical, 0);
            let name = gtk::Entry::new();
            let password = gtk::PasswordEntry::new();
            nested.append(&password);
            nested.append(&gtk::TextView::new());
            body.append(&name);
            body.append(&nested);

            let confirm = gtk::Button::with_label("Compress");
            let clicks = Rc::new(Cell::new(0_usize));
            let counted = clicks.clone();
            confirm.connect_clicked(move |_| counted.set(counted.get() + 1));
            submit_on_enter(&body, &confirm);

            name.emit_by_name::<()>("activate", &[]);
            assert_eq!(clicks.get(), 1, "a text field should submit the form");
            password.emit_by_name::<()>("activate", &[]);
            assert_eq!(clicks.get(), 2, "a nested password field should submit too");

            confirm.set_sensitive(false);
            name.emit_by_name::<()>("activate", &[]);
            assert_eq!(clicks.get(), 2, "a disabled primary action stays inert");
        },
    );
}

fn wait_until(condition: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !condition() {
        assert!(Instant::now() < deadline, "modal did not dismiss");
        glib::MainContext::default().iteration(false);
        std::thread::sleep(Duration::from_millis(1));
    }
}
