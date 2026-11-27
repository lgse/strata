// SPDX-License-Identifier: MIT

use super::super::webp::is_animated_webp_file;
use super::*;
use image::{ColorType, ImageEncoder, Rgba, RgbaImage, codecs::webp::WebPEncoder};

/// Encodes solid-colour RGBA frames into a valid lossless animated WebP bitstream.
fn build_animated_webp(width: u32, height: u32, frames: &[([u8; 4], u32)]) -> Vec<u8> {
    let mut body = Vec::new();
    let mut vp8x = [0u8; 10];
    vp8x[0] = 0x12; // Alpha + Animation
    vp8x[4..7].copy_from_slice(&(width - 1).to_le_bytes()[..3]);
    vp8x[7..10].copy_from_slice(&(height - 1).to_le_bytes()[..3]);
    write_chunk(&mut body, b"VP8X", &vp8x);
    write_chunk(&mut body, b"ANIM", &[0, 0, 0, 0, 0, 0]);
    for (rgba, duration_ms) in frames {
        let vp8l_chunk = encode_vp8l_chunk(width, height, *rgba);
        let mut anmf = Vec::with_capacity(16 + vp8l_chunk.len());
        anmf.extend_from_slice(&[0, 0, 0, 0, 0, 0]); // x = 0, y = 0
        anmf.extend_from_slice(&(width - 1).to_le_bytes()[..3]);
        anmf.extend_from_slice(&(height - 1).to_le_bytes()[..3]);
        anmf.extend_from_slice(&duration_ms.to_le_bytes()[..3]);
        anmf.push(0x02); // do not blend; dispose none
        anmf.extend_from_slice(&vp8l_chunk);
        write_chunk(&mut body, b"ANMF", &anmf);
    }
    let mut riff = Vec::with_capacity(12 + body.len());
    riff.extend_from_slice(b"RIFF");
    riff.extend_from_slice(&((4 + body.len()) as u32).to_le_bytes());
    riff.extend_from_slice(b"WEBP");
    riff.extend_from_slice(&body);
    riff
}

fn encode_vp8l_chunk(width: u32, height: u32, rgba: [u8; 4]) -> Vec<u8> {
    let image = RgbaImage::from_pixel(width, height, Rgba(rgba));
    let mut webp = Vec::new();
    WebPEncoder::new_lossless(&mut webp)
        .write_image(image.as_raw(), width, height, ColorType::Rgba8.into())
        .expect("encode lossless webp frame");
    assert_eq!(&webp[12..16], b"VP8L");
    webp[12..].to_vec()
}

fn write_chunk(out: &mut Vec<u8>, fourcc: &[u8; 4], payload: &[u8]) {
    out.extend_from_slice(fourcc);
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(payload);
    if payload.len() % 2 == 1 {
        out.push(0);
    }
}

fn write_static_webp(path: &Path, width: u32, height: u32) {
    let image = RgbaImage::from_pixel(width, height, Rgba([10, 20, 30, 255]));
    let mut webp = Vec::new();
    WebPEncoder::new_lossless(&mut webp)
        .write_image(image.as_raw(), width, height, ColorType::Rgba8.into())
        .expect("encode static webp");
    std::fs::write(path, webp).expect("write static webp");
}

#[test]
fn short_animated_webp_loops_within_bounded_generation_and_seeks_by_phase() {
    let directory = tempfile::tempdir().expect("webp fixture");
    let input = directory.path().join("loop.webp");
    let bytes = build_animated_webp(16, 12, &[([255, 0, 0, 255], 500), ([0, 0, 255, 255], 500)]);
    std::fs::write(&input, bytes).expect("save animated webp");

    let (header, frames, end) = decoded(&input, "160x90", 0);
    assert_eq!(
        (
            header.width,
            header.height,
            header.audio,
            header.duration_us,
            frames.len(),
            end
        ),
        (16, 12, false, 30_000_000, 900, 30_000_000)
    );
    assert_eq!(&frames[0].pixels[..4], &[255, 0, 0, 255]);
    assert_eq!(&frames[15].pixels[..4], &[0, 0, 255, 255]);
    assert_eq!(frames[0].pixels, frames[30].pixels, "WebP loop phase");
    assert_ne!(frames[0].pixels, frames[15].pixels, "moving WebP frames");

    let (_, sought, _) = decoded(&input, "160x90", 765);
    assert_eq!(
        frames[15].pixels, sought[0].pixels,
        "seek keeps the WebP loop phase"
    );

    let (_, loop_boundary, boundary_end) = decoded(&input, "160x90", 750);
    assert_eq!((loop_boundary.len(), boundary_end), (150, 30_000_000));
    for (actual, expected) in loop_boundary.iter().zip(&frames[750..]) {
        assert_eq!(actual.pixels, expected.pixels);
    }
}

#[test]
fn long_animated_webp_plays_once_seeks_linearly_and_rejects_past_end() {
    let directory = tempfile::tempdir().expect("long webp fixture");
    let input = directory.path().join("long.webp");
    let bytes = build_animated_webp(
        8,
        8,
        &[([20, 200, 20, 255], 20_000), ([200, 20, 200, 255], 15_000)],
    );
    std::fs::write(&input, bytes).expect("save long animated webp");

    let (header, frames, end) = decoded(&input, "160x90", 0);
    assert_eq!(
        (header.duration_us, frames.len(), end),
        (35_000_000, 1050, 35_000_000)
    );
    assert_eq!(&frames[0].pixels[..4], &[20, 200, 20, 255]);
    assert_eq!(&frames[600].pixels[..4], &[200, 20, 200, 255]);

    let (_, sought, sought_end) = decoded(&input, "160x90", 960);
    assert_eq!((sought.len(), sought_end), (90, 35_000_000));
    assert_eq!(frames[960].pixels, sought[0].pixels);
    assert!(stream_fails(&input, 1050));
}

