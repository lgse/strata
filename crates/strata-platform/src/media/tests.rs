// SPDX-License-Identifier: MIT

use std::{
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use super::{
    BackendKind, MAX_VOLUME, OutputConfig, OutputError, OutputEvent, PcmOutput, PcmSpec,
    SampleFormat,
    backend::AudioBackend,
    ring::{PcmRing, UnheardTail, apply_gain, sanitize_volume},
};

const STEREO_48K_S16: PcmSpec = PcmSpec::new(SampleFormat::S16Le, 48_000, 2);
const STEREO_48K_F32: PcmSpec = PcmSpec::new(SampleFormat::F32Le, 48_000, 2);

struct MockState {
    written: Vec<u8>,
    flushes: usize,
    latency_us: u64,
    latency_script: Vec<u64>,
    latency_calls: usize,
    fail_after_writes: Option<usize>,
    writes: usize,
}

#[derive(Clone)]
struct MockHandle {
    state: Arc<(Mutex<MockState>, Condvar)>,
}

impl MockHandle {
    fn new(latency_us: u64) -> (Self, MockBackend) {
        let state = Arc::new((
            Mutex::new(MockState {
                written: Vec::new(),
                flushes: 0,
                latency_us,
                latency_script: Vec::new(),
                latency_calls: 0,
                fail_after_writes: None,
                writes: 0,
            }),
            Condvar::new(),
        ));
        (
            Self {
                state: Arc::clone(&state),
            },
            MockBackend { state },
        )
    }

    fn set_latency_script(&self, script: Vec<u64>) {
        let mut guard = self.state.0.lock().unwrap();
        guard.latency_script = script;
        guard.latency_calls = 0;
    }

    fn set_fail_after_writes(&self, count: usize) {
        self.state.0.lock().unwrap().fail_after_writes = Some(count);
    }

    fn set_latency_us(&self, us: u64) {
        self.state.0.lock().unwrap().latency_us = us;
    }

    fn wait_for(&self, timeout: Duration, predicate: impl Fn(&MockState) -> bool) -> bool {
        let deadline = Instant::now() + timeout;
        let mut guard = self.state.0.lock().unwrap();
        while !predicate(&guard) {
            let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                return false;
            };
            let (next, _) = self.state.1.wait_timeout(guard, remaining).unwrap();
            guard = next;
        }
        true
    }

    fn written_s16(&self) -> Vec<i16> {
        let guard = self.state.0.lock().unwrap();
        guard
            .written
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| i16::from_le_bytes(*c))
            .collect()
    }

    fn flushes(&self) -> usize {
        self.state.0.lock().unwrap().flushes
    }
}

struct MockBackend {
    state: Arc<(Mutex<MockState>, Condvar)>,
}

impl AudioBackend for MockBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::PulseAudio
    }

    fn write(&mut self, bytes: &[u8]) -> Result<(), ()> {
        let mut guard = self.state.0.lock().unwrap();
        if guard
            .fail_after_writes
            .is_some_and(|limit| guard.writes >= limit)
        {
            return Err(());
        }
        guard.writes += 1;
        guard.written.extend_from_slice(bytes);
        self.state.1.notify_all();
        Ok(())
    }

    fn latency_us(&mut self) -> Option<u64> {
        let mut guard = self.state.0.lock().unwrap();
        if !guard.latency_script.is_empty() {
            let idx = guard.latency_calls.min(guard.latency_script.len() - 1);
            guard.latency_calls += 1;
            return Some(guard.latency_script[idx]);
        }
        Some(guard.latency_us)
    }

    fn flush(&mut self) -> Result<(), ()> {
        let mut guard = self.state.0.lock().unwrap();
        guard.flushes += 1;
        self.state.1.notify_all();
        Ok(())
    }
}

