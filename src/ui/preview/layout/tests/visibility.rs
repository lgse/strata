// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn closing_preserves_column_positions_without_locking_horizontal_scrolling() {
    crate::test_support::gtk_test(
        "ui::preview::layout::tests::visibility::closing_preserves_column_positions_without_locking_horizontal_scrolling",
        || {
            gtk::Settings::default()
                .expect("GTK settings")
                .set_gtk_enable_animations(true);
            let preferences = ThemeManager::shared();
            preferences.set_browser_mode(BrowserMode::Columns);
            for (chooser, reduced_motion) in [(false, true), (true, true), (false, false)] {
                preferences.set_reduce_motion(reduced_motion);
                let fixture = Fixture::new(chooser);
                fixture.resize(1200);
                fixture.enter_children();
                wait_until(|| find(&fixture.browser.widget(), "column-entering").is_none());
                fixture.preview.show(entry("first.png"));
                fixture.wait_adjacent();
                wait_until(|| !fixture.preview.state.animating.get());
                let last = fixture.last_column();
                let x = last
                    .compute_bounds(&fixture.split)
                    .expect("last column bounds")
                    .x();
                let offset = fixture.adjustment().value();
                let width = fixture.browser.widget().width();
                assert!(offset > 0.0);
                fixture.preview.close();
                wait_until(|| fixture.browser.widget().width() > width);
                fixture.settle();
                assert!(
                    (fixture.adjustment().value() - offset).abs() <= 1.0,
                    "chooser={chooser}, reduced={reduced_motion}, offset={offset} -> {}, page={}, upper={}, gap={}",
                    fixture.adjustment().value(),
                    fixture.adjustment().page_size(),
                    fixture.adjustment().upper(),
                    fixture.columns().margin_end()
                );
                assert!(
                    (last
                        .compute_bounds(&fixture.split)
                        .expect("closed bounds")
                        .x()
                        - x)
                        .abs()
                        <= 1.0,
                    "chooser={chooser}, reduced={reduced_motion}, column x={x} -> {}",
                    last.compute_bounds(&fixture.split)
                        .expect("closed bounds")
                        .x()
                );

                fixture.adjustment().set_value(0.0);
                fixture.settle();
                assert_eq!(fixture.adjustment().value(), 0.0);
                assert!(
                    last.compute_bounds(&fixture.split)
                        .expect("scrolled bounds")
                        .x()
                        > x
                );
                fixture.preview.show(entry("second.png"));
                fixture.wait_adjacent();
                fixture.close();
            }
        },
    );
}

fn assert_last_column_visible(fixture: &Fixture) {
    let column = fixture
        .last_column()
        .compute_bounds(&fixture.split)
        .expect("last column");
    let browser = fixture
        .browser
        .widget()
        .compute_bounds(&fixture.split)
        .expect("browser viewport");
    assert!(
        column.x() >= browser.x() - 1.0,
        "{column:?} outside {browser:?}"
    );
    assert!(column.x() + column.width() <= browser.x() + browser.width() + 1.0);
}

#[test]
fn last_column_wins_over_preferred_preview_width_and_hidden_requests_resume_once() {
    crate::test_support::gtk_test(
        "ui::preview::layout::tests::visibility::last_column_wins_over_preferred_preview_width_and_hidden_requests_resume_once",
        || {
            let preferences = ThemeManager::shared();
            preferences.set_browser_mode(BrowserMode::Columns);
            preferences.set_reduce_motion(true);
            for chooser in [false, true] {
                let fixture = Fixture::new(chooser);
                fixture.enter_children();
                fixture.resize(760);
                fixture.preview.show(entry("first.png"));
                fixture.settle();
                assert!(fixture.preview.is_open());
                assert!(!fixture.preview.widget().is_visible());
                assert!(fixture.requests.borrow().is_empty());

                fixture.resize(1400);
                wait_until(|| {
                    fixture.preview.widget().is_visible() && fixture.requests.borrow().len() == 1
                });
                fixture.preview.state.resize_preview(
                    &fixture.split,
                    fixture.split.width() - separator_width(&fixture.split) - 700,
                );
                wait_until(|| fixture.preview.widget().width() == 700);
                fixture.resize(1000);
                fixture.wait_adjacent();
                assert!(fixture.preview.widget().width() < COLUMN_WIDTH * MIN_COLUMN_MULTIPLIER);
                assert_last_column_visible(&fixture);
                fixture.last_column().set_width_request(420);
                wait_until(|| fixture.last_column().width() >= 420);
                fixture.wait_adjacent();
                assert_last_column_visible(&fixture);

                fixture.resize(780);
                wait_until(|| fixture.preview.state.sizing.is_suspended());
                fixture.settle();
                assert!(!fixture.preview.widget().is_visible());
                assert_last_column_visible(&fixture);
                fixture.preview.show(entry("latest.png"));
                fixture.settle();
                assert_eq!(
                    fixture.requests.borrow().len(),
                    1,
                    "hidden selections must not load"
                );
                fixture.resize(1400);
                wait_until(|| {
                    fixture.preview.widget().is_visible() && fixture.preview.widget().width() == 700
                });
                fixture.settle();
                assert_eq!(fixture.requests.borrow().len(), 2);
                assert_eq!(
                    fixture
                        .requests
                        .borrow()
                        .last()
                        .expect("restored request")
                        .entry
                        .display_name,
                    "latest.png"
                );
                assert_last_column_visible(&fixture);

                fixture.resize(780);
                wait_until(|| fixture.preview.state.sizing.is_suspended());
                fixture.preview.close();
                fixture.resize(1400);
                fixture.settle();
                assert!(!fixture.preview.is_open());
                assert!(!fixture.preview.widget().is_visible());
                assert_eq!(fixture.requests.borrow().len(), 2);
                fixture.close();
            }
        },
    );
}

