//! The trace cases are Strata's (`src/ui/preview/route/position/tests.rs`), ported; the rest
//! check playback and units on the state.

use super::trace::{at_distance, nearest, profile_distance, profile_x};
use super::{RouteCommand, RouteData, RouteLabels, RouteSample, RouteState, Seek, Units};

fn sample(
    x: f32,
    y: f32,
    distance_m: f64,
    elevation_y: Option<f32>,
    starts_segment: bool,
) -> RouteSample {
    RouteSample {
        position: [x, y],
        distance_m,
        elevation_y,
        starts_segment,
    }
}

#[test]
fn hovering_a_route_segment_and_seeking_distance_choose_the_same_elevation() {
    let samples = [
        sample(0.0, 0.0, 0.0, Some(120.0), true),
        sample(100.0, 100.0, 1000.0, Some(20.0), false),
    ];
    let hovered = nearest(&samples, [25.0, 25.0], None, 0.0).expect("nearest route position");
    assert_eq!(hovered.position, [25.0, 25.0]);
    assert_eq!(hovered.distance_m, 250.0);
    assert_eq!(hovered.elevation_y, Some(95.0));
    assert_eq!(at_distance(&samples, 250.0), Some(hovered));
    assert_eq!(
        at_distance(&samples, 5000.0).expect("end").distance_m,
        1000.0
    );
}

#[test]
fn hovering_does_not_invent_a_connection_across_route_or_elevation_gaps() {
    let samples = [
        sample(0.0, 0.0, 0.0, Some(120.0), true),
        sample(100.0, 0.0, 100.0, None, false),
        sample(500.0, 0.0, 100.0, Some(20.0), true),
        sample(600.0, 0.0, 200.0, Some(40.0), false),
    ];
    let gap = nearest(&samples, [300.0, 0.0], None, 0.0).expect("nearest segment endpoint");
    assert!(gap.position == [100.0, 0.0] || gap.position == [500.0, 0.0]);
    assert_eq!(
        at_distance(&samples, 50.0).expect("first").elevation_y,
        None
    );
    let second = at_distance(&samples, 150.0).expect("second segment");
    assert_eq!(second.position, [550.0, 0.0]);
    assert_eq!(second.elevation_y, Some(30.0));
}

#[test]
fn stationary_and_isolated_points_can_be_hovered_without_dividing_by_zero() {
    let samples = [
        sample(10.0, 20.0, 0.0, None, true),
        sample(10.0, 20.0, 0.0, None, false),
        sample(400.0, 100.0, 0.0, None, true),
    ];
    assert_eq!(
        nearest(&samples, [399.0, 100.0], None, 0.0)
            .expect("isolated point")
            .position,
        [400.0, 100.0]
    );
    assert_eq!(at_distance(&samples, 0.0), Some(samples[0]));
    assert!(at_distance(&[], 0.0).is_none());
}

#[test]
fn sub_metre_profile_hover_selects_the_same_route_position_as_the_drawing() {
    let samples = [
        sample(0.0, 0.0, 0.0, Some(120.0), true),
        sample(100.0, 100.0, 0.5, Some(20.0), false),
    ];
    let endpoint = profile_x(0.5, 0.5, 800.0);
    let hovered = at_distance(&samples, profile_distance(endpoint, 0.5, 800.0)).expect("end");
    assert_eq!(hovered.position, [100.0, 100.0]);
    assert_eq!(hovered.elevation_y, Some(20.0));
    let mid = at_distance(&samples, profile_distance(endpoint / 2.0 + 1.0, 0.5, 800.0));
    assert_eq!(mid.expect("middle").distance_m, 0.25);
}

