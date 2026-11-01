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
                    .expect("allocate route image")
                    .encode_png()
                    .expect("encode route image"),
                profile: elevation.map(|_| {
                    resvg::tiny_skia::Pixmap::new(
                        800,
                        crate::services::route_preview::PROFILE_HEIGHT,
                    )
                    .expect("allocate elevation image")
                    .encode_png()
                    .expect("encode elevation image")
                }),
                elevation,
                distance_m: 1000.0,
                duration_seconds: None,
                points: 2,
                samples: vec![
                    RouteSample {
                        position: [24.0, 300.0],
                        distance_m: 0.0,
                        elevation_y: elevation
                            .map(|_| crate::services::route_preview::PROFILE_HEIGHT as f32 - 4.0),
                        starts_segment: true,
                    },
                    RouteSample {
                        position: [776.0, 300.0],
                        distance_m: 1000.0,
                        elevation_y: elevation.map(|_| 4.0),
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
            let player = drawer
                .state
                .route
                .borrow()
                .clone()
                .expect("loaded route player");
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
            assert_eq!(
                player
                    .position
                    .get()
                    .expect("position at route end")
                    .distance_m,
                1000.0
            );
            assert!(drawer.media_key(MediaCommand::PlayPause));
            assert_eq!(
                player
                    .position
                    .get()
                    .expect("position after restart")
                    .distance_m,
                0.0
            );
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
            let player = drawer
                .state
                .route
                .borrow()
                .clone()
                .expect("replacement route player");
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
            first.media_key(MediaCommand::Seek(MediaSeek::By(-7_500_000)));
            let player = first
                .state
                .route
                .borrow()
                .clone()
                .expect("loaded route player");
            assert_eq!(
                player
                    .position
                    .get()
                    .expect("position after seeking")
                    .distance_m,
                500.0
            );
            player.hover(PlotKind::Elevation, 10.0, 20.0);
            assert!(labels(first.widget().upcast_ref()).contains(&"156 ft".into()));
            first.media_key(MediaCommand::PlayPause);
            assert!(labels(first.widget().upcast_ref()).contains(&"156 ft".into()));
            manager.set_unit_system(UnitSystem::Metric);
            assert!(player.play.is_active(), "unit changes preserve playback");
            assert_eq!(
                player
                    .position
                    .get()
                    .expect("position after changing units")
                    .distance_m,
                500.0
            );
            let text = labels(first.widget().upcast_ref());
            assert!(text.contains(&"0.5 km".into()));
            assert!(text.contains(&"1.0 km".into()));
            assert!(text.contains(&"-5–100 m".into()));
            assert!(labels(second.widget().upcast_ref()).contains(&"0–30 m".into()));
            player.hover(PlotKind::Elevation, 11.0, 20.0);
            assert!(labels(first.widget().upcast_ref()).contains(&"48 m".into()));
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

#[test]
fn suspended_route_reloads_with_the_latest_palette_on_resume() {
    crate::test_support::gtk_test(
        "ui::preview::tests::route::suspended_route_reloads_with_the_latest_palette_on_resume",
        || {
            let provider = Rc::new(Provider::default());
            let drawer = PreviewDrawer::new(provider.clone(), false);
            drawer.show(entry("route.gpx"), None);
            ready(&provider, 0);
            let split = gtk::Paned::new(gtk::Orientation::Horizontal);
            split.set_start_child(Some(&gtk::Box::new(gtk::Orientation::Vertical, 0)));
            split.set_end_child(Some(&drawer.widget()));
            drawer.state.sync_split(&split);
            assert!(drawer.state.sizing.is_suspended());
            let manager = crate::ui::theme::ThemeManager::shared();
            let mut tokens = manager.appearance_tokens();
            let old_palette = manager.active_model_palette();
            tokens.accent = if old_palette.accent == 0xff0000 {
                "#00ff00"
            } else {
                "#ff0000"
            }
            .into();
            manager.preview(&tokens);
            assert_eq!(
                provider.0.borrow().len(),
                1,
                "hidden previews defer rendering"
            );
            let window = gtk::Window::builder()
                .default_width(1200)
                .default_height(760)
                .child(&split)
                .build();
            window.present();
            while glib::MainContext::default().iteration(false) {}
            drawer.state.sync_split(&split);
            let pending = provider.0.borrow();
            assert_eq!(
                pending.len(),
                2,
                "restoration requests a fresh route render"
            );
            assert_eq!(
                pending[1].request.model_palette,
                manager.active_model_palette()
            );
            assert_ne!(pending[1].request.model_palette, old_palette);
            window.destroy();
        },
    );
}

#[test]
fn route_caption_counts_supported_files_in_listing_order() {
    crate::test_support::gtk_test(
        "ui::preview::tests::route::route_caption_counts_supported_files_in_listing_order",
        || {
            let (_directory, view) =
                super::video::sorted_listing(&["a.gpx", "b.fit", "c.kml", "notes.txt"]);
            let browser = view.browser();
            let provider = Rc::new(Provider::default());
            let drawer = PreviewDrawer::new(provider.clone(), false);
            drawer.state.keyboard_view.replace(Some(view.downgrade()));
            for index in 0..3 {
                drawer.show(browser.entry_at(0, index).expect("route entry"), Some(0));
                ready(&provider, index);
                let text = labels(drawer.widget().upcast_ref());
                assert!(text.contains(&format!("{} of 3 in folder", index + 1)));
            }
        },
    );
}
