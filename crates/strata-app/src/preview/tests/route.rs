// SPDX-License-Identifier: MIT

//! The route preview's host (`preview::route`): the route shows once decoded, with its place
//! among the folder's routes and its duration; the media key plays and pauses it; and the
//! view follows the saved units.

use rune::{Key, Mods, Tx};
use strata_components::viewers::route::{RouteState, Units};
use strata_core::model::Location;
use strata_desktop::settings::UnitSystem;
use strata_fs::services::{
    PreviewContent,
    route_preview::{RoutePreview, RouteSample},
};

use super::{
    MODULE, Viewer, disable_auto_preview, focus_view, press,
    provider::{Recorded, install, listed_with, png},
    select, space,
};
use crate::{
    App,
    host::offscreen::Offscreen,
    shell,
    test_support::{PATIENCE, hosted},
};

fn state(host: &Offscreen) -> Option<&RouteState> {
    match &host.app().preview.viewer {
        Viewer::Route(route) => Some(&route.state),
        _ => None,
    }
}

fn shows(host: &mut Offscreen, done: impl Fn(&RouteState) -> bool, what: &str) {
    let shown = host.run_until(PATIENCE, |host| state(host).is_some_and(&done));
    assert!(shown, "the route view: {what}; it shows {:?}", state(host));
}

fn sample(x: f32, distance_m: f64, elevation_y: f32) -> RouteSample {
    RouteSample {
        position: [x, 30.0],
        distance_m,
        elevation_y: Some(elevation_y),
        starts_segment: distance_m == 0.0,
    }
}

fn route() -> PreviewContent {
    PreviewContent::Route {
        preview: RoutePreview {
            png: png(80, 60),
            profile: Some(png(80, 7)),
            distance_m: 600.0,
            duration_seconds: Some(37_070),
            elevation: Some([10.0, 30.0]),
            points: 2,
            samples: vec![sample(10.0, 0.0, 5.0), sample(70.0, 600.0, 2.0)],
        },
    }
}

fn ctrl_alt() -> Mods {
    Mods {
        ctrl: true,
        alt: true,
        ..Mods::default()
    }
}

#[test]
fn a_route_plays_on_the_media_key_and_follows_the_saved_units() {
    hosted(
        MODULE,
        "route::a_route_plays_on_the_media_key_and_follows_the_saved_units",
        |mut host| {
            let provider = Recorded::default();
            install(&mut host, &provider);
            let files = [("route.gpx", "<gpx/>"), ("other.kml", "<kml/>")];
            let folder = listed_with(&mut host, &files);
            disable_auto_preview(&mut host);
            select(&mut host, &Location::local(folder.path().join("route.gpx")));
            focus_view(&mut host);
            space(&mut host);
            provider.wait_for(&mut host, 0);
            provider.answer(0, route());
            shows(
                &mut host,
                |s| s.total_text() == "0.6 km",
                "the decoded route",
            );
            let place = state(&host).and_then(|s| s.data().position);
            assert_eq!(
                place.map(|p| (p.index, p.count)),
                Some((2, 2)),
                "2 of 2 routes"
            );
            let duration = state(&host).and_then(RouteState::duration_text);
            assert_eq!(duration.as_deref(), Some("10:17:50"));

            let space = || Key::Text(" ".to_owned());
            press(&mut host, space(), ctrl_alt());
            shows(&mut host, RouteState::playing, "playing on Ctrl+Alt+Space");
            press(&mut host, space(), ctrl_alt());
            shows(&mut host, |s| !s.playing(), "paused on Ctrl+Alt+Space");

            host.runtime().update(|app: &mut App, _: &mut Tx| {
                shell::saved(app).change(|settings| settings.unit_system = UnitSystem::Imperial);
            });
            let imperial = |s: &RouteState| s.units() == Units::Imperial;
            shows(&mut host, imperial, "the saved units");
            assert_eq!(
                state(&host).map(RouteState::total_text).as_deref(),
                Some("0.4 mi")
            );
        },
    );
}