#[test]
fn animated_webp_scales_down_without_enlarging_and_handles_zero_delay() {
    let directory = tempfile::tempdir().expect("scaled webp fixture");
    let wide = directory.path().join("wide.webp");
    std::fs::write(
        &wide,
        build_animated_webp(64, 32, &[([255, 128, 0, 255], 0), ([0, 128, 255, 255], 0)]),
    )
    .expect("save wide animated webp");

    let (downscaled, frames, end) = decoded(&wide, "32x32", 0);
    assert_eq!((downscaled.width, downscaled.height), (32, 16));
    assert_eq!(
        (downscaled.duration_us, frames.len(), end),
        (30_000_000, 900, 30_000_000)
    );
    assert_ne!(frames[0].pixels, frames[3].pixels);

    let (native, _, _) = decoded(&wide, "520x800", 0);
    assert_eq!((native.width, native.height), (64, 32));
}

#[test]
fn animated_webp_streams_with_backpressure_and_cancels_when_consumer_closes() {
    let directory = tempfile::tempdir().expect("backpressure fixture");
    let input = directory.path().join("stream.webp");
    std::fs::write(
        &input,
        build_animated_webp(128, 96, &[([255, 0, 0, 255], 100), ([0, 255, 0, 255], 100)]),
    )
    .expect("save streaming animated webp");

    let (read, mut write) = UnixStream::pair().expect("private frame pipe");
    let worker = thread::spawn(move || {
        stream(
            &input,
            "520x800",
            MediaPreviewBackend::Software,
            0,
            false,
            &mut write,
        )
    });
    let cancellation = Cancellation::default();
    let mut reader = TimedReader {
        fd: &read,
        deadline: Instant::now() + Duration::from_secs(10),
        cancellation: &cancellation,
    };
    let header =
        Header::read(&mut reader, MediaPreviewSize::new(520, 800), 0).expect("streaming header");
    let mut decoder = Decoder::new(header);
    let Packet::Frame(first) = decoder.read(&mut reader).expect("first frame") else {
        panic!("expected first frame");
    };
    assert_eq!(first.tick, 0);
    drop(read);
    let deadline = Instant::now() + Duration::from_secs(2);
    while !worker.is_finished() {
        assert!(
            Instant::now() < deadline,
            "closed consumer must stop worker"
        );
        thread::sleep(Duration::from_millis(5));
    }
    assert!(worker.join().expect("worker").is_err());
}

#[test]
fn animated_webp_rejects_storyboard_malformed_and_oversized_inputs() {
    let directory = tempfile::tempdir().expect("error webp fixtures");
    let valid = directory.path().join("valid.webp");
    std::fs::write(
        &valid,
        build_animated_webp(8, 8, &[([255, 0, 0, 255], 100), ([0, 0, 255, 255], 100)]),
    )
    .expect("save valid animated webp");
    assert!(is_animated_webp_file(&valid));

    let storyboard_out = directory.path().join("storyboard.bin");
    assert_eq!(
        run_storyboard(&valid, &storyboard_out, 160).expect_err("no storyboard"),
        "Animations have no storyboard"
    );

    let static_path = directory.path().join("still.webp");
    write_static_webp(&static_path, 8, 8);
    assert!(!is_animated_webp_file(&static_path));

    let malformed = directory.path().join("corrupt.webp");
    let mut corrupt_header = [0u8; 30];
    corrupt_header[0..4].copy_from_slice(b"RIFF");
    corrupt_header[4..8].copy_from_slice(&22u32.to_le_bytes());
    corrupt_header[8..12].copy_from_slice(b"WEBP");
    corrupt_header[12..16].copy_from_slice(b"VP8X");
    corrupt_header[16..20].copy_from_slice(&10u32.to_le_bytes());
    corrupt_header[20] = 0x02;
    std::fs::write(&malformed, corrupt_header).expect("save corrupt animated webp");
    let mut out = Vec::new();
    assert!(
        stream(
            &malformed,
            "520x800",
            MediaPreviewBackend::Software,
            0,
            false,
            &mut out
        )
        .is_err()
    );
    assert!(
        out.is_empty(),
        "malformed input must emit no partial header"
    );

    let oversized = directory.path().join("oversized.webp");
    let mut huge = build_animated_webp(8, 8, &[([255, 0, 0, 255], 100)]);
    // Canvas width = 10_000, height = 10_000 (100 MP > 50 MP source budget).
    huge[24..27].copy_from_slice(&9_999u32.to_le_bytes()[..3]);
    huge[27..30].copy_from_slice(&9_999u32.to_le_bytes()[..3]);
    std::fs::write(&oversized, huge).expect("save oversized animated webp");
    let mut huge_out = Vec::new();
    assert!(
        stream(
            &oversized,
            "520x800",
            MediaPreviewBackend::Software,
            0,
            false,
            &mut huge_out
        )
        .is_err()
    );
    assert!(
        huge_out.is_empty(),
        "oversized canvas must fail before emitting any header"
    );
}
