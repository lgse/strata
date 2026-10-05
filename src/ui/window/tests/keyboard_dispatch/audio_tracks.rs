// SPDX-License-Identifier: MIT

use super::*;
use crate::services::SandboxedMedia;

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

pub(super) fn settle(step: &str, condition: impl Fn() -> bool) {
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

fn shows_track(fixture: &KeyboardFixture, name: &str) -> bool {
    let preview = fixture.preview.widget();
    widget_with_class(&preview, "preview-audio").is_some()
        && widget_with_class(&preview, "preview-title")
            .and_downcast::<gtk::Label>()
            .is_some_and(|label| label.text() == name)
}

#[test]
fn audio_steps_follow_search_results_including_subfolders() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::audio_tracks::audio_steps_follow_search_results_including_subfolders",
        || {
            let fixture = KeyboardFixture::with_provider(Rc::new(AudioPreview));
            let root = fixture._directory.path();
            std::fs::create_dir(root.join("nested")).expect("nested folder");
            for name in [
                "a-song.wav",
                "b-hidden.wav",
                "nested/c-song.wav",
                "d-song.m3u",
                "e-song.mid",
            ] {
                std::fs::write(root.join(name), []).expect("filter fixture");
            }
            let preferences = footer_prompt::enable_tenxer(&fixture);
            preferences.set_group_by_type(false);
            let browser = fixture.view.browser();
            fixture.preview.observe_browser(&browser);
            fixture.view.refresh();
            wait_loaded(&browser, 0);
            browser.set_sort(0, SortKey::Name, SortDirection::Ascending);
            for mode in [BrowserMode::Columns, BrowserMode::List, BrowserMode::Icons] {
                fixture.preview.close();
                fixture.view.dismiss_listing_search();
                fixture.view.set_view_mode(mode);
                wait_loaded(&browser, 0);
                move_to_named(&fixture, &browser, "b-hidden.wav");
                let hidden_cursor = || browser.cursor_entry(0).expect("directory cursor").location;
                let hidden = hidden_cursor();
                footer_prompt::commit_search(&fixture, "song");
                footer_prompt::wait_results(
                    &fixture,
                    &["a-song.wav", "c-song.wav", "d-song.m3u", "e-song.mid"],
                );
                fixture.press(Key::Home, ModifierType::empty());
                for _ in 0..4 {
                    pump(40);
                    if fixture
                        .view
                        .selected_search_result()
                        .is_some_and(|entry| entry.display_name == "a-song.wav")
                    {
                        break;
                    }
                    fixture.press(Key::j, ModifierType::empty());
                }
                let first = fixture
                    .view
                    .selected_search_result()
                    .expect("first audio result");
                assert_eq!(first.display_name, "a-song.wav");
                fixture.preview.show(first, Some(0));
                fixture.preview.take_keyboard();
                settle("first filtered track", || {
                    shows_track(&fixture, "a-song.wav") && preview_has_focus(&fixture)
                });
                assert!(
                    fixture.press(Key::greater, ModifierType::SHIFT_MASK),
                    "{mode:?}"
                );
                settle("nested track", || shows_track(&fixture, "c-song.wav"));
                assert!(preview_has_focus(&fixture));
                settle("results caption", || {
                    widget_with_class(&fixture.preview.widget(), "preview-audio-eyebrow")
                        .and_downcast::<gtk::Label>()
                        .is_some_and(|label| label.text() == "2 of 2 in results")
                });
                assert_eq!(hidden_cursor(), hidden);
                fixture.press(Key::h, ModifierType::empty());
                settle("filtered listing keys", || file_panes_have_focus(&fixture));
                assert!(fixture.press(Key::less, ModifierType::SHIFT_MASK));
                settle(&format!("{mode:?} previous filtered track"), || {
                    shows_track(&fixture, "a-song.wav")
                });
                assert_eq!(hidden_cursor(), hidden);
            }
        },
    );
}

#[test]
fn tenxer_angle_brackets_step_through_audio_files_from_the_preview() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::audio_tracks::tenxer_angle_brackets_step_through_audio_files_from_the_preview",
        || {
            let fixture = KeyboardFixture::with_provider(Rc::new(AudioPreview));
            let root = fixture._directory.path();
            for name in ["a.wav", "c.wav", "d.wav"] {
                std::fs::write(root.join(name), []).expect("audio fixture");
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
                    == ["a.txt", "a.wav", "b.txt", "c.txt", "c.wav", "d.wav"]
            });
            move_to_named(&fixture, &browser, "a.wav");
            fixture.press(Key::l, ModifierType::empty());
            settle("first track", || {
                preview_has_focus(&fixture) && shows_track(&fixture, "a.wav")
            });

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
