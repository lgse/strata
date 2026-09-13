// SPDX-License-Identifier: MIT

use super::*;
use std::{io::Cursor, os::unix::net::UnixStream, thread};

fn header() -> Header {
    Header {
        width: 16,
        height: 16,
        audio: true,
        duration_us: 30_000_000,
        start_tick: 0,
    }
}
fn bytes() -> Vec<u8> {
    let mut bytes = Vec::new();
    header().write(&mut bytes).expect("header");
    bytes
}
fn parse(bytes: &[u8]) -> io::Result<Header> {
    Header::read(&mut Cursor::new(bytes), MediaPreviewSize::new(640, 480), 0)
}

#[test]
fn headers_reject_unknown_formats_dimensions_strides_flags_and_timeline() {
    assert_eq!(parse(&bytes()).expect("valid header"), header());
    for (offset, value) in [
        (8, 0_u32),
        (8, 641),
        (8, u32::MAX),
        (12, 481),
        (16, 63),
        (20, 2),
        (32, 1),
        (36, 1),
    ] {
        let mut bad = bytes();
        bad[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        assert!(parse(&bad).is_err(), "offset {offset}, value {value}");
    }
    for duration in [0_u64, MAX_DURATION_US + 1, u64::MAX] {
        let mut bad = bytes();
        bad[24..32].copy_from_slice(&duration.to_le_bytes());
        assert!(parse(&bad).is_err());
    }
    let mut bad = bytes();
    bad[7] = b'2';
    assert!(parse(&bad).is_err());
    for length in 0..HEADER_BYTES {
        assert!(parse(&bytes()[..length]).is_err());
    }
}

#[test]
fn frames_are_length_checked_before_allocation_and_must_be_contiguous() {
    let mut bytes = Vec::new();
    Frame {
        tick: 0,
        pixels: vec![1; 1024],
        samples: vec![2; AUDIO_BYTES],
    }
    .write(&mut bytes)
    .expect("frame");
    for (offset, value) in [
        (0, 2_u32),
        (4, 1),
        (16, 1023),
        (16, u32::MAX),
        (20, 0),
        (20, u32::MAX),
    ] {
        let mut bad = bytes[..24].to_vec();
        bad[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        assert!(Decoder::new(header()).read(&mut Cursor::new(bad)).is_err());
    }
    let mut bad = bytes.clone();
    bad[8] = 1;
    assert!(Decoder::new(header()).read(&mut Cursor::new(bad)).is_err());
    for length in [0, 23, 24, 24 + 1023, bytes.len() - 1] {
        assert!(
            Decoder::new(header())
                .read(&mut Cursor::new(&bytes[..length]))
                .is_err()
        );
    }
    let mut decoder = Decoder::new(header());
    assert!(matches!(
        decoder.read(&mut Cursor::new(&bytes)),
        Ok(Packet::Frame(_))
    ));
    assert!(decoder.read(&mut Cursor::new(&bytes)).is_err());
    let mut end = Vec::new();
    write_end(&mut end, 1, timestamp(1)).expect("end");
    assert!(matches!(
        decoder.read(&mut Cursor::new(&end)),
        Ok(Packet::End(_))
    ));
    assert!(decoder.read(&mut Cursor::new(end)).is_err());
}

#[test]
fn full_length_seeks_and_frames_stop_at_the_source_end_without_tick_overflow() {
    for duration in [31_000_000, 3_600_000_000, MAX_DURATION_US] {
        let ticks = (duration * u64::from(FPS)).div_ceil(1_000_000) as u32;
        for requested in [duration, u64::MAX] {
            assert_eq!(seek_tick(requested, duration), ticks - 1);
        }
        let h = Header {
            duration_us: duration,
            start_tick: ticks - 1,
            ..header()
        };
        let mut encoded = Vec::new();
        h.write(&mut encoded).expect("header");
        let h = Header::read(
            &mut Cursor::new(encoded),
            MediaPreviewSize::new(640, 480),
            ticks - 1,
        )
        .expect("long timeline");
        let mut decoder = Decoder::new(h);
        let mut bytes = Vec::new();
        Frame {
            tick: ticks - 1,
            pixels: vec![0; h.video_bytes()],
            samples: vec![0; AUDIO_BYTES],
        }
        .write(&mut bytes)
        .expect("last frame");
        assert!(matches!(
            decoder.read(&mut Cursor::new(bytes)),
            Ok(Packet::Frame(_))
        ));
        let mut bytes = Vec::new();
        Frame {
            tick: ticks,
            pixels: vec![0; h.video_bytes()],
            samples: vec![0; AUDIO_BYTES],
        }
        .write(&mut bytes)
        .expect("extra frame");
        assert!(decoder.read(&mut Cursor::new(bytes)).is_err());
        let mut bytes = Vec::new();
        write_end(&mut bytes, ticks, duration + 1).expect("invalid end");
        assert!(decoder.read(&mut Cursor::new(bytes)).is_err());
        let mut bytes = Vec::new();
        write_end(&mut bytes, ticks, duration).expect("source end");
        assert!(
            matches!(decoder.read(&mut Cursor::new(bytes)), Ok(Packet::End(end)) if end == duration)
        );
    }
    assert_eq!(seek_tick(32_000_000, 3_600_000_000), 960);
    assert_eq!(seek_tick(u64::MAX, u64::MAX), u32::MAX - 1);
}

#[test]
fn blocked_writes_cancel_or_timeout_and_closed_workers_return_epipe() {
    let (write, read) = UnixStream::pair().expect("private transport");
    let cancellation = Cancellation::default();
    let mut writer = TimedWriter::new(
        &write,
        Instant::now() + Duration::from_millis(40),
        &cancellation,
    )
    .expect("nonblocking transport");
    assert_eq!(
        writer
            .write_all(&vec![0; 1024 * 1024])
            .expect_err("bounded write deadline")
            .kind(),
        io::ErrorKind::TimedOut
    );
    writer.deadline = Instant::now() + Duration::from_secs(10);
    cancellation.cancel();
    assert_eq!(
        writer.write_all(&[1]).expect_err("cancelled writer").kind(),
        io::ErrorKind::ConnectionAborted
    );
    drop(read);
    let cancellation = Cancellation::default();
    let mut writer = TimedWriter::new(
        &write,
        Instant::now() + Duration::from_secs(1),
        &cancellation,
    )
    .expect("closed pipe writer");
    assert_eq!(
        writer.write_all(&[1]).expect_err("EPIPE").kind(),
        io::ErrorKind::BrokenPipe
    );
}

#[test]
fn partial_reads_and_silent_workers_obey_deadlines_and_cancellation() {
    let (read, mut write) = UnixStream::pair().expect("private pipe");
    write.write_all(b"a").expect("partial record");
    let cancellation = Cancellation::default();
    let started = Instant::now();
    let mut reader = TimedReader {
        fd: &read,
        deadline: started + Duration::from_millis(50),
        cancellation: &cancellation,
    };
    assert_eq!(
        reader.read_exact(&mut [0; 2]).expect_err("deadline").kind(),
        io::ErrorKind::TimedOut
    );
    assert!(started.elapsed() < Duration::from_secs(1));
    let cancel = cancellation.clone();
    let worker = thread::spawn(move || {
        thread::sleep(Duration::from_millis(20));
        cancel.cancel();
    });
    reader.deadline = Instant::now() + Duration::from_secs(10);
    let started = Instant::now();
    assert_eq!(
        reader
            .read_exact(&mut [0; 1])
            .expect_err("cancelled")
            .kind(),
        io::ErrorKind::ConnectionAborted
    );
    assert!(started.elapsed() < Duration::from_secs(1));
    worker.join().expect("cancel thread");
}
