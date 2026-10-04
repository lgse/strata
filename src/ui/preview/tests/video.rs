// SPDX-License-Identifier: MIT

use super::*;

fn ready(provider: &Provider, index: usize, content_type: &str) {
    let pending = provider.0.borrow();
    let pending = &pending[index];
    (pending.emit)(PreviewEvent::Ready(Preview {
        request_id: pending.request.id,
        entry: pending.request.entry.clone(),
        content_type: content_type.into(),
        content: PreviewContent::SandboxedMedia {
            media: crate::services::SandboxedMedia {
                path: pending
                    .request
                    .entry
                    .location
                    .native_path()
                    .expect("local clip")
                    .to_path_buf(),
                size: pending.request.media_size,
                backend: crate::sandbox::MediaPreviewBackend::Software,
                input_owner: None,
                audio_only: false,
            },
        },
    }));
}

/// A loaded folder of empty files sorted by name.
fn sorted_listing(names: &[&str]) -> (tempfile::TempDir, crate::ui::browser::BrowserView) {
    let directory = tempfile::tempdir().expect("media directory");
    for name in names {
        std::fs::write(directory.path().join(name), []).expect("media fixture");
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
    (directory, view)
}

#[test]
fn media_families_split_audio_and_video_and_skip_gifs() {
    for name in ["clip.mp4", "clip.mkv", "clip.webm", "clip.mov", "clip.avi"] {
        assert_eq!(
            entry_family(&entry(name)),
            Some(MediaFamily::Video),
            "{name}"
        );
    }
    assert_eq!(entry_family(&entry("song.mp3")), Some(MediaFamily::Audio));
    for name in ["anim.gif", "note.txt", "list.m3u", "song.mid"] {
        assert_eq!(entry_family(&entry(name)), None, "{name}");
    }
}

#[test]
fn video_steps_reuse_one_view_and_keep_playback() {
    crate::test_support::gtk_test(
        "ui::preview::tests::video::video_steps_reuse_one_view_and_keep_playback",
        || {
            let (_directory, view) = sorted_listing(&["a.mp4", "b.mp4", "c.mp4"]);
            let browser = view.browser();
            let provider = Rc::new(Provider::default());
            let drawer = PreviewDrawer::new(provider.clone(), false);
            drawer.state.keyboard_view.replace(Some(view.downgrade()));
            drawer.observe_browser(&browser);
            let preferences = crate::ui::preferences::PreferenceManager::shared();
            preferences.set_preview_autoplay(false);
            browser.select(0, 0);
            drawer.show(browser.entry_at(0, 0).expect("first clip"), Some(0));
            ready(&provider, 0, "video/mp4");
            let state = &drawer.state;
            let stream = || state.media.borrow().clone().expect("rendered stream");
            let show_cursor = || state.show(browser.cursor_entry(0).expect("clip cursor"), Some(0));
            let cursor_name = || browser.cursor_entry(0).expect("clip cursor").display_name;
            stream().play();
            assert!(stream().is_playing());
            assert!(state.audio.borrow().is_none());
            let video = state
                .video
                .borrow()
                .as_ref()
                .expect("video view")
                .view
                .clone();

            assert!(state.step_media(1, true));
            assert_eq!(cursor_name(), "b.mp4");
            show_cursor();
            assert!(
                state.media.borrow().is_none(),
                "the next clip is still loading"
            );
            assert!(Rc::ptr_eq(
                &video,
                &state.video.borrow().as_ref().expect("retained view").view
            ));
            ready(&provider, 1, "video/mp4");
            assert!(stream().is_playing(), "stepping keeps the play intent");
            assert!(Rc::ptr_eq(
                &video,
                &state.video.borrow().as_ref().expect("retained view").view
            ));

            preferences.set_preview_audio(0.4, false);
            assert!(state.media_command(gtk::gdk::Key::Up));
            assert!((preferences.preview_volume() - 0.5).abs() < 0.001);
            assert!(state.media_command(gtk::gdk::Key::m));
            assert!(preferences.preview_muted());
            assert!(stream().is_muted());

            state.show_media_error(&glib::Error::new(gio::IOErrorEnum::Failed, "bad video"));
            assert!(
                state.video.borrow().is_some(),
                "errors stay inside the view"
            );
            assert!(state.media.borrow().is_none());
            assert!(
                state.step_media(1, true),
                "a decoder error must not strand navigation"
            );
            assert_eq!(cursor_name(), "c.mp4");
            show_cursor();
            ready(&provider, 2, "video/mp4");
            assert!(!stream().is_playing(), "an error clears the play intent");
            assert!(!state.step_media(1, true), "no clip after the last one");
            drawer.close();
            assert!(state.video.borrow().is_none());
        },
    );
}

#[test]
fn steps_stay_within_the_same_media_type() {
    crate::test_support::gtk_test(
        "ui::preview::tests::video::steps_stay_within_the_same_media_type",
        || {
            let (_directory, view) = sorted_listing(&["a.mp3", "b.mp4", "c.mp3", "d.mp4"]);
            let browser = view.browser();
            let provider = Rc::new(Provider::default());
            let drawer = PreviewDrawer::new(provider.clone(), false);
            drawer.state.keyboard_view.replace(Some(view.downgrade()));
            drawer.observe_browser(&browser);
            crate::ui::preferences::PreferenceManager::shared().set_preview_autoplay(false);
            let state = &drawer.state;
            let show_cursor =
                || state.show(browser.cursor_entry(0).expect("media cursor"), Some(0));
            let cursor_name = || browser.cursor_entry(0).expect("media cursor").display_name;

            browser.select(0, 0);
            drawer.show(browser.entry_at(0, 0).expect("first track"), Some(0));
            ready(&provider, 0, "audio/mpeg");
            assert!(state.audio.borrow().is_some());
            assert!(state.step_media(1, true));
            assert_eq!(cursor_name(), "c.mp3", "audio skips the video between");
            show_cursor();
            ready(&provider, 1, "audio/mpeg");
            assert!(
                !state.step_media(1, true),
                "no audio file after the last one"
            );

            drawer.show(browser.entry_at(0, 1).expect("first clip"), Some(0));
            ready(&provider, 2, "video/mp4");
            assert!(state.audio.borrow().is_none());
            assert!(state.video.borrow().is_some());
            assert!(!state.step_media(-1, true), "no video before the first one");
            assert!(state.step_media(1, true));
            assert_eq!(cursor_name(), "d.mp4", "video skips the audio between");
            drawer.close();
        },
    );
}

#[test]
fn the_frame_shows_a_poster_or_outline_until_the_first_frame() {
    crate::test_support::gtk_test(
        "ui::preview::tests::video::the_frame_shows_a_poster_or_outline_until_the_first_frame",
        || {
            let provider = Rc::new(Provider::default());
            let drawer = PreviewDrawer::new(provider.clone(), false);
            let preferences = crate::ui::preferences::PreferenceManager::shared();
            preferences.set_preview_autoplay(false);
            preferences.set_reduce_motion(true);
            let poster = gtk::gdk::MemoryTexture::new(
                32,
                18,
                gtk::gdk::MemoryFormat::R8g8b8a8,
                &glib::Bytes::from_owned(vec![0; 32 * 18 * 4]),
                32 * 4,
            )
            .upcast::<gtk::gdk::Texture>();
            let mut clip = entry("clip.mp4");
            clip.size = MetadataValue::Known(10);
            clip.modified_unix_seconds = MetadataValue::Known(1);
            crate::ui::thumbnail::remember_thumbnail_for_test(&clip, poster.clone());

            drawer.show(clip, None);
            ready(&provider, 0, "video/mp4");
            let state = &drawer.state;
            let view = state
                .video
                .borrow()
                .as_ref()
                .expect("video view")
                .view
                .clone();
            assert!(view.placeholder().is_visible());
            assert_eq!(view.placeholder().poster().as_ref(), Some(&poster));
            let media = state
                .media
                .borrow()
                .clone()
                .and_downcast::<crate::ui::media::DecodedMedia>()
                .expect("decoded stream");
            media.present_test_frame(64, 36);
            assert!(
                !view.placeholder().is_visible(),
                "reduced motion retires the placeholder with the first frame"
            );

            drawer.show(entry("other.mp4"), None);
            assert!(view.placeholder().is_visible(), "a new clip starts covered");
            assert!(
                view.placeholder().poster().is_none(),
                "no listing thumbnail means an outline"
            );
            ready(&provider, 1, "video/mp4");
            assert!(view.placeholder().is_visible());
            preferences.set_reduce_motion(false);
            drawer.close();
        },
    );
}

#[test]
fn badges_replace_their_skeleton_once_the_probe_answers() {
    crate::test_support::gtk_test(
        "ui::preview::tests::video::badges_replace_their_skeleton_once_the_probe_answers",
        || {
            let provider = Rc::new(Provider::default());
            let drawer = PreviewDrawer::new(provider.clone(), false);
            crate::ui::preferences::PreferenceManager::shared().set_preview_autoplay(false);
            drawer.show(entry("clip.mkv"), None);
            ready(&provider, 0, "video/x-matroska");
            let view = drawer
                .state
                .video
                .borrow()
                .as_ref()
                .expect("video view")
                .view
                .clone();
            assert!(
                view.badge_labels().is_empty(),
                "skeleton pills carry no text"
            );
            assert_eq!(
                view.badges_row().observe_children().n_items(),
                3,
                "the row keeps its height with empty pills"
            );

            let metadata = crate::sandbox::metadata::MediaMetadata::from_json(
                br#"{"streams":[
                    {"codec_type":"video","codec_name":"av1","width":3840,"height":1600,
                     "pix_fmt":"yuv420p10le","color_transfer":"arib-std-b67","avg_frame_rate":"50/1"},
                    {"codec_type":"audio","codec_name":"opus","channels":2,"channel_layout":"stereo"},
                    {"codec_type":"subtitle","codec_name":"ass"}
                ],"format":{}}"#,
                false,
            )
            .expect("probe json");
            view.show_details_for_test(Some(Rc::new(metadata)));
            assert_eq!(
                view.badge_labels(),
                ["4K", "HLG", "10-bit", "50 fps", "AV1", "Opus Stereo", "CC"]
            );

            view.show_details_for_test(None);
            assert!(view.badge_labels().is_empty());
            assert_eq!(
                view.badges_row().observe_children().n_items(),
                0,
                "a failed probe leaves no pills behind"
            );
            drawer.close();
        },
    );
}
