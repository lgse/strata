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
