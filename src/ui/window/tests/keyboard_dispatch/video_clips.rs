// SPDX-License-Identifier: MIT

use super::audio_tracks::settle;
use super::*;
use crate::services::SandboxedMedia;

/// Types files by extension so one folder can mix audio and video.
struct MediaPreview;

impl PreviewProvider for MediaPreview {
    fn load(&self, request: PreviewRequest, emit: Rc<dyn Fn(PreviewEvent)>) -> LoadHandle {
        glib::idle_add_local_once(move || {
            let path = request
                .entry
                .location
                .native_path()
                .expect("local fixture")
                .to_path_buf();
            let content_type = if path.extension().is_some_and(|extension| extension == "mp4") {
                "video/mp4"
            } else {
                "audio/x-wav"
            };
            emit(PreviewEvent::Ready(Preview {
                request_id: request.id,
                entry: request.entry,
                content_type: content_type.into(),
                content: PreviewContent::SandboxedMedia {
                    media: SandboxedMedia {
                        path,
                        size: request.media_size,
                        backend: crate::sandbox::MediaPreviewBackend::Software,
                        input_owner: None,
                        audio_only: false,
                    },
                },
            }))
        });
        LoadHandle::new(|| {})
    }
}

fn shows_clip(fixture: &KeyboardFixture, name: &str) -> bool {
    let preview = fixture.preview.widget();
    widget_with_class(&preview, "preview-video").is_some()
        && widget_with_class(&preview, "preview-title")
            .and_downcast::<gtk::Label>()
            .is_some_and(|label| label.text() == name)
}

#[test]
fn tenxer_angle_brackets_step_through_videos_and_skip_other_media() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::video_clips::tenxer_angle_brackets_step_through_videos_and_skip_other_media",
        || {
            let fixture = KeyboardFixture::with_provider(Rc::new(MediaPreview));
            let root = fixture._directory.path();
            for name in ["a.wav", "b.mp4", "c.wav", "d.mp4", "e.mp4"] {
                std::fs::write(root.join(name), []).expect("media fixture");
            }
            let preferences = PreferenceManager::shared();
            preferences.set_tenxer_mode(true);
            preferences.set_group_by_type(false);
            let browser = fixture.view.browser();
            fixture.preview.observe_browser(&browser);
            fixture.view.refresh();
            wait_loaded(&browser, 0);
            browser.set_folders_first(0, false);
            browser.set_sort(0, SortKey::Name, SortDirection::Ascending);
            fixture.view.set_view_mode(BrowserMode::List);
            settle("sorted listing", || {
                list_display_names(&fixture.view.widget())
                    == [
                        "a.txt", "a.wav", "b.mp4", "b.txt", "c.txt", "c.wav", "d.mp4", "e.mp4",
                    ]
            });
            move_to_named(&fixture, &browser, "b.mp4");
            fixture.press(Key::l, ModifierType::empty());
            settle("first clip", || {
                preview_has_focus(&fixture) && shows_clip(&fixture, "b.mp4")
            });

            for (key, expected) in [
                (Key::greater, "d.mp4"),
                (Key::greater, "e.mp4"),
                (Key::greater, "e.mp4"),
                (Key::less, "d.mp4"),
                (Key::less, "b.mp4"),
            ] {
                assert!(fixture.press(key, ModifierType::SHIFT_MASK), "{key:?}");
                settle(&format!("{key:?} cursor"), || {
                    focused_name(&browser) == expected
                });
                settle(&format!("{key:?} clip"), || shows_clip(&fixture, expected));
                assert!(preview_has_focus(&fixture), "{key:?} kept the keys");
            }
            settle("folder caption", || {
                widget_with_class(&fixture.preview.widget(), "preview-video-eyebrow")
                    .and_downcast::<gtk::Label>()
                    .is_some_and(|label| label.text() == "1 of 3 in folder")
            });

            fixture.press(Key::h, ModifierType::empty());
            settle("listing keys", || file_panes_have_focus(&fixture));
            assert!(
                fixture.press(Key::greater, ModifierType::SHIFT_MASK),
                "> from the listing"
            );
            settle("listing step", || {
                focused_name(&browser) == "d.mp4" && shows_clip(&fixture, "d.mp4")
            });
            assert!(file_panes_have_focus(&fixture), "the listing kept the keys");
        },
    );
}

