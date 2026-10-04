// SPDX-License-Identifier: MIT

use super::*;
use gstreamer as gst;
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
fn played_audio_history_keeps_unplayed_samples_and_drops_old_ones() {
    let mut history = PcmHistory::default();
    history.push(&pcm(std::iter::repeat_n(1, 5 * HISTORY_SAMPLES)));
    let mut window = [0.0; 4];
    let window_samples = HISTORY_SAMPLES as u64;

    history.trim_behind(window_samples / 2);
    assert!(
        history.window_ending_at(window_samples / 2, &mut window),
        "a sink far behind the decoder still finds its samples"
    );
    assert!(history.window_ending_at(5, &mut window));

    history.trim_behind(3 * window_samples);
    assert!(!history.window_ending_at(5, &mut window));
    assert!(!history.window_ending_at(2 * window_samples - 1, &mut window));
    assert!(history.window_ending_at(2 * window_samples + 4, &mut window));
    assert!(history.window_ending_at(5 * window_samples, &mut window));

    history.clear();
    assert!(!history.window_ending_at(1, &mut window));
}

// A sink whose position follows rendered data, like PipeWire-Pulse and
// Bluetooth sinks: it holds `hold_ms` of PCM before rendering anything and then
// takes `block_us` per 33-ms block.
fn model_sink(hold_ms: u64, block_us: u32) -> String {
    format!(
        "queue min-threshold-time={} max-size-time={} max-size-bytes=0 max-size-buffers=0 \
         ! identity name=consumer sleep-time={block_us} ! fakesink sync=false",
        hold_ms * 1_000_000,
        (hold_ms + 200) * 1_000_000
    )
}

fn open_with(
    header: Header,
    sink: String,
    clock: Option<gst::Clock>,
    retain_audio: bool,
) -> DecodedMedia {
    let media = DecodedMedia::new(SandboxedMedia {
        audio_only: header.width == 0,
        ..test_source("/model-sink")
    });
    if retain_audio {
        media.retain_played_audio();
    }
    media.imp().audio_sink.replace(Some(sink));
    media.imp().audio_clock.replace(clock);
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
    // The first tick also initialises GStreamer, which must not land in a timed window.
    let deadline = Instant::now() + Duration::from_secs(15);
    while !media.imp().first_frame.get() {
        media.tick().expect("tick succeeds");
        assert!(Instant::now() < deadline, "first frame deadline");
        std::thread::sleep(Duration::from_millis(1));
    }
    media
}

fn open_on(sink: String, header: Header) -> DecodedMedia {
    open_with(header, sink, None, false)
}

fn audio_header(edge: u32) -> Header {
    Header {
        width: edge,
        height: edge,
        audio: true,
        ..test_header(0)
    }
}

struct Playback {
    shown: u64,
    heard: Option<u64>,
}

/// Ticks once; the playhead may neither lead the sink nor jump back.
fn observe(media: &DecodedMedia, last: &mut u64) -> Playback {
    media.tick().expect("tick succeeds");
    let shown = media.timestamp() as u64;
    let heard = media
        .imp()
        .audio
        .borrow()
        .as_ref()
        .and_then(PcmOutput::position_us);
    assert!(
        shown + 40_000 >= *last,
        "playhead rewound from {last} to {shown}"
    );
    if let Some(heard) = heard {
        assert!(
            shown <= heard + 100_000,
            "playhead {shown} ran ahead of the sink at {heard}"
        );
    }
    *last = shown;
    std::thread::sleep(Duration::from_millis(1));
    Playback { shown, heard }
}

#[test]
fn audio_and_video_play_through_a_sink_that_starts_late() {
    crate::test_support::gtk_test(
        "ui::media::tests::audio_and_video_play_through_a_sink_that_starts_late",
        late_sink_playback,
    );
}

fn late_sink_playback() {
    // 600 ms is far more than the three-frame presentation queue holds.
    for (edge, depth) in [
        (0, PRESENTATION_QUEUE + 1..=usize::MAX),
        (16, 0..=PRESENTATION_QUEUE),
    ] {
        let media = open_on(model_sink(600, 33_333), audio_header(edge));
        let imp = media.imp();
        let deadline = Instant::now() + Duration::from_secs(10);
        let (mut last, mut deepest) = (0, 0);
        while observe(&media, &mut last).shown < 900_000 {
            deepest = deepest.max(imp.frames.borrow().len());
            assert!(
                Instant::now() < deadline,
                "{edge}px: late sink playback deadline"
            );
        }
        assert_eq!(
            imp.recoveries.get(),
            0,
            "{edge}px: a late start is not a stall"
        );
        assert!(
            depth.contains(&deepest),
            "{edge}px: queued {deepest} frames"
        );
        media.close();
    }
}