/// `preview/preview-panel/routes`: "Where route passes overlap, hovering keeps the pass
/// nearest the current distance among lines within 2 px of the closest."
#[test]
fn hover_stays_on_the_same_nearby_pass_without_snapping_to_recorded_points() {
    let samples = [
        sample(0.0, 0.0, 0.0, Some(10.0), true),
        sample(100.0, 0.0, 100.0, Some(20.0), false),
        sample(100.0, 1.0, 100.0, Some(30.0), true),
        sample(0.0, 1.0, 200.0, Some(40.0), false),
    ];
    let mut previous = 170.0;
    for x in [25.0, 24.5, 24.0] {
        let hovered = nearest(&samples, [x, 0.4], Some(previous), 2.0).expect("return pass");
        assert_eq!(hovered.position, [x as f32, 1.0]);
        assert_eq!(hovered.distance_m, 200.0 - x);
        assert_eq!(at_distance(&samples, hovered.distance_m), Some(hovered));
        previous = hovered.distance_m;
    }
    let switched = nearest(&samples, [25.0, 0.0], Some(previous), 0.5).expect("outward pass");
    assert_eq!(switched.distance_m, 25.0);
    assert_eq!(switched.position, [25.0, 0.0]);
}

/// A straight 12 km route along the picture's middle, heights rising from 10 to 30 m.
fn straight() -> RouteState {
    let samples = (0..=12)
        .map(|i| {
            let t = i as f32 / 12.0;
            sample(
                40.0 + 720.0 * t,
                300.0,
                f64::from(i) * 1000.0,
                Some(66.0 - 62.0 * t),
                i == 0,
            )
        })
        .collect();
    RouteState::new(
        RouteData {
            name: "ridge.gpx".into(),
            format: "GPX".into(),
            position: None,
            distance_m: 12_000.0,
            duration_s: Some(37_070),
            elevation: Some([10.0, 30.0]),
            samples,
            image_px: [800.0, 600.0],
            profile_px: [800.0, 70.0],
        },
        RouteLabels::default(),
    )
}

#[test]
fn seeks_pause_and_move_a_third_and_play_at_the_end_restarts() {
    let mut st = straight();
    st.command(RouteCommand::PlayPause);
    assert!(st.playing());
    st.command(RouteCommand::Seek(Seek::By(5.0)));
    assert!(!st.playing(), "a seek pauses");
    assert_eq!(st.distance_text(), "4.0 km");
    st.command(RouteCommand::Seek(Seek::End));
    assert_eq!(st.distance_text(), "12.0 km");
    st.toggle();
    assert!(st.playing());
    assert_eq!(st.distance_text(), "0.0 km", "play at the end starts over");
}

#[test]
fn units_change_the_words_and_keep_the_marker() {
    let mut st = straight();
    st.command(RouteCommand::Seek(Seek::By(7.5)));
    st.toggle();
    st.set_units(Units::Imperial);
    assert!(st.playing());
    assert_eq!(st.distance_text(), "3.7 mi");
    assert_eq!(st.total_text(), "7.5 mi");
    assert_eq!(st.range_text().as_deref(), Some("33–98 ft"));
    assert_eq!(st.height_text().as_deref(), Some("66 ft"));
}

#[test]
fn the_words_strata_shows() {
    let st = straight();
    assert_eq!(st.duration_text().as_deref(), Some("10:17:50"));
    assert_eq!(st.range_text().as_deref(), Some("10–30 m"));
    assert_eq!(st.height_text().as_deref(), Some("10 m"));
    let labels = RouteLabels::default();
    assert_eq!((labels.range)([-5.0, 20.0], Units::Metric), "-5–20 m");
    assert_eq!((labels.duration)(65), "1:05");
}

#[test]
fn a_route_of_zero_length_takes_no_play() {
    let mut st = RouteState::new(
        RouteData {
            name: "summit.gpx".into(),
            format: "GPX".into(),
            samples: vec![sample(400.0, 300.0, 0.0, None, true)],
            image_px: [800.0, 600.0],
            profile_px: [800.0, 70.0],
            ..RouteData::default()
        },
        RouteLabels::default(),
    );
    assert!(!st.playable());
    assert!(!st.command(RouteCommand::PlayPause));
    st.toggle();
    assert!(!st.playing());
}

// The element, in a runtime.

mod element {
    use std::cell::Cell;

