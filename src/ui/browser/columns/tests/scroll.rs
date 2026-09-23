// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn horizontal_gestures_scroll_columns_without_stealing_vertical_input() {
    crate::test_support::gtk_test(
        "ui::browser::columns::tests::scroll::horizontal_gestures_scroll_columns_without_stealing_vertical_input",
        || {
            let view = crate::ui::browser::BrowserView::new(
                Rc::new(crate::adapters::LocalFileSource),
                crate::ui::browser::PeekBehavior::default(),
            );
            let state = &view.state;
            let adjustment = state.scroller.hadjustment();
            adjustment.configure(200.0, 0.0, 1200.0, 10.0, 100.0, 400.0);
            let controller = state
                .scroller
                .observe_controllers()
                .iter::<glib::Object>()
                .filter_map(Result::ok)
                .filter_map(|object| object.downcast::<gtk::EventControllerScroll>().ok())
                .find(|controller| controller.propagation_phase() == gtk::PropagationPhase::Capture)
                .expect("horizontal scroll capture controller");
            let scroll = |dx: f64, dy: f64| controller.emit_by_name::<bool>("scroll", &[&dx, &dy]);
            let generation = state.horizontal_scroll_generation.get();
            assert!(!scroll(0.0, 1.0));
            assert!(!scroll(0.1, 1.0));
            assert!(!scroll(0.0, 0.0));
            assert_eq!(adjustment.value(), 200.0);
            assert_eq!(state.horizontal_scroll_generation.get(), generation);

            assert!(scroll(1.0, 0.1));
            assert!(adjustment.value() > 200.0);
            assert!(state.horizontal_scroll_generation.get() > generation);
            assert!(scroll(-1.0, 0.0));
            assert!((adjustment.value() - 200.0).abs() < 0.001);
            assert!(scroll(1000.0, 0.0));
            assert_eq!(adjustment.value(), 800.0);
            assert!(scroll(-1000.0, 0.0));
            assert_eq!(adjustment.value(), 0.0);

            adjustment.configure(0.0, 0.0, 400.0, 10.0, 100.0, 400.0);
            assert!(!scroll(1.0, 0.0));
            assert_eq!(adjustment.value(), 0.0);
        },
    );
}
