// SPDX-License-Identifier: MIT

use super::*;

fn ready(provider: &Provider, index: usize) {
    super::video::ready(provider, index, "audio/x-wav");
}

#[test]
fn audio_steps_keep_playback_and_volume_but_never_reuse_an_ended_request() {
    crate::test_support::gtk_test(
        "ui::preview::tests::audio::audio_steps_keep_playback_and_volume_but_never_reuse_an_ended_request",
        || {
            let (_directory, view) = super::video::sorted_listing(&["a.wav", "b.wav", "c.wav"]);
            let browser = view.browser();
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
            super::video::failed(&provider, 4, "load failed");
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
            let preferences = crate::ui::preferences::PreferenceManager::shared();
            preferences.set_preview_autoplay(true);
            drawer.show(entry("movie.ogg"), None);
            ready(&provider, 0);
            let media = drawer.state.media.borrow().clone().expect("rendered media");
            let decoded = media
                .downcast_ref::<crate::ui::media::DecodedMedia>()
                .expect("decoded stream");
            assert!(media.is_playing());
            assert_eq!(decoded.fade(), 0.0, "autoplay starts silent");
            media.stream_prepared(true, true, true, 10_000_000);
            assert!(drawer.state.audio.borrow().is_none());
            assert_eq!(drawer.state.media.borrow().as_ref(), Some(&media));
            assert!(media.is_playing());
            assert_eq!(decoded.fade(), 1.0, "the video view takes over with sound");
            preferences.set_preview_autoplay(false);
            drawer.close();
        },
    );
}

#[test]
fn autoplayed_audio_fades_in_quickly_unless_the_listener_acts() {
    crate::test_support::gtk_test(
        "ui::preview::tests::audio::autoplayed_audio_fades_in_quickly_unless_the_listener_acts",
        || {
            let provider = Rc::new(Provider::default());
            let drawer = PreviewDrawer::new(provider.clone(), false);
            let preferences = crate::ui::preferences::PreferenceManager::shared();
            preferences.set_preview_autoplay(true);
            preferences.set_preview_audio(0.8, false);
            let state = &drawer.state;
            let decoded = || {
                state
                    .media
                    .borrow()
                    .clone()
                    .and_downcast::<crate::ui::media::DecodedMedia>()
                    .expect("decoded stream")
            };
            let view = || {
                state
                    .audio
                    .borrow()
                    .as_ref()
                    .expect("audio view")
                    .view
                    .clone()
            };
            let pump = |until: &dyn Fn() -> bool, what: &str| {
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
                while !until() {
                    assert!(
                        std::time::Instant::now() < deadline,
                        "{what} did not happen"
                    );
                    glib::MainContext::default().iteration(false);
                    std::thread::sleep(std::time::Duration::from_millis(2));
                }
            };

            drawer.show(entry("a.wav"), None);
            ready(&provider, 0);
            let media = decoded();
            media.use_test_audio_stream();
            assert!(media.is_playing(), "autoplay starts the track");
            assert_eq!(media.fade(), 0.0, "silently");
            assert!(view().is_easing_in());
            pump(&|| media.fade() > 0.0, "the rise once sound flows");
            pump(&|| media.fade() == 1.0, "the half-second fade");
            assert!(!view().is_easing_in());
            assert_eq!(media.volume(), 0.8, "the saved volume is untouched");

            drawer.show(entry("b.wav"), None);
            ready(&provider, 1);
            let media = decoded();
            media.use_test_audio_stream();
            assert_eq!(media.fade(), 0.0);
            assert!(state.media_command(gtk::gdk::Key::space));
            assert_eq!(media.fade(), 1.0, "pausing brings the sound in at once");
            assert!(!view().is_easing_in());

            drawer.show(entry("c.wav"), None);
            ready(&provider, 2);
            let media = decoded();
            media.use_test_audio_stream_lasting(3_000_000);
            assert_eq!(media.fade(), 0.0);
            pump(&|| media.fade() > 0.0, "sound on a short file");
            assert_eq!(media.fade(), 1.0, "a file under ten seconds skips the rise");
            assert!(!view().is_easing_in());

            preferences.set_preview_autoplay(false);
            drawer.close();
        },
    );
}
