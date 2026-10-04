// SPDX-License-Identifier: MIT

use super::*;
use crate::media::Frame;

pub(crate) fn stream(header: Header) -> Result<Session, String> {
    stream_to(header, header.duration_us)
}

pub(crate) fn stream_to(header: Header, end: u64) -> Result<Session, String> {
    let slot = WorkerSlot::acquire().ok_or("Media previews are busy (four active players)")?;
    let slot = Arc::new(slot);
    let worker_slot = slot.clone();
    let cancellation = Cancellation::default();
    let cancelled = cancellation.clone();
    let (sender, receiver) = mpsc::sync_channel(QUEUED_PACKETS);
    let worker = thread::spawn(move || {
        let _slot = worker_slot;
        if send(&sender, Event::Prepared(header), &cancelled).is_err() {
            return;
        }
        let end_tick = Header {
            duration_us: end,
            ..header
        }
        .ticks();
        for tick in header.start_tick..end_tick {
            let frame = Frame {
                tick,
                pixels: vec![(tick % 255) as u8; header.video_bytes()],
                samples: vec![0; crate::media::audio_bytes(header, tick)],
            };
            if send(&sender, Event::Packet(Packet::Frame(frame)), &cancelled).is_err() {
                return;
            }
        }
        let _sent = send(&sender, Event::Packet(Packet::End(end)), &cancelled);
    });
    Ok(Session {
        cancellation,
        receiver,
        worker,
        _slot: slot,
    })
}

