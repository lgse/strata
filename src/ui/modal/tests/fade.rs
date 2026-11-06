// SPDX-License-Identifier: MIT

use super::*;
use crate::ui::modal::{fade_in, fade_out};
use crate::ui::motion::set_reduce_motion;

fn presented_layer() -> (gtk::Window, gtk::Box) {
    gtk::Settings::default()
        .expect("GTK settings")
        .set_gtk_enable_animations(true);
    let layer = gtk::Box::new(gtk::Orientation::Vertical, 0);
    layer.add_css_class("app-modal-layer");
    let root = gtk::Overlay::new();
    root.add_css_class("tab-window");
    root.add_overlay(&layer);
    let window = gtk::Window::builder().child(&root).build();
    window.present();
    wait_until(|| layer.is_mapped());
    (window, layer)
}

fn done_flag() -> (Rc<Cell<u32>>, impl FnOnce() + 'static) {
    let calls = Rc::new(Cell::new(0));
    let counted = calls.clone();
    (calls, move || counted.set(counted.get() + 1))
}

fn run_for(duration: Duration) {
    let until = Instant::now() + duration;
    wait_until(|| Instant::now() >= until);
}

#[test]
fn a_fading_in_layer_counts_as_open_and_reaches_full_opacity() {
    crate::test_support::gtk_test(
        "ui::modal::tests::fade::a_fading_in_layer_counts_as_open_and_reaches_full_opacity",
        || {
            let (window, layer) = presented_layer();

            fade_in(layer.upcast_ref());
            assert_eq!(
                crate::ui::window::visible_modal_layer(&window),
                Some(layer.clone().upcast())
            );
            assert!(layer.opacity() < 1.0);
            wait_until(|| layer.opacity() == 1.0);

            window.destroy();
        },
    );
}

#[test]
fn dismissing_during_a_fade_in_ends_transparent() {
    crate::test_support::gtk_test(
        "ui::modal::tests::fade::dismissing_during_a_fade_in_ends_transparent",
        || {
            let (window, layer) = presented_layer();

            fade_in(layer.upcast_ref());
            layer.add_css_class("dismissing");
            let (calls, done) = done_flag();
            fade_out(layer.upcast_ref(), done);
            wait_until(|| calls.get() == 1);
            assert_eq!(layer.opacity(), 0.0);

            run_for(Duration::from_millis(300));
            assert_eq!(calls.get(), 1);
            assert_eq!(layer.opacity(), 0.0, "the fade in must not resume");

            window.destroy();
        },
    );
}

#[test]
fn a_stalled_frame_does_not_use_up_the_fade() {
    crate::test_support::gtk_test(
        "ui::modal::tests::fade::a_stalled_frame_does_not_use_up_the_fade",
        || {
            let (window, layer) = presented_layer();

            fade_in(layer.upcast_ref());
            let started = layer.opacity();
            wait_until(|| layer.opacity() != started);
            std::thread::sleep(Duration::from_millis(300));
            let first_frame_after = Rc::new(Cell::new(None));
            let recorded = first_frame_after.clone();
            let frames = Cell::new(0);
            // GTK runs the newest callback first, so the stalled frame's step is
            // only visible from the frame after it.
            layer.add_tick_callback(move |layer, _| {
                frames.set(frames.get() + 1);
                if frames.get() < 2 {
                    return glib::ControlFlow::Continue;
                }
                recorded.set(Some(layer.opacity()));
                glib::ControlFlow::Break
            });
            wait_until(|| first_frame_after.get().is_some());
            assert!(first_frame_after.get().expect("recorded opacity") < 1.0);

            window.destroy();
        },
    );
}

#[test]
fn fades_finish_without_animating_when_they_cannot_be_seen() {
    crate::test_support::gtk_test(
        "ui::modal::tests::fade::fades_finish_without_animating_when_they_cannot_be_seen",
        || {
            let (window, shown) = presented_layer();
            let unmapped = gtk::Box::new(gtk::Orientation::Vertical, 0);
            for (layer, reduce_motion) in [(&shown, true), (&unmapped, false)] {
                set_reduce_motion(reduce_motion);
                layer.set_opacity(0.5);
                fade_in(layer.upcast_ref());
                assert_eq!(layer.opacity(), 1.0);

                let (calls, done) = done_flag();
                fade_out(layer.upcast_ref(), done);
                assert_eq!(calls.get(), 0, "closing must not hide during its own event");
                wait_until(|| calls.get() == 1);
            }
            set_reduce_motion(false);
            window.destroy();
        },
    );
}
