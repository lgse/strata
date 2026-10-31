// SPDX-License-Identifier: MIT

use super::*;

const DURATION: i64 = 60_000_000;
const TICK: f64 = 1_000_000.0 / crate::media::FPS as f64;
// The last frame: seeking to the duration itself would end the stream.
const LAST: i64 = 59_966_666;

fn with_class(widget: &gtk::Widget, class: &str) -> Option<gtk::Widget> {
    if widget.has_css_class(class) {
        return Some(widget.clone());
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        if let Some(found) = with_class(&widget, class) {
            return Some(found);
        }
        child = widget.next_sibling();
    }
    None
}

#[test]
fn volume_slider_steps_pages_and_jumps_persist_the_preview_volume() {
    crate::test_support::gtk_test(
        "ui::preview::tests::media_controls::volume_slider_steps_pages_and_jumps_persist_the_preview_volume",
        || {
            crate::ui::media::use_test_streams(true);
            let provider = Rc::new(Provider::default());
            let drawer = PreviewDrawer::new(provider.clone(), false);
            let preferences = crate::ui::preferences::PreferenceManager::shared();
            preferences.set_preview_autoplay(false);
            preferences.set_preview_audio(0.3, false);
            drawer.show(entry("movie.mp4"), None);
            super::video::ready(&provider, 0, "video/mp4");
            let slider = drawer
                .state
                .media_volume_slider
                .borrow()
                .clone()
                .expect("video volume slider");
            let media = drawer.state.media.borrow().clone().expect("video stream");

            let mut mismatches = Vec::new();
            for (scroll, slider_value, volume, muted) in [
                (gtk::ScrollType::StepBackward, 0.2, 0.2, false),
                // Reaching zero mutes and keeps the volume to restore.
                (gtk::ScrollType::PageBackward, 0.0, 0.2, true),
                (gtk::ScrollType::StepForward, 0.1, 0.1, false),
                (gtk::ScrollType::PageForward, 0.3, 0.3, false),
                (gtk::ScrollType::End, 1.0, 1.0, false),
                (gtk::ScrollType::Start, 0.0, 1.0, true),
            ] {
                slider.emit_move_slider(scroll);
                let observed = (
                    slider.value(),
                    preferences.preview_volume(),
                    preferences.preview_muted(),
                    media.is_muted(),
                );
                if (observed.0 - slider_value).abs() > 0.001
                    || (observed.1 - volume).abs() > 0.001
                    || observed.2 != muted
                    || observed.3 != muted
                {
                    mismatches.push(format!(
                        "{scroll:?}: (slider, saved volume, saved muted, stream muted) = {observed:?}, \
                         expected ({slider_value}, {volume}, {muted}, {muted})"
                    ));
                }
            }
            // Repeated steps from full volume still land on zero and mute.
            for (scroll, presses) in [
                (gtk::ScrollType::StepBackward, 10),
                (gtk::ScrollType::PageBackward, 5),
            ] {
                preferences.set_preview_audio(1.0, false);
                for _ in 0..presses {
                    slider.emit_move_slider(scroll);
                }
                if slider.value() != 0.0 || !preferences.preview_muted() || !media.is_muted() {
                    mismatches.push(format!(
                        "{presses}× {scroll:?} from 1.0: slider={:e}, saved volume={:e}, \
                         muted={}, stream muted={}",
                        slider.value(),
                        preferences.preview_volume(),
                        preferences.preview_muted(),
                        media.is_muted()
                    ));
                }
            }
            drawer.close();
            crate::ui::media::use_test_streams(false);
            assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
        },
    );
}

#[test]
fn unclassified_media_seek_slider_steps_seek_the_stream_immediately() {
    crate::test_support::gtk_test(
        "ui::preview::tests::media_controls::unclassified_media_seek_slider_steps_seek_the_stream_immediately",
        || {
            crate::ui::media::use_test_streams(true);
            let provider = Rc::new(Provider::default());
            let drawer = PreviewDrawer::new(provider.clone(), false);
            let preferences = crate::ui::preferences::PreferenceManager::shared();
            preferences.set_preview_autoplay(false);
            drawer.show(entry("tune.mid"), None);
            super::video::ready(&provider, 0, "audio/midi");
            assert!(
                drawer.state.audio.borrow().is_none() && drawer.state.video.borrow().is_none(),
                "MIDI uses the generic media controls"
            );
            let media = drawer.state.media.borrow().clone().expect("media stream");
            media.stream_prepared(true, false, true, DURATION);
            let seek = with_class(drawer.state.content.upcast_ref(), "preview-media-seek")
                .and_downcast::<gtk::Scale>()
                .expect("seek slider");
            assert!(seek.is_sensitive(), "a seekable stream enables the slider");

            let mut mismatches = Vec::new();
            for (scroll, expected) in [
                (gtk::ScrollType::StepForward, 5_000_000),
                (gtk::ScrollType::PageForward, 35_000_000),
                (gtk::ScrollType::StepBackward, 30_000_000),
                (gtk::ScrollType::End, LAST),
                (gtk::ScrollType::PageBackward, LAST - 30_000_000),
                (gtk::ScrollType::Start, 0),
                (gtk::ScrollType::StepBackward, 0),
            ] {
                seek.emit_move_slider(scroll);
                let (timestamp, knob) = (media.timestamp(), seek.value());
                if (timestamp - expected).abs() as f64 > TICK
                    || (knob - timestamp as f64).abs() > TICK
                {
                    mismatches.push(format!(
                        "{scroll:?}: stream timestamp={timestamp}, knob={knob}, expected≈{expected}"
                    ));
                }
            }
            // The mouse wheel jumps without a drag in progress; a touchpad's
            // jump within the current frame leaves the stream alone.
            for (target, expected) in [(12_000_000.0, 12_000_000), (12_010_000.0, 12_000_000)] {
                seek.emit_by_name::<bool>("change-value", &[&gtk::ScrollType::Jump, &target]);
                let timestamp = media.timestamp();
                if timestamp != expected {
                    mismatches.push(format!(
                        "jump to {target}: stream timestamp={timestamp}, expected {expected}"
                    ));
                }
            }
            drawer.close();
            crate::ui::media::use_test_streams(false);
            assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
        },
    );
}