fn wait_until(timeout: Duration, predicate: impl Fn() -> bool) -> bool {
    let deadline = Instant::now() + timeout;
    while !predicate() {
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    true
}

#[test]
fn ring_wraparound_backpressure_and_rewind() {
    let mut ring = PcmRing::new(18, 4); // rounded down to 16 bytes (4 frames)
    assert_eq!(ring.capacity(), 16);
    assert_eq!(ring.free(), 16);

    assert!(ring.push(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]));
    assert_eq!(ring.len(), 12);
    assert_eq!(ring.free(), 4);
    // Rejecting an oversized push leaves the ring unchanged.
    assert!(!ring.push(&[0; 8]));
    assert_eq!(ring.len(), 12);

    let mut out = [0u8; 8];
    assert_eq!(ring.pop_into(&mut out, 4), 8);
    assert_eq!(out, [1, 2, 3, 4, 5, 6, 7, 8]);
    // Wrap around the end of the internal buffer.
    assert!(ring.push(&[13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24]));
    assert_eq!(ring.len(), 16);
    assert_eq!(ring.free(), 0);

    // Rewind prefix pops ahead of ring contents even when ring is full.
    ring.push_front(&[99, 98, 97, 96]);
    let mut all = [0u8; 20];
    assert_eq!(ring.pop_into(&mut all, 4), 20);
    assert_eq!(
        all,
        [
            99, 98, 97, 96, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24
        ]
    );
    assert!(ring.is_empty());

    ring.push(&[1, 2, 3, 4]);
    ring.clear();
    assert!(ring.is_empty());
    assert_eq!(ring.free(), 16);
}

#[test]
fn unheard_tail_keeps_latest_bytes_and_takes_whole_frames() {
    let mut tail = UnheardTail::new(12);
    tail.record(&[1, 2, 3, 4, 5, 6, 7, 8]);
    tail.record(&[9, 10, 11, 12, 13, 14, 15, 16]);
    // Bounded to last 12 bytes: [5..=16]. Asking for 10 bytes rounds down to 8 (2 frames).
    let restored = tail.take_trailing(10, 4);
    assert_eq!(restored, vec![9, 10, 11, 12, 13, 14, 15, 16]);
    assert!(tail.take_trailing(4, 4).is_empty());
}

#[test]
fn gain_scaling_for_s16_and_f32_and_volume_sanitization() {
    assert_eq!(sanitize_volume(f32::NAN), 0.0);
    assert_eq!(sanitize_volume(f32::INFINITY), 0.0);
    assert_eq!(sanitize_volume(-0.5), 0.0);
    assert_eq!(sanitize_volume(5.0), MAX_VOLUME);

    // S16Le: half gain, mute, and saturating boost.
    let samples = [1000_i16, -2000_i16, 20_000_i16, -20_000_i16];
    let mut half: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
    apply_gain(SampleFormat::S16Le, &mut half, 0.5);
    let half_s16: Vec<i16> = half
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| i16::from_le_bytes(*c))
        .collect();
    assert_eq!(half_s16, vec![500, -1000, 10_000, -10_000]);

    let mut boosted: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
    apply_gain(SampleFormat::S16Le, &mut boosted, 2.0);
    let boosted_s16: Vec<i16> = boosted
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| i16::from_le_bytes(*c))
        .collect();
    assert_eq!(boosted_s16, vec![2000, -4000, i16::MAX, i16::MIN]);

    apply_gain(SampleFormat::S16Le, &mut boosted, 0.0);
    assert!(boosted.iter().all(|&b| b == 0));

    // F32Le: half gain, clamp, and non-finite guard.
    let f32_samples = [0.5_f32, -0.8_f32, 0.9_f32, f32::NAN];
    let mut f32_bytes: Vec<u8> = f32_samples.iter().flat_map(|s| s.to_le_bytes()).collect();
    apply_gain(SampleFormat::F32Le, &mut f32_bytes, 1.5);
    let scaled: Vec<f32> = f32_bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|c| f32::from_le_bytes(*c))
        .collect();
    assert!((scaled[0] - 0.75).abs() < 1e-6);
    assert!((scaled[1] + 1.0).abs() < 1e-6);
    assert!((scaled[2] - 1.0).abs() < 1e-6);
    assert_eq!(scaled[3], 0.0);
}

#[test]
fn validates_spec_alignment_backpressure_and_finished_state() {
    let invalid = OutputConfig::new(PcmSpec::new(SampleFormat::S16Le, 0, 2)).silent_only(true);
    assert!(matches!(
        PcmOutput::open(invalid),
        Err(OutputError::InvalidSpec)
    ));

    let output = PcmOutput::open(
        OutputConfig::new(STEREO_48K_S16)
            .buffer_duration(Duration::from_millis(10))
            .silent_only(true),
    )
    .unwrap();
    // 3 bytes is not a multiple of 4 (stereo S16Le frame).
    assert!(matches!(
        output.push(&[0; 3]),
        Err(OutputError::UnalignedChunk)
    ));
    // Mismatched convenience format.
    assert!(matches!(
        output.push_f32(&[0.0, 0.0]),
        Err(OutputError::UnalignedChunk)
    ));
    // Fill buffer to capacity while paused, then confirm backpressure rejects overflow.
    let cap = output.capacity_bytes();
    assert!(output.has_capacity(cap));
    output.push(&vec![0u8; cap]).unwrap();
    assert!(!output.has_capacity(4));
    assert!(matches!(output.push(&[0; 4]), Err(OutputError::BufferFull)));

    // Flushing clears buffer and restores capacity.
    output.flush();
    assert_eq!(output.buffered_bytes(), 0);
    assert_eq!(output.free_bytes(), cap);

    // Pushing after finish() returns Finished until flush().
    output.finish();
    assert!(matches!(output.push(&[0; 4]), Err(OutputError::Finished)));
    output.flush();
    assert!(!output.is_finished());
    output.push(&[0; 4]).unwrap();
}

