// SPDX-License-Identifier: MIT

use super::super::*;
use crate::{test_support::gtk_test, ui::theme::ThemeManager};

#[test]
fn saved_and_live_audio_preferences_reach_every_open_player() {
    gtk_test(
        "ui::preview::tests::preferences::saved_and_live_audio_preferences_reach_every_open_player",
        || {
            ThemeManager::seed_saved_preferences_for_test();
            let manager = ThemeManager::shared();
            let mut players = Vec::new();
            for _ in 0..2 {
                let drawer = PreviewDrawer::new(
                    Rc::new(crate::adapters::LocalPreviewProvider::new(Rc::new(|| {
                        crate::sandbox::MediaPreviewBackend::Software
                    }))),
                    false,
                );
                let media = gtk::MediaFile::new();
                drawer.state.append_media_controls(
                    &media,
                    &manager,
                    &gtk::Box::new(gtk::Orientation::Vertical, 0).upcast(),
                    &gtk::Button::new(),
                    false,
                );
                let slider = drawer
                    .state
                    .media_volume_slider
                    .borrow()
                    .clone()
                    .expect("volume slider");
                assert!(media.is_muted());
                assert_eq!(media.volume(), 0.35);
                assert_eq!(slider.value(), 0.0);
                players.push((drawer, media, slider));
            }
            manager.set_preview_muted(false);
            for (_, media, slider) in &players {
                assert!(!media.is_muted());
                assert_eq!(slider.value(), 0.35);
            }
            players[0].2.set_value(0.7);
            for (_, media, slider) in &players {
                assert_eq!(media.volume(), 0.7);
                assert_eq!(slider.value(), 0.7);
            }
            players[1].2.set_value(0.0);
            for (_, media, slider) in &players {
                assert!(media.is_muted());
                assert_eq!(slider.value(), 0.0);
            }
            assert_eq!(manager.preview_volume(), 0.7);
            let toggle = players[0]
                .0
                .state
                .media_toggle_mute
                .borrow()
                .clone()
                .expect("mute action");
            toggle();
            for (_, media, slider) in &players {
                assert!(!media.is_muted());
                assert_eq!(media.volume(), 0.7);
                assert_eq!(slider.value(), 0.7);
            }
        },
    );
}
