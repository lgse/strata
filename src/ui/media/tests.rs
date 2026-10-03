// SPDX-License-Identifier: MIT

use super::*;
use std::{path::PathBuf, rc::Rc, time::Duration};

const TEST_DURATION_US: u64 = 60_000_000;

fn test_header(start_tick: u32) -> Header {
    Header {
        width: 16,
        height: 16,
        audio: false,
        duration_us: TEST_DURATION_US,
        start_tick,
    }
}

fn test_source(path: &str) -> SandboxedMedia {
    SandboxedMedia {
        path: PathBuf::from(path),
        size: MediaPreviewSize::new(320, 240),
        backend: crate::sandbox::MediaPreviewBackend::Software,
        input_owner: None,
        audio_only: false,
    }
}

fn fake_loader(calls: &Rc<RefCell<Vec<u32>>>) -> TestLoader {
    let calls = calls.clone();
    Rc::new(move |_source: SandboxedMedia, tick: u32| {
        calls.borrow_mut().push(tick);
        crate::sandbox::media::tests::stream(test_header(tick))
    })
}

fn drive_until(media: &DecodedMedia, calls: &Rc<RefCell<Vec<u32>>>, want: usize) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while calls.borrow().len() < want {
        match media.tick() {
            Ok(()) => {}
            // Worker slots are shared with other tests.
            Err(error) if error.contains("busy") => {}
            Err(error) => panic!("tick failed: {error}"),
        }
        assert!(Instant::now() < deadline, "decode worker deadline");
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn open_restoring(path: &str, position: u64) -> (DecodedMedia, Rc<RefCell<Vec<u32>>>) {
    remember_media_position(path.into(), position);
    let media = DecodedMedia::new(test_source(path));
    let calls = Rc::new(RefCell::new(Vec::new()));
    media.imp().loader.replace(Some(fake_loader(&calls)));
    media.upcast_ref::<gtk::MediaStream>().play();
    (media, calls)
}

#[test]
fn reopening_resumes_where_the_preview_closed() {
    gtk::init().expect("GTK display");

    let (media, calls) = open_restoring("/remembered", media::timestamp(900));
    let deadline = Instant::now() + Duration::from_secs(15);
    while (media.timestamp() as u64) < media::timestamp(900) {
        media.tick().expect("tick succeeds");
        assert!(Instant::now() < deadline, "resume deadline");
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(*calls.borrow(), vec![0, 900]);
    let position = media.timestamp() as u64;
    media.close();
    assert_eq!(
        recall_media_position(Path::new("/remembered")),
        Some(position)
    );

    let (media, calls) = open_restoring("/shortened", TEST_DURATION_US + 1);
    drive_until(&media, &calls, 1);
    for _ in 0..20 {
        media.tick().expect("tick succeeds");
    }
    assert_eq!(*calls.borrow(), vec![0]);
    assert!(media.imp().first_frame.get());
    media.close();
    assert_eq!(recall_media_position(Path::new("/shortened")), None);

    let (media, calls) = open_restoring("/ended", media::timestamp(900));
    drive_until(&media, &calls, 2);
    for _ in 0..20 {
        media.tick().expect("tick succeeds");
    }
    media.imp().position.set(TEST_DURATION_US);
    media.close();
    assert_eq!(recall_media_position(Path::new("/ended")), None);

    let (media, calls) = open_restoring("/ended", media::timestamp(900));
    drive_until(&media, &calls, 2);
    media.imp().position.set(RESTORE_MIN_US);
    media.close();
    assert_eq!(recall_media_position(Path::new("/ended")), None);

    remember_media_position("/movie.ogg".into(), media::timestamp(900));
    let media = DecodedMedia::new(SandboxedMedia {
        audio_only: true,
        ..test_source("/movie.ogg")
    });
    let calls = Rc::new(RefCell::new(Vec::new()));
    media.imp().loader.replace(Some(fake_loader(&calls)));
    drive_until(&media, &calls, 2);
    assert_eq!(
        *calls.borrow(),
        vec![0, 900],
        "probed video resumes despite an audio extension"
    );
    media.close();

    remember_media_position("/song".into(), media::timestamp(900));
    let media = DecodedMedia::new(SandboxedMedia {
        audio_only: true,
        ..test_source("/song")
    });
    let calls = Rc::new(RefCell::new(Vec::new()));
    let audio_calls = calls.clone();
    media.imp().loader.replace(Some(Rc::new(move |_, tick| {
        audio_calls.borrow_mut().push(tick);
        crate::sandbox::media::tests::stream(Header {
            width: 0,
            height: 0,
            audio: true,
            ..test_header(tick)
        })
    })));
    media.upcast_ref::<gtk::MediaStream>().play();
    drive_until(&media, &calls, 1);
    for _ in 0..20 {
        media.tick().expect("tick succeeds");
    }
    assert_eq!(*calls.borrow(), vec![0]);
    media.imp().position.set(media::timestamp(600));
    media.close();
    assert_eq!(recall_media_position(Path::new("/song")), None);
}

fn pcm(values: impl IntoIterator<Item = i16>) -> Vec<u8> {
    values
        .into_iter()
        .flat_map(|value| {
            let bytes = value.to_le_bytes();
            [bytes[0], bytes[1], bytes[0], bytes[1]]
        })
        .collect()
}

#[test]
fn played_audio_window_ends_at_the_playhead_not_the_decoder() {
    let mut history = PcmHistory::default();
    history.push(&pcm((0..8).map(|value| value * 4096)));
    let mut window = [1.0; 4];

    assert!(history.window_ending_at(6, &mut window));
    assert_eq!(window, [0.25, 0.375, 0.5, 0.625]);

    assert!(history.window_ending_at(2, &mut window));
    assert_eq!(window, [0.0, 0.0, 0.0, 0.125]);

    assert!(history.window_ending_at(100, &mut window));
    assert_eq!(
        window[3], 0.875,
        "a lagging sink never reads past decoded audio"
    );
}

#[test]
fn played_audio_history_drops_samples_older_than_its_window() {
    let mut history = PcmHistory::default();
    history.push(&pcm(std::iter::repeat_n(1, HISTORY_SAMPLES + 10)));
    let mut window = [0.0; 4];
    assert!(!history.window_ending_at(5, &mut window));
    assert!(history.window_ending_at(HISTORY_SAMPLES as u64 + 10, &mut window));

    history.clear();
    assert!(!history.window_ending_at(1, &mut window));
}

// Holds 300 ms before rendering and then consumes in real time, so the sink
// position follows rendered data the way PipeWire-Pulse and Bluetooth sinks do.
const LATE_SINK: &str = "queue min-threshold-time=300000000 max-size-time=400000000 \
    max-size-bytes=0 max-size-buffers=0 ! identity sleep-time=33333 ! fakesink sync=false";

fn open_on_late_sink(
    header: Header,
    size: MediaPreviewSize,
) -> (DecodedMedia, Rc<RefCell<Vec<u32>>>) {
    let media = DecodedMedia::new(SandboxedMedia {
        audio_only: header.width == 0,
        size,
        ..test_source("/late-sink")
    });
    media.imp().audio_sink.replace(Some(LATE_SINK.into()));
    let calls = Rc::new(RefCell::new(Vec::new()));
    let loader_calls = calls.clone();
    media.imp().loader.replace(Some(Rc::new(move |_, tick| {
        loader_calls.borrow_mut().push(tick);
        crate::sandbox::media::tests::stream(Header {
            start_tick: tick,
            ..header
        })
    })));
    media.upcast_ref::<gtk::MediaStream>().play();
    drive_until(&media, &calls, 1);
    (media, calls)
}

#[test]
fn audio_feeds_a_sink_that_starts_late_without_rewinding_the_playhead() {
    crate::test_support::gtk_test(
        "ui::media::tests::audio_feeds_a_sink_that_starts_late_without_rewinding_the_playhead",
        audio_feeds_a_late_sink,
    );
}

fn audio_feeds_a_late_sink() {
    let header = Header {
        width: 0,
        height: 0,
        audio: true,
        ..test_header(0)
    };
    let (media, _calls) = open_on_late_sink(header, MediaPreviewSize::new(320, 240));
    let imp = media.imp();
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut shown = 0;
    let mut deepest = 0;
    while shown < 600_000 {
        media.tick().expect("tick succeeds");
        let now = media.timestamp() as u64;
        assert!(now >= shown, "playhead rewound from {shown} to {now}");
        shown = now;
        let heard = imp
            .audio
            .borrow()
            .as_ref()
            .and_then(PcmOutput::position_us)
            .unwrap_or(0);
        assert!(
            shown <= heard + 100_000,
            "playhead {shown} ran ahead of the sink at {heard}"
        );
        deepest = deepest.max(imp.frames.borrow().len());
        assert!(Instant::now() < deadline, "late sink playback deadline");
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(imp.recoveries.get(), 0, "a late start is not a stall");
    assert!(
        deepest > PRESENTATION_QUEUE,
        "audio read ahead stopped at the presentation queue ({deepest})"
    );
    media.close();
}

#[test]
fn video_lookahead_is_bounded_by_the_pixel_budget_and_silent_video_by_the_queue() {
    crate::test_support::gtk_test(
        "ui::media::tests::video_lookahead_is_bounded_by_the_pixel_budget_and_silent_video_by_the_queue",
        video_lookahead_bounds,
    );
}

fn video_lookahead_bounds() {
    for (edge, audio, expected) in [
        (16, false, PRESENTATION_QUEUE..=PRESENTATION_QUEUE),
        (16, true, PRESENTATION_QUEUE + 1..=usize::MAX),
        (
            1280,
            true,
            PRESENTATION_QUEUE + 1..=PRESENTATION_BYTES / (1280 * 1280 * 4),
        ),
    ] {
        let header = Header {
            width: edge,
            height: edge,
            audio,
            ..test_header(0)
        };
        let (media, _calls) =
            open_on_late_sink(header, MediaPreviewSize::new(edge as i32, edge as i32));
        let imp = media.imp();
        let until = Instant::now() + Duration::from_millis(700);
        let mut deepest = 0;
        while Instant::now() < until {
            media.tick().expect("tick succeeds");
            deepest = deepest.max(imp.frames.borrow().len());
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(
            expected.contains(&deepest),
            "{edge}px audio={audio}: queued {deepest} frames, expected {expected:?}"
        );
        media.close();
    }
}
