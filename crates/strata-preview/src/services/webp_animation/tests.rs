// SPDX-License-Identifier: MIT

use std::io::Cursor;

use super::*;

fn chunk(fourcc: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let mut bytes = fourcc.to_vec();
    bytes.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    bytes.extend_from_slice(payload);
    if payload.len() % 2 == 1 {
        bytes.push(0);
    }
    bytes
}

fn u24(value: u32) -> [u8; 3] {
    let bytes = value.to_le_bytes();
    [bytes[0], bytes[1], bytes[2]]
}

fn vp8x(flags: u8, width: u32, height: u32) -> Vec<u8> {
    let mut payload = vec![flags, 0, 0, 0];
    payload.extend_from_slice(&u24(width - 1));
    payload.extend_from_slice(&u24(height - 1));
    chunk(b"VP8X", &payload)
}

fn anmf(duration_ms: u32) -> Vec<u8> {
    let mut payload = Vec::new();
    for value in [0, 0, 15, 15, duration_ms] {
        payload.extend_from_slice(&u24(value));
    }
    payload.push(0);
    payload.extend_from_slice(&chunk(b"VP8L", &[0x2f, 0, 0, 0, 0]));
    chunk(b"ANMF", &payload)
}

fn webp(chunks: &[Vec<u8>]) -> Vec<u8> {
    let body: Vec<u8> = chunks.concat();
    let mut bytes = b"RIFF".to_vec();
    bytes.extend_from_slice(&(body.len() as u32 + 4).to_le_bytes());
    bytes.extend_from_slice(b"WEBP");
    bytes.extend_from_slice(&body);
    bytes
}

fn animated(durations: &[u32]) -> Vec<u8> {
    let mut chunks = vec![
        vp8x(ANIMATION_FLAG, 250, 198),
        chunk(b"ANIM", &[0; 6]),
        chunk(b"ICCP", &[1, 2, 3]),
    ];
    chunks.extend(durations.iter().map(|duration| anmf(*duration)));
    webp(&chunks)
}

#[test]
fn only_extended_webp_headers_with_the_animation_flag_are_animated() {
    let still_extended = webp(&[vp8x(0x10, 8, 8), chunk(b"VP8L", &[0x2f, 0, 0, 0, 0])]);
    let simple_lossy = webp(&[chunk(b"VP8 ", &[0; 10])]);
    let cases: [(&str, &[u8], bool); 6] = [
        ("animated", &animated(&[70]), true),
        ("extended still", &still_extended, false),
        ("simple lossy", &simple_lossy, false),
        ("truncated before flags", &animated(&[70])[..20], false),
        (
            "not RIFF",
            b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0\x01",
            false,
        ),
        (
            "RIFF but not WebP",
            b"RIFF\x24\0\0\0WAVEfmt \x10\0\0\0\x12\0\0\0",
            false,
        ),
    ];
    for (name, bytes, expected) in cases {
        assert_eq!(is_animated_webp(bytes), expected, "{name}");
    }
}

#[test]
fn animation_reads_canvas_frame_count_and_total_duration_across_padded_chunks() {
    let animation = read_webp_animation(&mut Cursor::new(animated(&[70, 70, 100])))
        .expect("readable animation");
    assert_eq!(
        animation,
        Some(WebpAnimation {
            canvas_width: 250,
            canvas_height: 198,
            frames: 3,
            duration_us: 240_000,
        })
    );
}

#[test]
fn malformed_or_still_files_have_no_animation() {
    let still = webp(&[vp8x(0x10, 8, 8), chunk(b"VP8L", &[0x2f, 0, 0, 0, 0])]);
    let mut truncated_frame = animated(&[70]);
    truncated_frame.truncate(truncated_frame.len() - 12);
    let cases: [(&str, Vec<u8>); 4] = [
        ("still", still),
        ("no frames", animated(&[])),
        ("zero duration", animated(&[0, 0])),
        ("truncated frame", truncated_frame),
    ];
    for (name, bytes) in cases {
        assert_eq!(
            read_webp_animation(&mut Cursor::new(bytes)).expect("readable bytes"),
            None,
            "{name}"
        );
    }
}

#[test]
fn frame_counts_beyond_the_loop_limit_have_no_animation() {
    let durations = vec![10; MAX_FRAMES as usize + 1];
    assert_eq!(
        read_webp_animation(&mut Cursor::new(animated(&durations))).expect("readable bytes"),
        None
    );
}
