// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn decoded_peaks_become_displayed_levels_and_clearing_lowers_them() {
    crate::test_support::gtk_test(
        "ui::preview::waveform::tests::decoded_peaks_become_displayed_levels_and_clearing_lowers_them",
        || {
            let waveform = Waveform::new();
            let mut levels = vec![0u8; BUCKETS as usize];
            levels[0] = 255;
            levels[1] = 64;
            waveform.add_levels(0, &levels);
            let shown = waveform.shown_levels();
            assert!(
                shown[0] > shown[1] && shown[1] > 0.0,
                "a quieter bucket reads lower, not flat"
            );
            assert_eq!(shown[2], 0.0, "silence stays flat");

            waveform.clear_levels();
            assert!(
                waveform.shown_levels().iter().all(|level| *level == 0.0),
                "clearing lowers the waveform to silence"
            );
        },
    );
}

#[test]
fn keys_step_and_jump_the_playback_position() {
    crate::test_support::gtk_test(
        "ui::preview::waveform::tests::keys_step_and_jump_the_playback_position",
        || {
            const DURATION: i64 = 60_000_000;
            const TICK: i64 = 1_000_000 / crate::media::FPS as i64;
            // The last frame: seeking to the duration itself would end the stream.
            const LAST: i64 = 59_966_666;
            use gtk::gdk::{Key, ModifierType as Mods};
            crate::ui::media::use_test_streams(true);
            let media = crate::ui::media::DecodedMedia::new(crate::services::SandboxedMedia {
                path: "/clip.mp4".into(),
                size: crate::services::MediaPreviewSize::new(320, 240),
                backend: crate::sandbox::MediaPreviewBackend::Software,
                input_owner: None,
                audio_only: false,
            });
            media.stream_prepared(false, true, true, DURATION);
            let waveform = Waveform::new();
            waveform.set_media(Some(media.upcast_ref()));
            let controllers = waveform.observe_controllers();
            let keys = (0..controllers.n_items())
                .find_map(|index| {
                    controllers
                        .item(index)
                        .and_downcast::<gtk::EventControllerKey>()
                })
                .expect("timeline key controller");
            let press = |key: Key, mods: Mods| {
                keys.emit_by_name::<bool>("key-pressed", &[&key, &0u32, &mods])
            };

            let mut mismatches = Vec::new();
            for (start, key, mods, expected) in [
                (20_000_000, Key::Right, Mods::empty(), Some(25_000_000)),
                (20_000_000, Key::KP_Left, Mods::empty(), Some(15_000_000)),
                (20_000_000, Key::Right, Mods::SHIFT_MASK, Some(50_000_000)),
                (40_000_000, Key::Left, Mods::SHIFT_MASK, Some(10_000_000)),
                (20_000_000, Key::Page_Up, Mods::empty(), Some(50_000_000)),
                (40_000_000, Key::Page_Down, Mods::empty(), Some(10_000_000)),
                (50_000_000, Key::KP_Page_Up, Mods::empty(), Some(LAST)),
                (10_000_000, Key::KP_Page_Down, Mods::empty(), Some(0)),
                (20_000_000, Key::Home, Mods::empty(), Some(0)),
                (20_000_000, Key::KP_Home, Mods::empty(), Some(0)),
                (20_000_000, Key::_0, Mods::empty(), Some(0)),
                // AZERTY types 0 with Shift.
                (20_000_000, Key::_0, Mods::SHIFT_MASK, Some(0)),
                (20_000_000, Key::End, Mods::empty(), Some(LAST)),
                (20_000_000, Key::KP_End, Mods::empty(), Some(LAST)),
                (20_000_000, Key::G, Mods::SHIFT_MASK, Some(LAST)),
                // Caps Lock never changes a shortcut: this is g, not G.
                (20_000_000, Key::G, Mods::LOCK_MASK, None),
                // Modified paging and jump keys stay free for other bindings.
                (20_000_000, Key::Page_Up, Mods::SHIFT_MASK, None),
                (
                    20_000_000,
                    Key::End,
                    Mods::CONTROL_MASK | Mods::ALT_MASK,
                    None,
                ),
                (20_000_000, Key::Right, Mods::CONTROL_MASK, None),
            ] {
                media.seek(start);
                let handled = press(key, mods);
                let timestamp = media.timestamp();
                let (target, claimed) = (expected.unwrap_or(start), expected.is_some());
                // A seek lands on the nearest decoder tick.
                if handled != claimed || (timestamp - target).abs() > TICK || timestamp >= DURATION
                {
                    mismatches.push(format!(
                        "{mods:?}+{}: handled={handled}, timestamp={timestamp}, expected≈{target}",
                        key.name().unwrap_or_default()
                    ));
                }
            }

            let preferences = crate::ui::preferences::PreferenceManager::shared();
            for (start, key, mods, presses, volume, muted) in [
                (0.5, Key::plus, Mods::SHIFT_MASK, 1, 0.6, false),
                (0.5, Key::equal, Mods::empty(), 1, 0.6, false),
                (0.5, Key::KP_Add, Mods::empty(), 1, 0.6, false),
                (0.5, Key::minus, Mods::empty(), 1, 0.4, false),
                (0.5, Key::KP_Subtract, Mods::empty(), 1, 0.4, false),
                // Reaching zero mutes and keeps the volume to restore, even
                // after steps that accumulate float error.
                (0.1, Key::minus, Mods::empty(), 1, 0.1, true),
                (0.3, Key::minus, Mods::empty(), 3, 0.1, true),
                (1.0, Key::minus, Mods::empty(), 10, 0.1, true),
            ] {
                preferences.set_preview_audio(start, false);
                let handled = (0..presses).all(|_| press(key, mods));
                let observed = (preferences.preview_volume(), preferences.preview_muted());
                if !handled || (observed.0 - volume).abs() > 0.001 || observed.1 != muted {
                    mismatches.push(format!(
                        "{presses}×{mods:?}+{}: handled={handled}, (volume, muted)={observed:?}, \
                         expected ({volume}, {muted})",
                        key.name().unwrap_or_default()
                    ));
                }
            }
            crate::ui::media::use_test_streams(false);
            assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
        },
    );
}
