// SPDX-License-Identifier: MIT

use super::*;
use crate::services::SandboxedMedia;

/// Serves every file as a sandboxed audio preview, like the local provider does for audio.
struct AudioPreview;

impl PreviewProvider for AudioPreview {
    fn load(&self, request: PreviewRequest, emit: Rc<dyn Fn(PreviewEvent)>) -> LoadHandle {
        glib::idle_add_local_once(move || {
            let path = request
                .entry
                .location
                .native_path()
                .expect("local fixture")
                .to_path_buf();
            emit(PreviewEvent::Ready(Preview {
                request_id: request.id,
                entry: request.entry,
                content_type: "audio/x-wav".into(),
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

fn settle(step: &str, condition: impl Fn() -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !condition() {
        assert!(
            std::time::Instant::now() < deadline,
            "{step} did not settle"
        );
        glib::MainContext::default().iteration(false);
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

/// The audio view shows `name`; its tag line may still be loading.
fn shows_track(fixture: &KeyboardFixture, name: &str) -> bool {
    let preview = fixture.preview.widget();
    widget_with_class(&preview, "preview-audio").is_some()
        && widget_with_class(&preview, "preview-title")
            .and_downcast::<gtk::Label>()
            .is_some_and(|label| label.text() == name)
}

#[test]
fn tenxer_angle_brackets_step_through_audio_files_from_the_preview() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::audio_tracks::tenxer_angle_brackets_step_through_audio_files_from_the_preview",
        || {
            let fixture = KeyboardFixture::with_provider(Rc::new(AudioPreview));
            let root = fixture._directory.path();
            for name in ["a.wav", "c.wav", "d.wav"] {
                let status = std::process::Command::new("ffmpeg")
                    .args(["-nostdin", "-v", "error", "-f", "lavfi", "-i"])
                    .arg("sine=frequency=440:duration=1")
                    .arg(root.join(name))
                    .status()
                    .expect("FFmpeg tools are required");
                assert!(status.success());
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
            // The fixture's text files sit between the tracks.
            settle("sorted listing", || {
                list_display_names(&fixture.view.widget())
                    == ["a.txt", "a.wav", "b.txt", "c.txt", "c.wav", "d.wav"]
            });
            move_to_named(&fixture, &browser, "a.wav");
            fixture.press(Key::l, ModifierType::empty());
            settle("first track", || {
                preview_has_focus(&fixture) && shows_track(&fixture, "a.wav")
            });

            // Repeated presses outrun the preview's debounce and still advance.
            assert!(fixture.press(Key::greater, ModifierType::SHIFT_MASK));
            assert!(fixture.press(Key::greater, ModifierType::SHIFT_MASK));
            settle("repeated steps", || focused_name(&browser) == "d.wav");
            settle("repeated steps track", || shows_track(&fixture, "d.wav"));
            assert!(fixture.press(Key::less, ModifierType::SHIFT_MASK));
            assert!(fixture.press(Key::less, ModifierType::SHIFT_MASK));
            settle("repeated steps back", || focused_name(&browser) == "a.wav");

            for (key, expected) in [
                (Key::greater, "c.wav"),
                (Key::greater, "d.wav"),
                (Key::greater, "d.wav"),
                (Key::less, "c.wav"),
            ] {
                assert!(fixture.press(key, ModifierType::SHIFT_MASK), "{key:?}");
                settle(&format!("{key:?} cursor"), || {
                    focused_name(&browser) == expected
                });
                settle(&format!("{key:?} track"), || {
                    shows_track(&fixture, expected)
                });
                assert!(preview_has_focus(&fixture), "{key:?} kept the keys");
            }

            // From the listing, like J / K, they act on the open audio preview.
            fixture.press(Key::h, ModifierType::empty());
            settle("listing keys", || file_panes_have_focus(&fixture));
            for (key, modifiers, expected) in [
                (Key::greater, ModifierType::SHIFT_MASK, "d.wav"),
                (Key::less, ModifierType::empty(), "c.wav"),
            ] {
                assert!(fixture.press(key, modifiers), "{key:?} from the listing");
                settle(&format!("{key:?} listing cursor"), || {
                    focused_name(&browser) == expected && shows_track(&fixture, expected)
                });
                assert!(
                    file_panes_have_focus(&fixture),
                    "{key:?} kept the listing keys"
                );
            }
        },
    );
}