#[test]
fn mock_backend_tracks_latency_pause_rewind_flush_volume_and_drain() {
    let (mock, backend) = MockHandle::new(20_000); // 20 ms simulated device latency (960 frames)
    let drained_events = Arc::new(AtomicUsize::new(0));
    let drained_cb = Arc::clone(&drained_events);

    let output = PcmOutput::open_with_backend(
        OutputConfig::new(STEREO_48K_S16)
            .buffer_duration(Duration::from_millis(200))
            .volume(0.5)
            .muted(false)
            .on_event(move |ev| {
                if ev == OutputEvent::Drained {
                    drained_cb.fetch_add(1, Ordering::SeqCst);
                }
            }),
        move || Box::new(backend),
    )
    .unwrap();

    // Push 40 ms (1920 frames) of constant amplitude 1000.
    let chunk: Vec<i16> = vec![1000; 1920 * 2];
    output.push_s16(&chunk).unwrap();
    output.play();
    assert!(mock.wait_for(Duration::from_secs(2), |s| s.written.len() >= 1920 * 4));

    // Scaled by 0.5 -> samples written to device are 500.
    let written = mock.written_s16();
    assert_eq!(written.len(), 1920 * 2);
    assert!(written.iter().all(|&s| s == 500));

    // Reported position accounts for the 20 ms (960 frames) device latency.
    let pos_frames = output.position_frames();
    assert!(
        (960..=1920).contains(&pos_frames),
        "expected latency-adjusted position in 960..=1920, got {pos_frames}"
    );

    // Pause flushes backend and freezes position.
    output.pause();
    assert!(mock.wait_for(Duration::from_secs(2), |s| s.flushes >= 1));
    let paused_pos = output.position_frames();
    std::thread::sleep(Duration::from_millis(20));
    assert_eq!(output.position_frames(), paused_pos);

    // Seek via flush resets position to 0 and clears queued audio.
    output.flush();
    assert_eq!(output.position_frames(), 0);
    assert!(mock.wait_for(Duration::from_secs(2), |s| s.flushes >= 2));

    // Live mute produces silence and finish() drains to EOF.
    output.set_muted(true);
    mock.set_latency_us(0);
    let before_len = mock.written_s16().len();
    output.push_s16(&vec![2000; 480 * 2]).unwrap();
    output.finish();
    output.play();

    assert!(wait_until(Duration::from_secs(2), || output.is_drained()));
    assert_eq!(drained_events.load(Ordering::SeqCst), 1);
    assert_eq!(output.position_frames(), 480);
    let after = mock.written_s16();
    assert!(after[before_len..].iter().all(|&s| s == 0));
    assert!(mock.flushes() >= 2);
}

#[test]
fn device_loss_mid_playback_degrades_cleanly_to_silent_backend() {
    let (mock, backend) = MockHandle::new(0);
    // Fail on the second quantum write.
    mock.set_fail_after_writes(1);

    let lost = Arc::new(AtomicBool::new(false));
    let lost_cb = Arc::clone(&lost);
    let drained = Arc::new(AtomicBool::new(false));
    let drained_cb = Arc::clone(&drained);

    let output = PcmOutput::open_with_backend(
        OutputConfig::new(STEREO_48K_F32)
            .buffer_duration(Duration::from_millis(200))
            .on_event(move |ev| match ev {
                OutputEvent::DeviceLost => lost_cb.store(true, Ordering::SeqCst),
                OutputEvent::Drained => drained_cb.store(true, Ordering::SeqCst),
                OutputEvent::Underrun => {}
            }),
        move || Box::new(backend),
    )
    .unwrap();

    // 30 ms of f32 audio (3 quanta of 10 ms): quantum 1 hits mock, quantum 2 triggers failover.
    output.push_f32(&vec![0.25_f32; 1440 * 2]).unwrap();
    output.finish();
    output.play();

    assert!(wait_until(Duration::from_secs(2), || output.is_drained()));
    assert!(output.device_lost());
    assert_eq!(output.backend(), BackendKind::Silent);
    assert!(lost.load(Ordering::SeqCst));
    assert!(drained.load(Ordering::SeqCst));
    assert_eq!(output.position_frames(), 1440);
}