#[test]
fn a_sink_slower_than_the_stuck_timeout_is_not_a_stall() {
    crate::test_support::gtk_test(
        "ui::media::tests::a_sink_slower_than_the_stuck_timeout_is_not_a_stall",
        slow_start_is_not_a_stall,
    );
}

fn slow_start_is_not_a_stall() {
    let clock = ManualClock::new();
    let media = open_with(
        audio_header(0),
        "fakesink sync=true".into(),
        Some(clock.clone().upcast()),
        false,
    );
    let imp = media.imp();
    // The sink has not run yet, so the stuck timer must not count this wait.
    let until = Instant::now() + AUDIO_STUCK_TIMEOUT + Duration::from_millis(300);
    let mut last = 0;
    while Instant::now() < until {
        assert_eq!(
            observe(&media, &mut last).shown,
            0,
            "playhead moved before the sink"
        );
        assert_eq!(
            imp.recoveries.get(),
            0,
            "waiting for the sink is not a stall"
        );
    }
    let started = Instant::now();
    let deadline = started + Duration::from_secs(10);
    loop {
        clock.set_time(started.elapsed().as_micros() as u64);
        if observe(&media, &mut last).shown >= 100_000 {
            break;
        }
        assert!(Instant::now() < deadline, "manual clock playback deadline");
    }
    assert_eq!(imp.recoveries.get(), 0);
    media.close();
}

#[test]
fn a_mid_play_stall_keeps_the_samples_the_sink_will_still_play() {
    crate::test_support::gtk_test(
        "ui::media::tests::a_mid_play_stall_keeps_the_samples_the_sink_will_still_play",
        stall_keeps_unplayed_samples,
    );
}

