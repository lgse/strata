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
            image_dimensions: MetadataValue::Unknown,
            child_count: MetadataValue::Unknown,
            duration_seconds: MetadataValue::Unknown,
        },
        content_type: "text/plain".into(),
        content: crate::services::PreviewContent::Text {
            content: "A long line of preview text. ".repeat(20),
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
fn document_defaults_apply_before_settings_and_after_reopening_in_two_windows() {
    gtk_test(
        "ui::preview::tests::preferences::document_defaults_apply_before_settings_and_after_reopening_in_two_windows",
        || {
            ThemeManager::seed_saved_preferences_for_test();
            let manager = ThemeManager::shared();
            assert!(!manager.render_documents_by_default());
            let fixture = tempfile::tempdir().expect("document fixtures");
            std::fs::write(
                fixture.path().join("notes.md"),
                "# Heading\n\nDocument text.",
            )
            .expect("Markdown fixture");
            std::fs::write(
                fixture.path().join("page.html"),
                "<h1>Heading</h1><p>Document text.</p>",
            )
            .expect("HTML fixture");
            std::fs::write(fixture.path().join("data.csv"), "name,value\nalpha,1\n")
                .expect("CSV fixture");
            std::fs::write(fixture.path().join("data.tsv"), "name\tvalue\nalpha\t1\n")
                .expect("TSV fixture");
            std::fs::write(fixture.path().join("broken.html"), "<p>text</span>")
                .expect("malformed fixture");
            let browser = Browser::new(Rc::new(crate::adapters::LocalFileSource));
            browser.navigate(crate::model::Location::local(fixture.path()));
            crate::ui::media::tests::wait(|| {
                browser.column_snapshot(0).is_some_and(|s| !s.loading)
            });
            let entries = [
                "notes.md",
                "page.html",
                "broken.html",
                "data.csv",
                "data.tsv",
            ]
            .map(|name| {
                (0..5)
                    .filter_map(|index| browser.entry_at(0, index))
                    .find(|entry| entry.display_name == name)
                    .expect("loaded document entry")
            });
            let drawers = [true, false].map(|external_open| {
                PreviewDrawer::new(
                    Rc::new(crate::adapters::LocalPreviewProvider::new(Rc::new(|| {
                        crate::sandbox::MediaPreviewBackend::Software
                    }))),
                    external_open,
                )
            });
            let windows = drawers.each_ref().map(|drawer| {
                let window = gtk::Window::builder().child(&drawer.widget()).build();
                window.present();
                window
            });
            let visible_view = |drawer: &PreviewDrawer, name: &str| {
                drawer
                    .state
                    .document_preview
                    .borrow()
                    .as_ref()
                    .is_some_and(|preview| {
                        preview.stack.visible_child_name().as_deref() == Some(name)
                    })
            };
            for pair in [&entries[..2], &entries[3..]] {
                manager.set_render_documents_by_default(false);
                for (drawer, entry) in drawers.iter().zip(pair) {
                    drawer.show(entry.clone(), Some(0));
                    crate::ui::media::tests::wait(|| visible_view(drawer, "source"));
                    assert!(
                        drawer
                            .state
                            .document_preview
                            .borrow()
                            .as_ref()
                            .expect("source document awaiting render")
                            .render_pending
                    );
                }
                drawers[0].state.document_view_button.emit_clicked();
                crate::ui::media::tests::wait(|| visible_view(&drawers[0], "rendered"));
                assert!(visible_view(&drawers[1], "source"));
                assert!(!manager.render_documents_by_default());
                manager.set_render_documents_by_default(true);
                assert!(visible_view(&drawers[1], "source"));
                for (drawer, entry) in drawers.iter().zip(pair) {
                    drawer.close();
                    drawer.show(entry.clone(), Some(0));
                    crate::ui::media::tests::wait(|| visible_view(drawer, "rendered"));
                    drawer.state.document_view_button.emit_clicked();
                    assert!(visible_view(drawer, "source"));
                    assert!(manager.render_documents_by_default());
                }
            }
            drawers[0].show(entries[2].clone(), Some(0));
            crate::ui::media::tests::wait(|| {
                visible_view(&drawers[0], "source")
                    && !drawers[0].state.document_view_button.is_visible()
            });
            assert!(
                !drawers[0]
                    .state
                    .document_preview
                    .borrow()
                    .as_ref()
                    .expect("source fallback document")
                    .render_pending
            );
            for window in windows {
                window.close();
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