#[test]
fn underrun_notifies_once_and_recovers_when_data_arrives() {
    let (_mock, backend) = MockHandle::new(0);
    let underruns = Arc::new(AtomicU64::new(0));
    let underruns_cb = Arc::clone(&underruns);

    let output = PcmOutput::open_with_backend(
        OutputConfig::new(STEREO_48K_S16)
            .buffer_duration(Duration::from_millis(100))
            .on_event(move |ev| {
                if ev == OutputEvent::Underrun {
                    underruns_cb.fetch_add(1, Ordering::SeqCst);
                }
            }),
        move || Box::new(backend),
    )
    .unwrap();

    // Playing with an empty ring triggers one Underrun notification.
    output.play();
    assert!(wait_until(Duration::from_secs(2), || {
        underruns.load(Ordering::SeqCst) == 1
    }));
    std::thread::sleep(Duration::from_millis(25));
    assert_eq!(
        underruns.load(Ordering::SeqCst),
        1,
        "underrun should not spam while continuously starved"
    );

    // Pushing new audio and finishing drains cleanly.
    output.push_s16(&vec![0; 480 * 2]).unwrap();
    output.finish();
    assert!(wait_until(Duration::from_secs(2), || output.is_drained()));
    assert_eq!(output.position_frames(), 480);
}

#[test]
fn silent_fallback_paces_clock_and_tears_down_cleanly() {
    let output = PcmOutput::open(
        OutputConfig::new(STEREO_48K_S16)
            .buffer_duration(Duration::from_millis(200))
            .silent_only(true),
    )
    .unwrap();

    // Push 60 ms (2880 frames) of PCM and play on the real-time silent fallback.
    let pcm = vec![0u8; 2880 * 4];
    output.push(&pcm).unwrap();
    output.finish();
    let started = Instant::now();
    output.play();

    assert!(wait_until(Duration::from_secs(2), || output.is_drained()));
    let elapsed = started.elapsed();
    assert_eq!(output.backend(), BackendKind::Silent);
    assert_eq!(output.position(), Duration::from_millis(60));
    assert!(
        elapsed >= Duration::from_millis(40),
        "silent fallback should pace in real time, drained in {elapsed:?}"
    );
    drop(output);
}

#[test]
fn position_stays_monotonic_under_constant_and_jittering_latency() {
    let (mock, backend) = MockHandle::new(40_000);
    // Latency jumps up and down across writes and stays non-zero during drain ticks.
    mock.set_latency_script(vec![
        15_000, 45_000, 20_000, 55_000, 40_000, 40_000, 40_000, 40_000,
    ]);

    let output = PcmOutput::open_with_backend(
        OutputConfig::new(STEREO_48K_S16).buffer_duration(Duration::from_millis(300)),
        move || Box::new(backend),
    )
    .unwrap();

    // Push 60 ms (6 quanta of 480 frames), finish, and sample position continuously while draining.
    let total_frames = 2880u64;
    output
        .push_s16(&vec![0; (total_frames as usize) * 2])
        .unwrap();
    output.finish();
    output.play();

    let mut prev = 0u64;
    let deadline = Instant::now() + Duration::from_secs(2);
    while !output.is_drained() {
        assert!(Instant::now() < deadline, "timed out waiting for drain");
        let now = output.position_frames();
        assert!(
            now >= prev,
            "playback position moved backward under jittering latency: {prev} -> {now}"
        );
        prev = now;
        std::thread::sleep(Duration::from_millis(2));
    }
    let final_pos = output.position_frames();
    assert!(final_pos >= prev);
    assert_eq!(final_pos, total_frames);
}

#[test]
fn flush_wakes_starved_underrun_worker_immediately() {
    let (mock, backend) = MockHandle::new(0);
    let output = PcmOutput::open_with_backend(
        OutputConfig::new(STEREO_48K_S16).buffer_duration(Duration::from_millis(100)),
        move || Box::new(backend),
    )
    .unwrap();

    // Enter idle underrun wait, then verify flush wakes the worker and flushes the backend.
    output.play();
    std::thread::sleep(Duration::from_millis(15));
    output.flush();
    assert!(mock.wait_for(Duration::from_secs(1), |s| s.flushes >= 1));
    drop(output);
}
