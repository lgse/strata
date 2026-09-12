// SPDX-License-Identifier: MIT

use super::*;
use std::time::{Duration, Instant};

fn output() -> PcmOutput {
    gst::init().expect("GStreamer must be available");
    let sink = gst::ElementFactory::make("fakesink")
        .property("sync", true)
        .build()
        .expect("fakesink must be available");
    PcmOutput::with_sink(sink, false, 1.0).expect("raw audio pipeline")
}

#[test]
fn audio_rejects_invalid_samples_and_timestamps() {
    let output = output();
    for length in [0, 1, 3, 5, MAX_CHUNK_BYTES + FRAME_BYTES] {
        assert!(output.push(vec![0; length], 0).is_err());
    }
    assert!(output.push(vec![0; 4], 1).is_err());
    output
        .push(vec![0; MAX_CHUNK_BYTES], 0)
        .expect("first chunk");
    assert!(output.push(vec![0; 4], 0).is_err());
    assert!(output.push(vec![0; 4], 33_334).is_err());
    assert!(output.push(vec![0; 4], u64::MAX).is_err());
    output
        .push(vec![0; MAX_CHUNK_BYTES], 33_333)
        .expect("second chunk");
    output
        .push(vec![0; 4], 66_666)
        .expect("partial final chunk");
    assert!(output.error().is_none());
}

#[test]
fn audio_keeps_sample_exact_timing_past_thirty_seconds_and_for_long_sources() {
    gst::init().expect("GStreamer must be available");
    for seconds in [30, 3600, crate::media::MAX_DURATION_US / 1_000_000 - 1] {
        let sink = gstreamer_app::AppSink::builder().sync(false).build();
        let output = PcmOutput::with_sink(sink.clone().upcast(), false, 1.0).expect("audio output");
        let frames = seconds * SAMPLE_RATE - 1;
        output.frames.set(frames);
        let timestamp = frames * 1_000_000 / SAMPLE_RATE;
        output
            .push(vec![0; 8], timestamp)
            .expect("samples across boundary");
        output.play().expect("play");
        let sample = sink
            .try_pull_sample(gst::ClockTime::from_seconds(2))
            .expect("decoded samples");
        let buffer = sample.buffer().expect("audio buffer");
        let start_ns = (u128::from(frames) * 1_000_000_000 / u128::from(SAMPLE_RATE)) as u64;
        let end_ns = (u128::from(frames + 2) * 1_000_000_000 / u128::from(SAMPLE_RATE)) as u64;
        assert_eq!(buffer.pts().expect("PTS").nseconds(), start_ns);
        assert_eq!(
            buffer.duration().expect("duration").nseconds(),
            end_ns - start_ns
        );
        output
            .push(vec![0; 4], (frames + 2) * 1_000_000 / SAMPLE_RATE)
            .expect("next sample");
        output.finish().expect("EOS");
        assert!(!output.has_capacity());
    }
    for frames in [
        u64::MAX,
        (u128::from(u64::MAX) * u128::from(SAMPLE_RATE) / 1_000_000_000) as u64,
    ] {
        let output = output();
        output.frames.set(frames);
        let timestamp = (u128::from(frames) * 1_000_000 / u128::from(SAMPLE_RATE))
            .min(u128::from(u64::MAX)) as u64;
        assert!(output.push(vec![0; 8], timestamp).is_err());
        assert_eq!(output.source.current_level_bytes(), 0);
    }
}

#[test]
fn audio_queue_is_bounded_and_finish_is_terminal() {
    let output = output();
    let mut chunks = 0;
    while output.has_capacity() {
        assert!(chunks < 10, "appsrc capacity must be bounded");
        output
            .push(
                vec![0; MAX_CHUNK_BYTES],
                chunks * 1_600 * 1_000_000 / SAMPLE_RATE,
            )
            .expect("bounded chunk");
        chunks += 1;
    }
    assert!(chunks > 0);
    assert!(output.source.current_level_bytes() <= MAX_BYTES);
    assert!(output.source.current_level_time().nseconds() <= MAX_TIME_NS);
    output.finish().expect("EOS");
    output.finish().expect("idempotent EOS");
    assert!(!output.has_capacity());
    assert!(
        output
            .push(vec![0; 4], output.frames.get() * 1_000_000 / SAMPLE_RATE)
            .is_err()
    );
}

#[test]
fn audio_mute_and_volume_are_applied_immediately() {
    let output = PcmOutput::new(true, 0.25).expect("muted output");
    assert!(output.volume.property::<bool>("mute"));
    assert_eq!(output.volume.property::<f64>("volume"), 0.25);
    output.set_audio(false, 0.75);
    assert!(!output.volume.property::<bool>("mute"));
    assert_eq!(output.volume.property::<f64>("volume"), 0.75);
    for (input, expected) in [(2.0, 1.0), (-1.0, 0.0), (f64::NAN, 0.0)] {
        output.set_audio(false, input);
        assert_eq!(output.volume.property::<f64>("volume"), expected);
    }
}

fn wait_for_position(output: &PcmOutput, minimum: u64) -> u64 {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        assert!(output.error().is_none());
        if let Some(position) = output.position_us().filter(|position| *position >= minimum) {
            return position;
        }
        assert!(
            Instant::now() < deadline,
            "sink playback clock did not advance"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn audio_sink_clock_pauses_and_resumes() {
    let output = output();
    for chunk in 0..5 {
        output
            .push(
                vec![0; MAX_CHUNK_BYTES],
                chunk * 1_600 * 1_000_000 / SAMPLE_RATE,
            )
            .expect("preroll chunk");
    }
    output.play().expect("play");
    wait_for_position(&output, 10_000);
    output.pause().expect("pause");
    let (result, state, _) = output.pipeline.state(gst::ClockTime::from_seconds(3));
    result.expect("pause transition");
    assert_eq!(state, gst::State::Paused);
    let paused = output.position_us().expect("paused position");
    std::thread::sleep(Duration::from_millis(30));
    assert_eq!(output.position_us(), Some(paused));
    output.play().expect("resume");
    wait_for_position(&output, paused + 5_000);
}

#[test]
fn audio_drop_explicitly_stops_pipeline_repeatedly() {
    for _ in 0..12 {
        let output = output();
        output.push(vec![0; MAX_CHUNK_BYTES], 0).expect("preroll");
        output.play().expect("play");
        let pipeline = output.pipeline.clone();
        drop(output);
        let (result, state, _) = pipeline.state(gst::ClockTime::from_seconds(3));
        result.expect("teardown transition");
        assert_eq!(state, gst::State::Null);
    }
}