/// A default video player named `mpv` that records its arguments.
fn player_recorder(mime_type: &str, output: &std::path::Path) {
    let id = "strata-mpv-recorder";
    let applications = glib::user_data_dir().join("applications");
    std::fs::create_dir_all(&applications).expect("applications");
    let script = applications.join("mpv");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\n",
            output.display()
        ),
    )
    .expect("recorder");
    std::fs::set_permissions(
        &script,
        <std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o755),
    )
    .expect("executable");
    std::fs::write(
        applications.join(format!("{id}.desktop")),
        format!(
            "[Desktop Entry]\nType=Application\nName=Player Recorder\nExec={} %U\nMimeType={mime_type};\n",
            script.display()
        ),
    )
    .expect("desktop file");
    std::fs::create_dir_all(glib::user_config_dir()).expect("config");
    std::fs::write(
        glib::user_config_dir().join("mimeapps.list"),
        format!(
            "[Added Associations]\n{mime_type}={id}.desktop;\n[Default Applications]\n{mime_type}={id}.desktop\n"
        ),
    )
    .expect("associations");
}

#[test]
fn enter_on_a_video_preview_opens_it_where_it_stopped() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::video_clips::enter_on_a_video_preview_opens_it_where_it_stopped",
        || {
            crate::ui::media::use_test_streams(true);
            let fixture = KeyboardFixture::with_provider(Rc::new(MediaPreview));
            let root = fixture._directory.path();
            // GIO types an empty file as zero-size; a real MP4 signature keeps it a video.
            std::fs::write(
                root.join("b.mp4"),
                b"\x00\x00\x00\x18ftypmp42\x00\x00\x00\x00mp42isom",
            )
            .expect("clip");
            let output = root.join("launch.txt");
            player_recorder("video/mp4", &output);
            let preferences = PreferenceManager::shared();
            preferences.set_tenxer_mode(true);
            preferences.set_group_by_type(false);
            preferences.set_preview_autoplay(false);
            let browser = fixture.view.browser();
            fixture.preview.observe_browser(&browser);
            fixture.view.refresh();
            wait_loaded(&browser, 0);
            browser.set_folders_first(0, false);
            browser.set_sort(0, SortKey::Name, SortDirection::Ascending);
            fixture.view.set_view_mode(BrowserMode::List);
            move_to_named(&fixture, &browser, "b.mp4");
            fixture.press(Key::l, ModifierType::empty());
            settle("the clip", || {
                preview_has_focus(&fixture) && shows_clip(&fixture, "b.mp4")
            });
            let media = fixture.preview.media_for_test().expect("video stream");
            media.play();
            settle("playback past the first second", || {
                media.timestamp() > 1_500_000
            });

            assert!(fixture.press(Key::Return, ModifierType::empty()));
            settle("the 10xer launch", || output.exists());
            let recorded = std::fs::read_to_string(&output).expect("recorded arguments");
            assert!(
                recorded.starts_with("--start=1.") || recorded.starts_with("--start=2."),
                "{recorded}"
            );
            assert!(recorded.contains("b.mp4"), "{recorded}");
            assert!(!media.is_playing(), "the preview pauses for the handoff");

            std::fs::remove_file(&output).expect("reset recorder");
            preferences.set_tenxer_mode(false);
            media.play();
            assert!(fixture.preview.take_keyboard());
            settle("preview focus", || preview_has_focus(&fixture));
            assert!(fixture.press(Key::Return, ModifierType::empty()));
            settle("the default-map launch", || output.exists());
            let recorded = std::fs::read_to_string(&output).expect("recorded arguments");
            assert!(recorded.starts_with("--start="), "{recorded}");
            assert!(!media.is_playing());
            crate::ui::media::use_test_streams(false);
        },
    );
}
