// SPDX-License-Identifier: MIT

use super::super::*;
use crate::{test_support::gtk_test, ui::theme::ThemeManager};

fn render_text(drawer: &PreviewDrawer) {
    drawer.state.render(crate::services::Preview {
        request_id: crate::services::PreviewRequestId(1),
        entry: crate::model::FileEntry {
            location: crate::model::Location::local("/fixture/wrap.txt"),
            native_name: "wrap.txt".into(),
            thumbnail_path: None,
            display_name: "wrap.txt".into(),
            kind: crate::model::EntryKind::File,
            size: crate::model::MetadataValue::Known(100),
            modified_unix_seconds: crate::model::MetadataValue::Unknown,
            mode: crate::model::MetadataValue::Unknown,
            is_hidden: false,
        },
        content_type: "text/plain".into(),
        content: crate::services::PreviewContent::Text {
            content: "A long line of preview text. ".repeat(100),
            truncated: false,
        },
    });
}

#[test]
fn saved_and_live_wrap_preferences_reach_existing_and_rebuilt_previews() {
    gtk_test(
        "ui::preview::tests::preferences::saved_and_live_wrap_preferences_reach_existing_and_rebuilt_previews",
        || {
            ThemeManager::seed_saved_preferences_for_test();
            let manager = ThemeManager::shared();
            let drawers = [true, false].map(|browser| {
                let drawer = PreviewDrawer::new(Rc::new(super::NoopPreviewProvider), browser);
                render_text(&drawer);
                drawer
            });
            for wrapped in [true, false, true] {
                drawers[0].state.wrap.set_active(wrapped);
                assert_eq!(manager.preview_text_wrap(), wrapped);
                for drawer in &drawers {
                    assert_eq!(drawer.state.wrap.is_active(), wrapped);
                    assert!(drawer.state.wrap.is_visible());
                    assert_eq!(
                        drawer
                            .state
                            .text_view
                            .borrow()
                            .as_ref()
                            .expect("rendered text view")
                            .wrap_mode(),
                        if wrapped {
                            gtk::WrapMode::Word
                        } else {
                            gtk::WrapMode::None
                        },
                    );
                    assert_eq!(
                        drawer
                            .state
                            .text_scroll
                            .borrow()
                            .as_ref()
                            .expect("rendered text scroller")
                            .hscrollbar_policy(),
                        if wrapped {
                            gtk::PolicyType::Never
                        } else {
                            gtk::PolicyType::Automatic
                        },
                    );
                }
                drawers[1].state.clear_content();
                assert!(drawers[1].state.text_view.borrow().is_none());
                assert!(drawers[1].state.text_scroll.borrow().is_none());
                assert!(!drawers[1].state.wrap.is_visible());
                render_text(&drawers[1]);
                assert_eq!(drawers[1].state.wrap.is_active(), wrapped);
                assert_eq!(
                    drawers[1]
                        .state
                        .text_view
                        .borrow()
                        .as_ref()
                        .expect("rebuilt text view")
                        .wrap_mode(),
                    if wrapped {
                        gtk::WrapMode::Word
                    } else {
                        gtk::WrapMode::None
                    },
                );
            }
        },
    );
}

#[test]
fn saved_and_live_audio_preferences_reach_every_open_player() {
    gtk_test(
        "ui::preview::tests::preferences::saved_and_live_audio_preferences_reach_every_open_player",
        || {
            ThemeManager::seed_saved_preferences_for_test();
            let manager = ThemeManager::shared();
            let mut players = Vec::new();
            for browser in [true, false] {
                let drawer = PreviewDrawer::new(
                    Rc::new(crate::adapters::LocalPreviewProvider::new(Rc::new(|| {
                        crate::sandbox::MediaPreviewBackend::Software
                    }))),
                    browser,
                );
                let media = crate::ui::media::tests::player(true, 30_000_000);
                drawer.state.media.replace(Some(media.clone().upcast()));
                drawer.state.append_media_controls(
                    media.upcast_ref(),
                    &manager,
                    &gtk::Box::new(gtk::Orientation::Vertical, 0),
                    &gtk::Button::new(),
                    false,
                );
                media.play();
                crate::ui::media::tests::wait(|| media.timestamp() > 0);
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
