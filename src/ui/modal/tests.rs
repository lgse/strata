// SPDX-License-Identifier: MIT

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
fn repeated_dismissal_does_not_repeat_the_confirmed_operation() {
    crate::test_support::gtk_test(
        "ui::modal::tests::repeated_dismissal_does_not_repeat_the_confirmed_operation",
        || {
            let overlay = gtk::Overlay::new();
            let layer = modal_layer(&gtk::Label::new(None), &overlay, None, None);
            overlay.add_overlay(&layer);
            let calls = Rc::new(Cell::new(0));
            for _ in 0..2 {
                let calls = calls.clone();
                dismiss_modal_layer_then(&layer, &overlay, None, move || {
                    calls.set(calls.get() + 1);
                });
            }
            wait_until(|| layer.parent().is_none());
            assert_eq!(calls.get(), 1);
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

#[test]
fn modal_focus_restoration_preserves_explicit_action_focus() {
    crate::test_support::gtk_test(
        "ui::modal::tests::modal_focus_restoration_preserves_explicit_action_focus",
        || {
            let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
            let origin = gtk::Button::with_label("Origin");
            let action_target = gtk::Entry::new();
            body.append(&origin);
            body.append(&action_target);
            let overlay = gtk::Overlay::new();
            overlay.set_child(Some(&body));
            let window = gtk::Window::builder().child(&overlay).build();
            window.present();
            origin.grab_focus();
            let modal_field = gtk::Entry::new();
            let layer = modal_layer(&modal_field, &overlay, None, None);
            let restore = remember_modal_focus(&layer, &overlay);
            overlay.add_overlay(&layer);
            modal_field.grab_focus();
            let unwanted_restores = Rc::new(Cell::new(0));
            let restored = unwanted_restores.clone();
            origin.connect_has_focus_notify(move |origin| {
                if origin.has_focus() {
                    restored.set(restored.get() + 1);
                }
            });
            restore.set(false);
            let target = action_target.clone();
            dismiss_modal_layer_then(&layer, &overlay, None, move || {
                target.grab_focus();
            });
            wait_until(|| layer.parent().is_none());
            let focus = gtk::prelude::RootExt::focus(&window).expect("action focus");
            assert!(
                focus == action_target.clone().upcast::<gtk::Widget>()
                    || focus.is_ancestor(&action_target)
            );
            assert!(!origin.has_focus());
            assert_eq!(unwanted_restores.get(), 0);
            window.destroy();
        },
    );
}

struct FocusFixture {
    window: gtk::Window,
    overlay: gtk::Overlay,
    origin: gtk::Button,
    target: gtk::Button,
    fallbacks: Rc<Cell<usize>>,
}

impl FocusFixture {
    fn new() -> Self {
        let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let origin = gtk::Button::with_label("Origin");
        let target = gtk::Button::with_label("File list");
        body.append(&origin);
        body.append(&target);
        let overlay = gtk::Overlay::new();
        overlay.set_child(Some(&body));
        let window = gtk::Window::builder().child(&overlay).build();
        crate::ui::window::install_modal_focus_trap(&window);
        let fallbacks = Rc::new(Cell::new(0));
        let counted = fallbacks.clone();
        let fallback_target = target.clone();
        set_modal_focus_fallback(
            &window,
            Rc::new(move || {
                counted.set(counted.get() + 1);
                fallback_target.grab_focus();
            }),
        );
        window.present();
        assert!(origin.grab_focus());
        Self {
            window,
            overlay,
            origin,
            target,
            fallbacks,
        }
    }

    fn assert_focus(&self, expected: &gtk::Button, fallbacks: usize, case: &str) {
        let focus = gtk::prelude::RootExt::focus(&self.window).and_downcast::<gtk::Button>();
        assert_eq!(
            focus.as_ref(),
            Some(expected),
            "{case}: focus is on {:?}",
            focus.as_ref().and_then(gtk::Button::label)
        );
        assert_eq!(self.fallbacks.get(), fallbacks, "{case}: fallback calls");
    }
}

#[derive(Clone, Copy, Debug)]
enum Closing {
    OriginOnScreen,
    OriginHidden,
    FocusTakenMeanwhile,
}

#[test]
fn dismissed_modal_restores_the_origin_or_falls_back_to_the_window_target() {
    crate::test_support::gtk_test(
        "ui::modal::tests::dismissed_modal_restores_the_origin_or_falls_back_to_the_window_target",
        || {
            for closing in [
                Closing::OriginOnScreen,
                Closing::OriginHidden,
                Closing::FocusTakenMeanwhile,
            ] {
                let fixture = FocusFixture::new();
                let elsewhere = gtk::Button::with_label("Elsewhere");
                fixture
                    .origin
                    .parent()
                    .and_downcast::<gtk::Box>()
                    .expect("fixture body")
                    .append(&elsewhere);
                let layer =
                    modal_layer(&gtk::Button::with_label("Close"), &fixture.overlay, None, None);
                remember_modal_focus(&layer, &fixture.overlay);
                fixture.overlay.add_overlay(&layer);
                layer.grab_focus();
                if matches!(closing, Closing::OriginHidden) {
                    fixture.origin.set_visible(false);
                }
                dismiss_modal_layer(&layer, &fixture.overlay, None);
                if matches!(closing, Closing::FocusTakenMeanwhile) {
                    assert!(elsewhere.grab_focus());
                }
                wait_until(|| layer.parent().is_none());
                let case = format!("{closing:?}");
                match closing {
                    Closing::OriginOnScreen => fixture.assert_focus(&fixture.origin, 0, &case),
                    Closing::OriginHidden => fixture.assert_focus(&fixture.target, 1, &case),
                    Closing::FocusTakenMeanwhile => fixture.assert_focus(&elsewhere, 0, &case),
                }
                fixture.window.destroy();
            }
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
