// SPDX-License-Identifier: MIT

use super::*;

fn pcm(samples: impl IntoIterator<Item = f64>) -> Vec<u8> {
    samples
        .into_iter()
        .flat_map(|sample| ((sample * 32_767.0) as i16).to_le_bytes())
        .collect()
}

fn decode(stream: &[u8]) -> io::Result<(u64, Vec<Option<u8>>)> {
    let mut reader = stream;
    let duration = read_header(&mut reader)?;
    let mut runs = RunReader::default();
    let mut levels = vec![None; BUCKETS as usize];
    while let Some((start, run)) = runs.read(&mut reader)? {
        for (offset, level) in run.into_iter().enumerate() {
            levels[start as usize + offset] = Some(level);
        }
    }
    Ok((duration, levels))
}

#[test]
fn streamed_levels_follow_loudness_across_the_track() {
    let duration_us = 4_096_000;
    let total = duration_us * u64::from(SAMPLE_RATE) / 1_000_000;
    let mut stream = Vec::new();
    write_header(&mut stream, duration_us).expect("waveform stream");
    let mut accumulator = Accumulator::new(duration_us);
    let samples: Vec<f64> = (0..total)
        .map(|index| {
            let loud = index >= total / 2;
            let phase = index as f64 * 2.0 * std::f64::consts::PI * 440.0 / 8_000.0;
            if loud { 0.5 * phase.sin() } else { 0.0 }
        })
        .collect();
    for chunk in samples.chunks(997) {
        accumulator
            .push(&pcm(chunk.iter().copied()), &mut stream)
            .expect("waveform stream");
    }
    accumulator.finish(&mut stream).expect("waveform stream");

    let (duration, levels) = decode(&stream).expect("waveform stream");
    assert_eq!(duration, duration_us);
    assert!(levels.iter().all(Option::is_some), "every bucket is sent");
    let quiet = rms(levels[100].expect("waveform stream"));
    let loud = rms(levels[900].expect("waveform stream"));
    assert_eq!(quiet, 0.0);
    assert!((loud - 0.354).abs() < 0.02, "sine RMS was {loud}");
}

#[test]
fn a_short_decode_ends_without_filling_every_bucket() {
    let mut stream = Vec::new();
    write_header(&mut stream, 60_000_000).expect("waveform stream");
    let mut accumulator = Accumulator::new(60_000_000);
    accumulator
        .push(&pcm(std::iter::repeat_n(0.25, 8_000)), &mut stream)
        .expect("waveform stream");
    accumulator.finish(&mut stream).expect("waveform stream");

    let (_, levels) = decode(&stream).expect("waveform stream");
    assert!(levels[0].is_some());
    assert!(levels[BUCKETS as usize - 1].is_none());
}

#[test]
fn malformed_streams_are_rejected() {
    let mut header = Vec::new();
    write_header(&mut header, 1_000_000).expect("waveform stream");

    let mut wrong_magic = header.clone();
    wrong_magic[0] = b'X';
    assert!(decode(&wrong_magic).is_err());

    let mut zero_duration = Vec::new();
    write_header(&mut zero_duration, 0).expect("waveform stream");
    assert!(read_header(&mut zero_duration.as_slice()).is_err());

    let mut skipped = header.clone();
    write_run(&mut skipped, 5, &[1, 2]).expect("waveform stream");
    assert!(decode(&skipped).is_err());

    let mut overflowing = header.clone();
    write_run(&mut overflowing, 0, &vec![1; MAX_RUN as usize + 1]).expect("waveform stream");
    assert!(decode(&overflowing).is_err());

    let mut truncated = header;
    write_run(&mut truncated, 0, &[1, 2, 3]).expect("waveform stream");
    truncated.pop();
    assert!(decode(&truncated).is_err());
}