fn stall_keeps_unplayed_samples() {
    let clock = ManualClock::new();
    let media = open_with(
        audio_header(0),
        "fakesink sync=true".into(),
        Some(clock.clone().upcast()),
        true,
    );
    let imp = media.imp();
    let deadline = Instant::now() + Duration::from_secs(10);
    let (started, mut last) = (Instant::now(), 0);
    loop {
        clock.set_time(started.elapsed().as_micros() as u64);
        if observe(&media, &mut last).shown >= 300_000 {
            break;
        }
        assert!(Instant::now() < deadline, "manual clock playback deadline");
    }
    // Freeze the sink for longer than the retained second while the playhead runs on.
    let base = imp.clock_base.get();
    let until = Instant::now() + Duration::from_millis(1_500);
    while Instant::now() < until {
        media.tick().expect("tick succeeds");
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(imp.recoveries.get(), 0);
    let mut window = [0.0; 4];
    let kept = imp.history.borrow().as_ref().is_some_and(|history| {
        history.window_ending_at(base * media::SAMPLE_RATE / 1_000_000 + 4, &mut window)
    });
    assert!(kept, "samples at the stalled sink position were trimmed");
    media.close();
}

#[test]
fn resume_waits_for_the_sink_instead_of_leading_and_snapping_back() {
    crate::test_support::gtk_test(
        "ui::media::tests::resume_waits_for_the_sink_instead_of_leading_and_snapping_back",
        resume_waits_for_the_sink,
    );
}

fn resume_waits_for_the_sink() {
    let clock = ManualClock::new();
    let media = open_with(
        audio_header(0),
        "fakesink sync=true".into(),
        Some(clock.clone().upcast()),
        false,
    );
    let imp = media.imp();
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut last = 0;
    let hold = |last: &mut u64, base: u64| {
        let until = Instant::now() + Duration::from_millis(150);
        let mut shown = None;
        let mut ticks = 0;
        while Instant::now() < until {
            let playback = observe(&media, last);
            assert!(playback.heard.is_none_or(|heard| heard <= base));
            let shown = *shown.get_or_insert(playback.shown);
            assert_eq!(playback.shown, shown, "playhead moved before the sink");
            ticks += 1;
        }
        assert!(ticks > 10, "held for only {ticks} ticks");
        shown.expect("observed playback")
    };
    let run = |last: &mut u64, from: u64, until_shown: u64| {
        let started = Instant::now();
        loop {
            clock.set_time(from + started.elapsed().as_micros() as u64);
            if observe(&media, last).shown >= until_shown {
                break;
            }
            assert!(Instant::now() < deadline, "manual clock playback deadline");
        }
    };

    assert_eq!(hold(&mut last, 0), 0, "playback starts where the sink is");
    run(&mut last, 0, 300_000);

    media.pause();
    let paused_at = imp.position.get();
    let base = imp.clock_base.get();
    media.play();
    let resumed_at = hold(&mut last, base);
    assert!(
        resumed_at <= paused_at && paused_at - resumed_at <= 40_000,
        "resumed at {resumed_at} after pausing at {paused_at}"
    );
    run(&mut last, base, base + 300_000);
    assert_eq!(imp.recoveries.get(), 0);
    media.close();
}

mod manual_clock {
    use super::*;
    use gst::subclass::prelude::*;
    use std::sync::{Condvar, Mutex};

    /// A pipeline clock that only moves when a test sets its time.
    #[derive(Default)]
    pub struct Imp {
        time: Mutex<(u64, u64)>,
        changed: Condvar,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Imp {
        const NAME: &'static str = "StrataManualClock";
        type Type = ManualClock;
        type ParentType = gst::Clock;
    }

    impl ObjectImpl for Imp {}
    impl GstObjectImpl for Imp {}

    impl ClockImpl for Imp {
        fn internal_time(&self) -> gst::ClockTime {
            gst::ClockTime::from_nseconds(self.time.lock().expect("clock").0)
        }

        fn wait(
            &self,
            id: &gst::ClockId,
        ) -> (
            Result<gst::ClockSuccess, gst::ClockError>,
            gst::ClockTimeDiff,
        ) {
            let target = id.time().nseconds();
            let mut time = self.time.lock().expect("clock");
            let generation = time.1;
            while time.0 < target && time.1 == generation {
                time = self.changed.wait(time).expect("clock");
            }
            if time.1 != generation {
                return (Err(gst::ClockError::Unscheduled), 0);
            }
            (Ok(gst::ClockSuccess::Ok), time.0 as i64 - target as i64)
        }

        // The sink is the only waiter, so any unschedule releases it.
        fn unschedule(&self, _id: &gst::ClockId) {
            self.time.lock().expect("clock").1 += 1;
            self.changed.notify_all();
        }
    }

    glib::wrapper! {
        pub struct ManualClock(ObjectSubclass<Imp>) @extends gst::Clock, gst::Object;
    }

    impl ManualClock {
        pub fn new() -> Self {
            glib::Object::new()
        }

        pub fn set_time(&self, micros: u64) {
            let imp = self.imp();
            imp.time.lock().expect("clock").0 = micros * 1_000;
            imp.changed.notify_all();
        }
    }
}
use manual_clock::ManualClock;

#[test]
fn sessions_start_after_the_dwell_and_never_for_a_replaced_selection() {
    crate::test_support::gtk_test(
        "ui::media::tests::sessions_start_after_the_dwell_and_never_for_a_replaced_selection",
        || {
            let media = DecodedMedia::new(test_source("/replaced"));
            let calls = Rc::new(RefCell::new(Vec::new()));
            media.imp().loader.replace(Some(fake_loader(&calls)));
            let created = Instant::now();
            while created.elapsed() < START_DWELL / 2 {
                media.tick().expect("tick succeeds");
                std::thread::sleep(Duration::from_millis(1));
            }
            assert!(calls.borrow().is_empty(), "no decoder inside the dwell");
            media.close();
            std::thread::sleep(START_DWELL);
            let _ = media.tick();
            assert!(
                calls.borrow().is_empty(),
                "a selection replaced within the dwell never spawns a decoder"
            );

            let media = DecodedMedia::new(test_source("/kept"));
            let calls = Rc::new(RefCell::new(Vec::new()));
            media.imp().loader.replace(Some(fake_loader(&calls)));
            let created = Instant::now();
            drive_until(&media, &calls, 1);
            assert!(created.elapsed() >= START_DWELL);
            assert_eq!(*calls.borrow(), vec![0]);
            media.close();
        },
    );
}

#[test]
fn prepared_streams_size_the_frame_before_it_is_decoded() {
    crate::test_support::gtk_test(
        "ui::media::tests::prepared_streams_size_the_frame_before_it_is_decoded",
        || {
            let media = DecodedMedia::new(test_source("/sized"));
            assert_eq!(media.video_size(), None);
            assert_eq!(media.intrinsic_width(), 0);
            let calls = Rc::new(RefCell::new(Vec::new()));
            media.imp().loader.replace(Some(fake_loader(&calls)));
            let deadline = Instant::now() + Duration::from_secs(15);
            while !media.is_prepared() {
                media.tick().expect("tick succeeds");
                assert!(Instant::now() < deadline, "prepare deadline");
                std::thread::sleep(Duration::from_millis(1));
            }
            assert_eq!(media.video_size(), Some((16, 16)));
            assert_eq!(
                (media.intrinsic_width(), media.intrinsic_height()),
                (16, 16)
            );
            media.close();
        },
    );
}
