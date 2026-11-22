// SPDX-License-Identifier: MIT

//! Streams that stand in for a sandboxed decoder in tests.

use super::*;
use crate::media::Frame;

pub fn stream(header: Header) -> Result<Session, String> {
    stream_to(header, header.duration_us)
}

pub fn stream_to(header: Header, end: u64) -> Result<Session, String> {
    stream_with_tone(header, end, None)
}

/// A fixture stream whose interleaved 48 kHz stereo S16LE PCM carries a sine wave at
/// `frequency_hz`, for verifying live spectrum analysis and ballistics end-to-end.
pub fn stream_tone(header: Header, frequency_hz: f32) -> Result<Session, String> {
    stream_with_tone(header, header.duration_us, Some(frequency_hz))
}

fn stream_with_tone(header: Header, end: u64, tone_hz: Option<f32>) -> Result<Session, String> {
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
        let mut sample_index: u64 = 0;
        for tick in header.start_tick..end_tick {
            let bytes = crate::media::audio_bytes(header, tick);
            let samples = match tone_hz {
                Some(frequency) => sine_pcm(bytes, frequency, &mut sample_index),
                None => vec![0; bytes],
            };
            let frame = Frame {
                tick,
                pixels: vec![(tick % 255) as u8; header.video_bytes()],
                samples,
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

fn sine_pcm(byte_len: usize, frequency_hz: f32, sample_index: &mut u64) -> Vec<u8> {
    let frames = byte_len / 4;
    let mut out = Vec::with_capacity(frames * 4);
    let step = std::f32::consts::TAU * frequency_hz / crate::media::SAMPLE_RATE as f32;
    for _ in 0..frames {
        let sample = ((*sample_index as f32 * step).sin() * 16_384.0) as i16;
        let le = sample.to_le_bytes();
        out.extend_from_slice(&[le[0], le[1], le[0], le[1]]);
        *sample_index += 1;
    }
    out
}

/// A session whose decoder dies before producing a header, as a seek past the
/// end of a truncated file does.
pub fn failing_stream() -> Result<Session, String> {
    let slot = WorkerSlot::acquire().ok_or("Media previews are busy (four active players)")?;
    let slot = Arc::new(slot);
    let worker_slot = slot.clone();
    let cancellation = Cancellation::default();
    let cancelled = cancellation.clone();
    let (sender, receiver) = mpsc::sync_channel(QUEUED_PACKETS);
    let worker = thread::spawn(move || {
        let _slot = worker_slot;
        let _sent = send(
            &sender,
            Event::Failed("failed to fill whole buffer".into()),
            &cancelled,
        );
    });
    Ok(Session {
        cancellation,
        receiver,
        worker,
        _slot: slot,
    })
}
