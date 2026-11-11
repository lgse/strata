//! C14a's page: what the viewport does in process, beside its goldens and its script.
//!
//! No feature-map bullet describes zooming or panning an image (Strata's image preview has
//! neither), so the WP's scripts for wheel zoom about the pointer, pinch, fit after a resize,
//! pan limits and the keyboard are these tests: Runtime input in, viewport state out.

mod c14a {
    use rune::{Env, Key, KeyEvent, MagnifyEvent, Mods, Runtime, WheelEvent};
    use strata_components::viewers::viewport::Mode;
    use strata_components::viewers::viewport::zoom::wheel_factor;
    use strata_gallery::{Gallery, scenes};

    const DT: f64 = 1.0 / 60.0;

    fn scene(name: &str) -> Runtime<Gallery> {
        let f = scenes::all_scenes()
            .into_iter()
            .find(|(n, _)| *n == name)
            .map(|(_, f)| f)
            .unwrap_or_else(|| panic!("no scene {name}"));
        let mut g = Gallery::default();
        f(&mut g);
        let env = Env::new(g.dark, 1.0);
        let mut rt = strata_gallery::runtime(g, env);
        rt.frame(0.0);
        settle(&mut rt);
        rt
    }

    fn settle(rt: &mut Runtime<Gallery>) {
        for _ in 0..30 {
            rt.frame(DT);
        }
    }

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() <= 1e-3 * a.abs().max(b.abs()).max(1.0)
    }

    fn near2(a: [f32; 2], b: [f32; 2]) -> bool {
        (a[0] - b[0]).abs() <= 0.05 && (a[1] - b[1]).abs() <= 0.05
    }

    fn level(rt: &Runtime<Gallery>) -> f32 {
        rt.state.c14a.view.level().expect("laid out")
    }

    /// The viewport's frame in window points.
    fn frame_rect(rt: &Runtime<Gallery>) -> [f32; 4] {
        rt.elements()
            .into_iter()
            .find(|e| e.key.as_deref() == Some("c14a-viewport"))
            .expect("the viewport")
            .rect
    }

    /// The content pixel under window point `p`.
    fn under(rt: &Runtime<Gallery>, p: [f32; 2]) -> [f32; 2] {
        let r = frame_rect(rt);
        rt.state
            .c14a
            .view
            .content_at([p[0] - r[0], p[1] - r[1]])
            .expect("laid out")
    }

    fn ctrl() -> Mods {
        Mods {
            cmd: true,
            ctrl: true,
            ..Mods::default()
        }
    }

    fn key(rt: &mut Runtime<Gallery>, key: Key, mods: Mods) {
        rt.key(KeyEvent { key, mods });
        settle(rt);
    }

    fn text(t: &str) -> Key {
        Key::Text(t.into())
    }

    /// A click on the viewport, which focuses it.
    fn focus(rt: &mut Runtime<Gallery>) {
        let (x, y) = rt.find("c14a-viewport").expect("the viewport");
        rt.pointer_move(x, y);
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        settle(rt);
    }

    fn drag(rt: &mut Runtime<Gallery>, from: [f32; 2], by: [f32; 2]) {
        rt.pointer_move(from[0], from[1]);
        rt.pointer_down(from[0], from[1]);
        for i in 1..=6 {
            let t = i as f32 / 6.0;
            rt.pointer_move(from[0] + by[0] * t, from[1] + by[1] * t);
            rt.frame(DT);
        }
        rt.pointer_up(from[0] + by[0], from[1] + by[1]);
        settle(rt);
    }

    #[test]
    fn ctrl_wheel_zooms_about_the_pointer() {
        let mut rt = scene("c14a-fit-light");
        let fit = level(&rt);
        let p = [300.0, 250.0];
        let before = under(&rt, p);
        rt.wheel_event(WheelEvent::new(0.0, 40.0).at(p[0], p[1]).mods(ctrl()));
        settle(&mut rt);
        assert!(near(level(&rt), fit * wheel_factor(40.0)));
        assert!(level(&rt) > fit, "a wheel turned away zooms in");
        assert_eq!(rt.state.c14a.view.mode(), Mode::Free);
        assert!(
            near2(under(&rt, p), before),
            "{:?} then {:?}",
            before,
            under(&rt, p)
        );
        assert_eq!(rt.state.c14a.reports, 2, "the fit, then the zoom");
    }

    /// A pinch zooms about the pointer on macOS. Elsewhere it arrives as Ctrl+wheel (the test
    /// above), and the viewport has no pinch handler.
    #[test]
    fn a_pinch_zooms_about_the_pointer_on_macos() {
        let mut rt = scene("c14a-fit-light");
        let fit = level(&rt);
        let p = [700.0, 420.0];
        let before = under(&rt, p);
        for _ in 0..2 {
            rt.magnify_event(MagnifyEvent::new(0.5).at(p[0], p[1]));
            settle(&mut rt);
        }
        if !cfg!(target_os = "macos") {
            assert!(near(level(&rt), fit), "no pinch handler off macOS");
            return;
        }
        assert!(near(level(&rt), fit * 1.0f32.exp()));
        assert!(near2(under(&rt, p), before));
        // Pinching back in the same steps returns to fit.
        for _ in 0..2 {
            rt.magnify_event(MagnifyEvent::new(-0.5).at(p[0], p[1]));
            settle(&mut rt);
        }
        assert!(near(level(&rt), fit));
    }

    #[test]
    fn fit_follows_a_resize_and_a_zoom_keeps_its_level() {
        let mut rt = scene("c14a-fit-light");
        let fit = level(&rt);
        let reports = rt.state.c14a.reports;
        rt.set_viewport((480.0, 640.0), 1.0);
        settle(&mut rt);
        assert!(fit > 0.4);
        // 480 points across a 1200-pixel photo at scale 1.
        assert!(near(level(&rt), 0.4), "the narrower frame fits by width");
        assert_eq!(
            rt.state.c14a.reports,
            reports + 1,
            "the new fit is reported"
        );

        rt.wheel_event(WheelEvent::new(0.0, 200.0).at(240.0, 300.0).mods(ctrl()));
        settle(&mut rt);
        let zoomed = level(&rt);
        rt.set_viewport((960.0, 640.0), 1.0);
        settle(&mut rt);
        assert!(near(level(&rt), zoomed));
    }

    #[test]
    fn a_scaled_down_thumbnail_is_never_enlarged() {
        let mut rt = scene("c14a-scale-down-light");
        assert!(near(level(&rt), 1.0));
        rt.set_viewport((1600.0, 1200.0), 1.0);
        settle(&mut rt);
        assert!(near(level(&rt), 1.0));
        rt.set_viewport((128.0, 640.0), 1.0);
        settle(&mut rt);
        assert!(near(level(&rt), 0.5));
    }

    #[test]
    fn pans_stop_at_the_photos_edges() {
        let mut rt = scene("c14a-zoomed-light");
        let r = frame_rect(&rt);
        // Dragged far up and left, the photo stops with its bottom-right corner on the frame's.
        drag(&mut rt, [400.0, 300.0], [-5000.0, -5000.0]);
        let corner = rt.state.c14a.view.pan();
        assert!(near2(
            under(&rt, [r[0] + r[2], r[1] + r[3]]),
            [1200.0, 800.0]
        ));
        // Further that way, by drag and by wheel, does nothing.
        drag(&mut rt, [400.0, 300.0], [-150.0, -100.0]);
        rt.wheel_event(WheelEvent::new(-60.0, -60.0).at(400.0, 300.0));
        settle(&mut rt);
        assert_eq!(rt.state.c14a.view.pan(), corner);
        // Back the other way, the photo follows the pointer, then stops at its top-left.
        drag(&mut rt, [400.0, 300.0], [150.0, 100.0]);
        assert!(near2(
            rt.state.c14a.view.pan(),
            [corner[0] + 150.0, corner[1] + 100.0]
        ));
        drag(&mut rt, [400.0, 300.0], [5000.0, 5000.0]);
        assert!(near2(rt.state.c14a.view.pan(), [-corner[0], -corner[1]]));
        assert!(near2(under(&rt, [r[0], r[1]]), [0.0, 0.0]));
    }

    #[test]
    fn plus_minus_and_zero_step_and_reset() {
        let mut rt = scene("c14a-fit-light");
        let fit = level(&rt);
        focus(&mut rt);
        key(&mut rt, text("+"), Mods::default());
        let up = level(&rt);
        assert!(up > fit && rt.state.c14a.view.steps().steps().contains(&up));
        key(&mut rt, text("="), Mods::default());
        assert!(level(&rt) > up);
        key(&mut rt, text("-"), Mods::default());
        assert!(near(level(&rt), up));
        key(&mut rt, text("0"), Mods::default());
        assert_eq!(rt.state.c14a.view.mode(), Mode::Fit);
        assert!(near(level(&rt), fit));
        assert_eq!(rt.state.c14a.passed, 0, "the viewport took every key");
    }

    #[test]
    fn ctrl_zero_resets_and_is_consumed_even_at_fit() {
        let mut rt = scene("c14a-zoomed-light");
        focus(&mut rt);
        key(&mut rt, Key::Command('0'), ctrl());
        assert_eq!(rt.state.c14a.view.mode(), Mode::Fit);
        assert_eq!(rt.state.c14a.view.pan(), [0.0, 0.0]);
        key(&mut rt, Key::Command('0'), ctrl());
        assert_eq!(
            rt.state.c14a.passed, 0,
            "a reset at fit is still the viewport's"
        );
        // With Alt or Super it isn't a reset, and the app hears it.
        for extra in [
            Mods {
                alt: true,
                ..ctrl()
            },
            Mods {
                logo: true,
                ..ctrl()
            },
        ] {
            key(&mut rt, Key::Command('0'), extra);
        }
        assert_eq!(rt.state.c14a.passed, 2);
        // Ctrl++ is Strata's text size.
        key(&mut rt, Key::Command('='), ctrl());
        assert_eq!(rt.state.c14a.passed, 3);
    }

    #[test]
    fn arrows_pan_only_where_the_photo_overflows() {
        let mut rt = scene("c14a-actual-light");
        focus(&mut rt);
        // 1200 × 800 at 1:1 in a frame 960 wide and shorter than 800: both axes overflow.
        key(&mut rt, Key::Right, Mods::default());
        key(&mut rt, Key::Down, Mods::default());
        let pan = rt.state.c14a.view.pan();
        assert!(
            pan[0] < 0.0 && pan[1] < 0.0,
            "the view moved right and down: {pan:?}"
        );
        assert_eq!(rt.state.c14a.passed, 0);

        // At an edge, an arrow that would move nothing passes on too.
        let left = rt.state.c14a.view.pan()[0];
        for _ in 0..40 {
            key(&mut rt, Key::Right, Mods::default());
        }
        let edge = rt.state.c14a.view.pan()[0];
        assert!(edge < left);
        let passed = rt.state.c14a.passed;
        assert!(passed > 0, "the arrows past the edge passed on");
        key(&mut rt, Key::Right, Mods::default());
        assert_eq!(rt.state.c14a.view.pan()[0], edge);
        assert_eq!(rt.state.c14a.passed, passed + 1);

        let mut rt = scene("c14a-fit-light");
        focus(&mut rt);
        key(&mut rt, Key::Right, Mods::default());
        key(&mut rt, Key::Up, Mods::default());
        assert_eq!(rt.state.c14a.view.pan(), [0.0, 0.0]);
        assert_eq!(rt.state.c14a.passed, 2, "at fit the arrows are the app's");
    }

    #[test]
    fn a_video_frame_takes_no_zoom_or_keys() {
        let mut rt = scene("c14a-stream-light");
        let fit = level(&rt);
        rt.wheel_event(WheelEvent::new(0.0, 40.0).at(480.0, 320.0).mods(ctrl()));
        rt.magnify_event(MagnifyEvent::new(0.5).at(480.0, 320.0));
        settle(&mut rt);
        assert!(near(level(&rt), fit));
        key(&mut rt, text("+"), Mods::default());
        assert!(near(level(&rt), fit));
        assert_eq!(rt.state.c14a.passed, 1, "no focus: the app hears the key");
        assert!(rt.state.c14a.frames > 0, "the frames kept coming");
    }
}