    use rune::{Cx, El, Env, Runtime, Tx};
    use strata_kit::look::Look;

    use super::super::{RouteCommand, RouteData, RouteLabels, RouteState, RouteView, Seek};
    use super::sample;

    const DT: f64 = 1.0 / 60.0;

    thread_local! {
        static VIEWS: Cell<u32> = const { Cell::new(0) };
    }

    struct App {
        route: RouteState,
        look: Look,
    }

    fn view(s: &App, cx: &mut Cx<App>) -> El {
        VIEWS.with(|v| v.set(v.get() + 1));
        RouteView::new(
            &s.route,
            |s: &mut App| &mut s.route,
            |s: &mut App| &mut s.route.viewport,
        )
        .build(&s.look, cx)
    }

    fn poll(s: &mut App, t: f64, tx: &mut Tx) -> bool {
        s.route.tick(t, tx)
    }

    /// A straight route `total` metres long, with heights.
    fn runtime(total: f64) -> Runtime<App> {
        let samples = (0..=100)
            .map(|i| {
                let t = i as f32 / 100.0;
                sample(
                    40.0 + 720.0 * t,
                    300.0,
                    total * f64::from(t),
                    Some(60.0 - 50.0 * t),
                    i == 0,
                )
            })
            .collect();
        let data = RouteData {
            name: "traverse.gpx".into(),
            format: "GPX".into(),
            distance_m: total,
            elevation: Some([10.0, 30.0]),
            samples,
            image_px: [800.0, 600.0],
            profile_px: [800.0, 70.0],
            ..RouteData::default()
        };
        let app = App {
            route: RouteState::new(data, RouteLabels::default()),
            look: Look::fallback(false),
        };
        let mut rt = Runtime::new(app, view, (420.0, 640.0), Env::new(false, 1.0));
        rt.poll = Some(poll);
        frames(&mut rt, 30);
        rt
    }

    fn frames(rt: &mut Runtime<App>, n: usize) {
        for _ in 0..n {
            rt.frame(DT);
        }
    }

    fn views() -> u32 {
        VIEWS.with(Cell::get)
    }

    /// The text the element keyed `key` shows.
    fn shown(rt: &Runtime<App>, key: &str) -> String {
        rt.elements()
            .into_iter()
            .find(|e| e.key.as_deref() == Some(key))
            .and_then(|e| e.text)
            .unwrap_or_else(|| panic!("no text keyed {key}"))
    }

    /// A 1,000 km route's tenths change faster than frames: playback commits the text about ten
    /// times a second while the markers move every frame, and a pause shows the exact place.
    #[test]
    fn long_playback_commits_its_text_at_most_ten_times_a_second() {
        let mut rt = runtime(1_000_000.0);
        rt.update(|s, _| s.route.toggle());
        frames(&mut rt, 2);
        let before = views();
        frames(&mut rt, 180);
        let built = views() - before;
        assert!(built <= 33, "{built} view builds in 3 s of playback");
        assert!(built >= 20, "the text still follows: {built} builds in 3 s");
        rt.update(|s, _| {
            s.route.command(RouteCommand::PlayPause);
        });
        frames(&mut rt, 2);
        assert!(!rt.state.route.playing());
        assert_eq!(shown(&rt, "route-elapsed"), rt.state.route.distance_text());
        rt.update(|s, _| {
            s.route.command(RouteCommand::Seek(Seek::By(1.0)));
        });
        frames(&mut rt, 2);
        assert_eq!(shown(&rt, "route-elapsed"), rt.state.route.distance_text());
    }

    /// The end of the route shows exactly, whenever the last text commit was.
    #[test]
    fn playback_that_reaches_the_end_shows_the_total() {
        let mut rt = runtime(1_000_000.0);
        rt.update(|s, _| s.route.toggle());
        frames(&mut rt, 16 * 60);
        assert!(!rt.state.route.playing());
        assert_eq!(shown(&rt, "route-elapsed"), "1000.0 km");
    }
}
