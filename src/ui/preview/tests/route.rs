// SPDX-License-Identifier: MIT

use super::*;
use crate::services::route_preview::{RoutePreview, RouteSample};
use crate::ui::preview::{keyboard::MediaSeek, route::plot::PlotKind};

fn ready(provider: &Provider, index: usize) {
    ready_with_elevation(provider, index, None);
}

fn ready_with_elevation(provider: &Provider, index: usize, elevation: Option<[f64; 2]>) {
    let pending = provider.0.borrow();
    let pending = &pending[index];
    (pending.emit)(PreviewEvent::Ready(Preview {
        request_id: pending.request.id,
        entry: pending.request.entry.clone(),
        content_type: "application/gpx+xml".into(),
        content: PreviewContent::Route {
            preview: RoutePreview {
                png: resvg::tiny_skia::Pixmap::new(800, 600)
                    .unwrap()
                    .encode_png()
                    .unwrap(),
                profile: elevation.map(|_| {
                    resvg::tiny_skia::Pixmap::new(800, 140)
                        .unwrap()
                        .encode_png()
                        .unwrap()
                }),
                elevation,
                distance_m: 1000.0,
                points: 2,
                samples: vec![
                    RouteSample {
                        position: [24.0, 300.0],
                        distance_m: 0.0,
                        elevation_y: None,
                        starts_segment: true,
                    },
                    RouteSample {
                        position: [776.0, 300.0],
                        distance_m: 1000.0,
                        elevation_y: None,
                        starts_segment: false,
                    },
                ],
            },
        },
    }));
}

#[test]
fn route_media_commands_restart_at_the_end_and_stop_on_replacement_or_close() {
    crate::test_support::gtk_test(
        "ui::preview::tests::route::route_media_commands_restart_at_the_end_and_stop_on_replacement_or_close",
        || {
            let provider = Rc::new(Provider::default());
            let drawer = PreviewDrawer::new(provider.clone(), false);
            drawer.show(entry("first.gpx"), None);
            ready(&provider, 0);
            let player = drawer.state.route.borrow().clone().unwrap();
            assert!(
                drawer.has_video(),
                "route participates in the media shortcut dispatcher"
            );
            assert!(drawer.media_key(MediaCommand::PlayPause));
            assert!(player.play.is_active());
            assert!(!drawer.media_key(MediaCommand::Mute));
            player.hover(PlotKind::Route, 10.0, 20.0);
            assert!(!player.play.is_active(), "pointer movement pauses playback");
            assert!(drawer.media_key(MediaCommand::PlayPause));
            player.hover(PlotKind::Route, 10.0, 20.0);
            assert!(
                player.play.is_active(),
                "duplicate motion must not undo explicit play"
            );
            player.hover(PlotKind::Route, 11.0, 20.0);
            assert!(
                !player.play.is_active(),
                "a new pointer position pauses again"
            );
            assert!(drawer.media_key(MediaCommand::Seek(MediaSeek::End)));
            assert!(!player.play.is_active());
            assert_eq!(player.position.get().unwrap().distance_m, 1000.0);
            assert!(drawer.media_key(MediaCommand::PlayPause));
            assert_eq!(player.position.get().unwrap().distance_m, 0.0);
            assert!(player.play.is_active());
            drawer.show(entry("second.gpx"), None);
            ready(&provider, 1);
            assert!(
                !player.play.is_active(),
                "changing files pauses the old route"
            );
            let weak = Rc::downgrade(&player);
            drop(player);
            assert!(
                weak.upgrade().is_none(),
                "tick and widget handlers do not retain the route"
            );
            let player = drawer.state.route.borrow().clone().unwrap();
            assert!(drawer.media_key(MediaCommand::PlayPause));
            drawer.close();
            assert!(!player.play.is_active());
            assert!(!drawer.media_key(MediaCommand::PlayPause));
        },
    );
}

#[test]
fn route_units_load_before_settings_and_update_across_drawers_and_files() {
    crate::test_support::gtk_test(
        "ui::preview::tests::route::route_units_load_before_settings_and_update_across_drawers_and_files",
        || {
            use crate::ui::preferences::{PreferenceManager, UnitSystem};
            crate::ui::preferences::fixtures::seed_saved_preferences_for_test();
            let manager = PreferenceManager::shared();
            let provider = Rc::new(Provider::default());
            let first = PreviewDrawer::new(provider.clone(), false);
            let second = PreviewDrawer::new(provider.clone(), false);
            first.show(entry("first.gpx"), None);
            ready_with_elevation(&provider, 0, Some([-5.0, 100.0]));
            second.show(entry("second.fit"), None);
            ready_with_elevation(&provider, 1, Some([0.0, 30.48]));
            assert!(labels(first.widget().upcast_ref()).contains(&"-16–328 ft".into()));
            assert!(labels(second.widget().upcast_ref()).contains(&"0–100 ft".into()));
            assert!(labels(first.widget().upcast_ref()).contains(&"0.6 mi".into()));
            first.media_key(MediaCommand::Seek(MediaSeek::End));
            first.media_key(MediaCommand::Seek(MediaSeek::By(-15_000_000)));
            let player = first.state.route.borrow().clone().unwrap();
            assert_eq!(player.position.get().unwrap().distance_m, 500.0);
            first.media_key(MediaCommand::PlayPause);
            manager.set_unit_system(UnitSystem::Metric);
            assert!(player.play.is_active(), "unit changes preserve playback");
            assert_eq!(player.position.get().unwrap().distance_m, 500.0);
            let text = labels(first.widget().upcast_ref());
            assert!(text.contains(&"0.5 km".into()));
            assert!(text.contains(&"1.0 km".into()));
            assert!(text.contains(&"-5–100 m".into()));
            assert!(labels(second.widget().upcast_ref()).contains(&"0–30 m".into()));
            first.show(entry("third.kml"), None);
            ready_with_elevation(&provider, 2, Some([0.0, 100.0]));
            assert!(labels(first.widget().upcast_ref()).contains(&"0–100 m".into()));
            manager.set_unit_system(UnitSystem::Imperial);
            assert!(labels(first.widget().upcast_ref()).contains(&"0–328 ft".into()));
            assert!(labels(second.widget().upcast_ref()).contains(&"0.6 mi".into()));
        },
    );
}

fn labels(widget: &gtk::Widget) -> Vec<String> {
    let mut result = Vec::new();
    if let Some(label) = widget.downcast_ref::<gtk::Label>() {
        result.push(label.text().to_string());
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        result.extend(labels(&widget));
        child = widget.next_sibling();
    }
    result
}