fn wait_for(condition: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while !condition() {
        assert!(Instant::now() < deadline, "worker teardown deadline");
        thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn four_slots_backpressure_cancellation_and_repeated_teardown_are_bounded() {
    let h = Header {
        width: 16,
        height: 16,
        audio: true,
        duration_us: 60_000_000,
        start_tick: 0,
    };
    for _ in 0..30 {
        let workers: Vec<_> = (0..MAX_WORKERS)
            .map(|_| stream(h).expect("available worker"))
            .collect();
        assert!(stream(h).is_err());
        thread::sleep(Duration::from_millis(20));
        for worker in &workers {
            assert!(
                !worker.finished(),
                "backpressure must stop whole-clip decoding"
            );
            worker.cancel();
        }
        wait_for(|| workers.iter().all(Session::finished));
        for worker in &workers {
            assert!(worker.receiver.try_iter().count() <= QUEUED_PACKETS);
            assert!(matches!(worker.receive(), Some(Event::Failed(_))));
        }
        drop(workers);
        assert_eq!(ACTIVE_WORKERS.load(Ordering::Acquire), 0);
    }
    let workers: Vec<_> = (0..MAX_WORKERS)
        .map(|_| {
            stream(Header {
                duration_us: 33_333,
                ..h
            })
            .expect("available worker")
        })
        .collect();
    wait_for(|| workers.iter().all(Session::finished));
    assert!(
        stream(h).is_err(),
        "buffered playback retains its slot between GIF loops"
    );
    drop(workers);
    assert_eq!(ACTIVE_WORKERS.load(Ordering::Acquire), 0);
}

#[test]
fn decoder_failure_and_trailing_output_are_not_successful_end_of_stream() {
    let source = SandboxedMedia {
        path: "/unused".into(),
        size: crate::services::MediaPreviewSize::new(16, 16),
        backend: MediaPreviewBackend::Software,
        input_owner: None,
        audio_only: false,
    };
    let h = Header {
        width: 1,
        height: 1,
        audio: false,
        duration_us: 33_333,
        start_tick: 0,
    };
    let mut bytes = Vec::new();
    h.write(&mut bytes).expect("header");
    Frame {
        tick: 0,
        pixels: vec![0; 4],
        samples: vec![],
    }
    .write(&mut bytes)
    .expect("frame");
    crate::media::write_end(&mut bytes, 1, h.duration_us).expect("end");
    let file = tempfile::NamedTempFile::new().expect("wire fixture");
    fs::write(file.path(), bytes).expect("wire bytes");
    for suffix in ["exit 1", "printf garbage"] {
        let mut child = spawn_renderer(
            Command::new("sh")
                .args(["-c", &format!("cat \"$1\"; {suffix}"), "fixture"])
                .arg(file.path())
                .stdout(Stdio::piped()),
        )
        .expect("worker");
        let (sender, receiver) = mpsc::sync_channel(8);
        assert!(consume(&mut child, &source, 0, &Cancellation::default(), &sender).is_err());
        assert!(
            !receiver
                .try_iter()
                .any(|event| matches!(event, Event::Packet(Packet::End(_))))
        );
        terminate(&mut child);
    }
}

#[test]
fn waveform_streams_forward_levels_and_reject_failed_or_trailing_output() {
    let mut bytes = Vec::new();
    crate::media::peaks::write_header(&mut bytes, 2_000_000).expect("header");
    let mut accumulator = crate::media::peaks::Accumulator::new(2_000_000);
    accumulator
        .push(&[0x40; 32_000], &mut bytes)
        .expect("levels");
    accumulator.finish(&mut bytes).expect("end");
    let file = tempfile::NamedTempFile::new().expect("wire fixture");
    fs::write(file.path(), bytes).expect("wire bytes");
    for (suffix, succeeds) in [("", true), ("exit 1", false), ("printf garbage", false)] {
        let mut child = spawn_renderer(
            Command::new("sh")
                .args(["-c", &format!("cat \"$1\"; {suffix}"), "fixture"])
                .arg(file.path())
                .stdout(Stdio::piped()),
        )
        .expect("worker");
        let (sender, receiver) = mpsc::sync_channel(4096);
        let result = consume_peaks(&mut child, &Cancellation::default(), &sender);
        assert_eq!(result.is_ok(), succeeds, "{suffix:?}");
        let levels: usize = receiver
            .try_iter()
            .map(|event| match event {
                PeaksEvent::Levels { levels, .. } => levels.len(),
                _ => 0,
            })
            .sum();
        assert_eq!(levels, crate::media::peaks::BUCKETS as usize);
        terminate(&mut child);
    }
}

#[test]
fn storyboards_forward_cells_and_reject_failed_or_trailing_output() {
    use crate::media::storyboard::{Sheet, write_cell, write_end};

    let sheet = Sheet {
        width: 2,
        height: 2,
        count: 8,
        duration_us: 16_000_000,
    };
    let mut bytes = Vec::new();
    sheet.write(&mut bytes).expect("sheet");
    write_cell(&mut bytes, 4, &[7; 16]).expect("cell");
    write_cell(&mut bytes, 2, &[9; 16]).expect("cell");
    write_end(&mut bytes, sheet).expect("end");
    let file = tempfile::NamedTempFile::new().expect("wire fixture");
    fs::write(file.path(), bytes).expect("wire bytes");
    for (suffix, succeeds) in [("", true), ("exit 1", false), ("printf garbage", false)] {
        let mut child = spawn_renderer(
            Command::new("sh")
                .args(["-c", &format!("cat \"$1\"; {suffix}"), "fixture"])
                .arg(file.path())
                .stdout(Stdio::piped()),
        )
        .expect("worker");
        let (sender, receiver) = mpsc::sync_channel(64);
        let result = consume_storyboard(&mut child, &Cancellation::default(), &sender);
        assert_eq!(result.is_ok(), succeeds, "{suffix:?}");
        let events: Vec<_> = receiver.try_iter().collect();
        assert!(matches!(events.first(), Some(StoryboardEvent::Sheet(read)) if *read == sheet));
        let cells: Vec<u32> = events
            .iter()
            .filter_map(|event| match event {
                StoryboardEvent::Cell { index, pixels } if pixels.len() == 16 => Some(*index),
                _ => None,
            })
            .collect();
        assert_eq!(cells, [4, 2], "{suffix:?}");
        terminate(&mut child);
    }
}

#[test]
fn storyboards_and_waveforms_share_one_background_slot() {
    let source = SandboxedMedia {
        path: "/unused".into(),
        size: crate::services::MediaPreviewSize::new(16, 16),
        backend: MediaPreviewBackend::Software,
        input_owner: None,
        audio_only: false,
    };
    let slot = BackgroundSlot::acquire().expect("free slot");
    assert!(StoryboardSession::start(source.clone(), 128).is_none());
    assert!(PeaksSession::start(source.clone()).is_none());
    drop(slot);
    wait_for(|| ACTIVE_BACKGROUND.load(Ordering::Acquire) == 0);
    assert!(BackgroundSlot::acquire().is_some());
}