#[test]
fn a_hidden_media_preview_pauses_and_restores_only_the_same_players_playing_state() {
    crate::test_support::gtk_test(
        "ui::preview::layout::tests::visibility::a_hidden_media_preview_pauses_and_restores_only_the_same_players_playing_state",
        || {
            let preferences = ThemeManager::shared();
            preferences.set_browser_mode(BrowserMode::Columns);
            preferences.set_reduce_motion(true);
            let fixture = Fixture::new(false);
            fixture.preview.show(entry("clip.mp4"));
            let request = fixture.requests.borrow()[0].clone();
            fixture.resize(760);
            wait_until(|| fixture.preview.state.sizing.is_suspended());
            fixture.preview.state.handle_event(
                request.id,
                PreviewEvent::Ready(Preview {
                    request_id: request.id,
                    entry: request.entry,
                    content_type: "video/mp4".into(),
                    content: PreviewContent::SandboxedMedia {
                        media: crate::services::SandboxedMedia {
                            path: "/synthetic-video.mp4".into(),
                            size: MediaPreviewSize::new(320, 180),
                            backend: crate::sandbox::MediaPreviewBackend::Software,
                        },
                    },
                }),
            );
            let media = fixture
                .preview
                .state
                .media
                .borrow()
                .clone()
                .expect("late media result")
                .downcast::<crate::ui::media::DecodedMedia>()
                .expect("decoded media");
            crate::ui::media::tests::use_test_decoder(&media, false, 30_000_000);
            wait_until(|| media.is_prepared());
            assert!(
                !media.is_playing(),
                "late results must not autoplay while hidden"
            );
            fixture.resize(1800);
            wait_until(|| media.is_playing() && media.timestamp() > 0);
            for playing in [true, false] {
                media.set_playing(playing);
                fixture.resize(760);
                wait_until(|| fixture.preview.state.sizing.is_suspended());
                let paused_at = media.timestamp();
                assert!(!media.is_playing());
                assert!(!fixture.preview.has_video());
                assert!(!fixture.preview.handle_video_key(gtk::gdk::Key::space));
                fixture.resize(1800);
                wait_until(|| !fixture.preview.state.sizing.is_suspended());
                assert_eq!(media.is_playing(), playing);
                assert!(media.timestamp() >= paused_at);
                assert_eq!(
                    fixture.preview.state.media.borrow().as_ref(),
                    Some(media.upcast_ref())
                );
            }
            fixture.resize(760);
            wait_until(|| fixture.preview.state.sizing.is_suspended());
            fixture.preview.show(entry("other.mp4"));
            assert_eq!(media.intrinsic_width(), 0);
            assert!(!media.is_playing());
            assert_eq!(fixture.requests.borrow().len(), 1);
            fixture.window.destroy();
            assert!(!fixture.preview.is_open());
            fixture.close();
        },
    );
}

#[test]
fn temporarily_hiding_a_document_keeps_its_view_and_scroll_position() {
    crate::test_support::gtk_test(
        "ui::preview::layout::tests::visibility::temporarily_hiding_a_document_keeps_its_view_and_scroll_position",
        || {
            let preferences = ThemeManager::shared();
            preferences.set_browser_mode(BrowserMode::Columns);
            preferences.set_reduce_motion(true);
            let fixture = Fixture::new(false);
            fixture.preview.show(entry("notes.txt"));
            let request = fixture.requests.borrow()[0].clone();
            fixture.preview.state.handle_event(
                request.id,
                PreviewEvent::Ready(Preview {
                    request_id: request.id,
                    entry: request.entry,
                    content_type: "text/plain".into(),
                    content: PreviewContent::Text {
                        content: "a line of text\n".repeat(500),
                        truncated: false,
                    },
                }),
            );
            let scroll = fixture
                .preview
                .state
                .content
                .first_child()
                .expect("document view")
                .downcast::<gtk::ScrolledWindow>()
                .expect("document scroller");
            wait_until(|| scroll.vadjustment().upper() > scroll.vadjustment().page_size() + 200.0);
            scroll.vadjustment().set_value(200.0);
            fixture.settle();
            fixture.resize(760);
            wait_until(|| fixture.preview.state.sizing.is_suspended());
            fixture.resize(1800);
            wait_until(|| !fixture.preview.state.sizing.is_suspended());
            fixture.settle();
            assert_eq!(
                fixture.preview.state.content.first_child().as_ref(),
                Some(scroll.upcast_ref())
            );
            assert_eq!(scroll.vadjustment().value(), 200.0);
            assert_eq!(fixture.requests.borrow().len(), 1);
            fixture.close();
        },
    );
}
