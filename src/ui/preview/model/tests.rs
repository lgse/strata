// SPDX-License-Identifier: MIT

use super::*;

fn pump_until(predicate: impl Fn() -> bool) {
    let context = glib::MainContext::default();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !predicate() {
        assert!(
            std::time::Instant::now() < deadline,
            "camera motion timed out"
        );
        context.iteration(false);
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

#[test]
fn camera_animation_stops_for_reduced_motion_and_when_the_preview_unmaps() {
    crate::test_support::gtk_test(
        "ui::preview::model::tests::camera_animation_stops_for_reduced_motion_and_when_the_preview_unmaps",
        || {
            let settings = gtk::Settings::default().expect("GTK settings");
            settings.set_gtk_enable_animations(true);
            crate::ui::motion::set_reduce_motion(false);
            let motion = Rc::new(CameraMotion::new(ModelView::FrontLeft));
            let camera = camera_position(motion.clone());
            let window = gtk::Window::builder().child(&camera).build();
            window.present();
            pump_until(|| camera.is_mapped());

            let initial = motion.angle.get();
            motion.set_view(&camera, motion.view.get().step(1));
            assert!(motion.tick.borrow().is_some());
            assert_eq!(motion.angle.get(), initial);
            pump_until(|| motion.angle.get() != initial);
            assert!(motion.angle.get() > initial, "Next turns clockwise");
            pump_until(|| motion.tick.borrow().is_none());
            assert_eq!(motion.angle.get(), motion.target.get());

            motion.set_view(&camera, ModelView::BackRight);
            assert!(motion.tick.borrow().is_some());
            settings.set_gtk_enable_animations(false);
            pump_until(|| motion.tick.borrow().is_none());
            assert_eq!(motion.angle.get(), motion.target.get());

            settings.set_gtk_enable_animations(true);
            crate::ui::motion::set_reduce_motion(true);
            motion.set_view(&camera, ModelView::BackLeft);
            assert!(motion.tick.borrow().is_none());
            assert_eq!(motion.angle.get(), motion.target.get());

            crate::ui::motion::set_reduce_motion(false);
            motion.set_view(&camera, ModelView::FrontLeft);
            assert!(motion.tick.borrow().is_some());
            window.close();
            pump_until(|| !camera.is_mapped());
            assert!(motion.tick.borrow().is_none());
            assert_eq!(motion.angle.get(), motion.target.get());
        },
    );
}
