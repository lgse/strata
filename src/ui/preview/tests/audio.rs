// SPDX-License-Identifier: MIT

use super::*;

fn ready(provider: &Provider, index: usize) {
    let pending = provider.0.borrow();
    let pending = &pending[index];
    (pending.emit)(PreviewEvent::Ready(Preview {
        request_id: pending.request.id,
        entry: pending.request.entry.clone(),
        content_type: "audio/x-wav".into(),
        content: PreviewContent::SandboxedMedia {
            media: crate::services::SandboxedMedia {
                path: pending
                    .request
                    .entry
                    .location
                    .native_path()
                    .expect("local track")
                    .to_path_buf(),
                size: pending.request.media_size,
                backend: crate::sandbox::MediaPreviewBackend::Software,
                input_owner: None,
                audio_only: false,
            },
        },
    }));
}

#[test]
fn track_candidates_exclude_playlists_and_midi() {
    for name in ["song.mp3", "song.flac", "song.opus", "song.ogg", "song.wav"] {
        assert!(is_audio_entry(&entry(name)), "{name}");
    }
    for name in [
        "list.m3u",
        "list.m3u8",
        "list.pls",
        "song.mid",
        "song.midi",
        "note.txt",
    ] {
        assert!(!is_audio_entry(&entry(name)), "{name}");
    }
}

#[test]
fn audio_steps_keep_playback_and_volume_but_never_reuse_an_ended_request() {
    crate::test_support::gtk_test(
        "ui::preview::tests::audio::audio_steps_keep_playback_and_volume_but_never_reuse_an_ended_request",
        || {
            let directory = tempfile::tempdir().expect("track directory");
            for name in ["a.wav", "b.wav", "c.wav"] {
                std::fs::write(directory.path().join(name), []).expect("track fixture");
            }
            let view = crate::ui::browser::BrowserView::new(
                Rc::new(crate::adapters::LocalFileSource),
                crate::ui::browser::PeekBehavior::default(),
            );
            let browser = view.browser();
            browser.navigate(Location::local(directory.path()));
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            while !browser
                .column_snapshot(0)
                .is_some_and(|column| !column.loading)
            {
                assert!(std::time::Instant::now() < deadline);
                glib::MainContext::default().iteration(false);
            }
            browser.set_sort(
                0,
                crate::model::SortKey::Name,
                crate::model::SortDirection::Ascending,
            );
            let provider = Rc::new(Provider::default());
            let drawer = PreviewDrawer::new(provider.clone(), false);
            drawer.state.keyboard_view.replace(Some(view.downgrade()));
            drawer.observe_browser(&browser);
            let preferences = crate::ui::preferences::PreferenceManager::shared();
            preferences.set_preview_autoplay(false);
            browser.select(0, 0);
            drawer.show(browser.entry_at(0, 0).expect("first track"), Some(0));
            ready(&provider, 0);
            let state = &drawer.state;
            let stream = || state.media.borrow().clone().expect("rendered stream");
            let show_cursor =
                || state.show(browser.cursor_entry(0).expect("track cursor"), Some(0));
            stream().play();
            assert!(stream().is_playing());
            let audio = state
                .audio
                .borrow()
                .as_ref()
                .expect("audio view")
                .view
                .clone();

            assert!(state.step_media(1, true));
            show_cursor();
            assert!(state.media.borrow().is_none());
            assert!(Rc::ptr_eq(
                &audio,
                &state.audio.borrow().as_ref().expect("retained view").view
            ));
            preferences.set_preview_audio(0.4, false);
            assert!(state.media_command(gtk::gdk::Key::Up));
            assert!((preferences.preview_volume() - 0.5).abs() < 0.001);
            assert!(state.media_command(gtk::gdk::Key::m));
            assert!(preferences.preview_muted());

            assert!(state.step_media(1, true));
            show_cursor();
            ready(&provider, 2);
            assert!(
                stream().is_playing(),
                "second step while loading keeps play intent"
            );
            assert!(stream().is_muted());

            state.show_media_error(&glib::Error::new(gio::IOErrorEnum::Failed, "bad audio"));
            assert!(state.audio.borrow().is_some());
            assert!(
                state.step_media(-1, true),
                "a decoder error must not strand navigation"
            );
            show_cursor();
            ready(&provider, 3);
            stream().play();

            assert!(state.step_media(-1, true));
            show_cursor();
            {
                let pending = provider.0.borrow();
                let pending = &pending[4];
                (pending.emit)(PreviewEvent::Failed {
                    request_id: pending.request.id,
                    entry: pending.request.entry.clone(),
                    message: "load failed".into(),
                });
            }
            assert!(state.continue_playback.borrow().is_none());
            drawer.show(browser.entry_at(0, 0).expect("first track"), Some(0));
            ready(&provider, 5);
            assert!(!stream().is_playing());

            stream().play();
            assert!(state.step_media(1, true));
            show_cursor();
            drawer.close();
            drawer.show(browser.entry_at(0, 1).expect("second track"), Some(0));
            ready(&provider, 7);
            assert!(
                !stream().is_playing(),
                "closing cancels pending play intent"
            );
            drawer.close();
        },
    );
}

#[test]
fn probed_video_replaces_the_audio_view_without_stopping_playback() {
    crate::test_support::gtk_test(
        "ui::preview::tests::audio::probed_video_replaces_the_audio_view_without_stopping_playback",
        || {
            let provider = Rc::new(Provider::default());
            let drawer = PreviewDrawer::new(provider.clone(), false);
            drawer.show(entry("movie.ogg"), None);
            ready(&provider, 0);
            let media = drawer.state.media.borrow().clone().expect("rendered media");
            media.play();
            media.stream_prepared(true, true, true, 10_000_000);
            assert!(drawer.state.audio.borrow().is_none());
            assert_eq!(drawer.state.media.borrow().as_ref(), Some(&media));
            assert!(media.is_playing());
            drawer.close();
        },
    );
}
