// SPDX-License-Identifier: MIT

use super::*;

fn audio_size() -> MediaPreviewSize {
    MediaPreviewSize::new(520, 800)
}

fn tone(path: &Path, frequency: u32, seconds: &str, codec: &[&str]) {
    success(
        Command::new("ffmpeg")
            .args(["-nostdin", "-v", "error", "-f", "lavfi", "-i"])
            .arg(format!("sine=frequency={frequency}:duration={seconds}"))
            .args(codec)
            .arg(path),
    );
}

fn still_image(path: &Path, color: &str, size: &str) {
    success(
        Command::new("ffmpeg")
            .args(["-nostdin", "-v", "error", "-f", "lavfi", "-i"])
            .arg(format!("color=c={color}:size={size}"))
            .args(["-frames:v", "1", "-threads", "1"])
            .arg(path),
    );
}

/// Muxes `cover` into `audio` as an attached picture with the given codec arguments.
fn with_cover(audio: &Path, cover: &Path, output: &Path, codecs: &[&str]) {
    success(
        Command::new("ffmpeg")
            .args(["-v", "error", "-i"])
            .arg(audio)
            .arg("-i")
            .arg(cover)
            .args(["-map", "0:a", "-map", "1:v"])
            .args(codecs)
            .args(["-disposition:v", "attached_pic"])
            .arg(output),
    );
}

fn header_of(path: &Path, audio_only: bool) -> (Header, Cursor<Vec<u8>>) {
    let mut bytes = Vec::new();
    stream(
        path,
        "520x800",
        MediaPreviewBackend::Software,
        0,
        audio_only,
        &mut bytes,
    )
    .expect("audio stream");
    let mut reader = Cursor::new(bytes);
    let header = Header::read(&mut reader, audio_size(), 0).expect("audio header");
    (header, reader)
}

fn assert_audio_plays_under_every_policy(audio: &Path) {
    for policy in [
        MediaPreviewBackend::Automatic,
        MediaPreviewBackend::VaApi,
        MediaPreviewBackend::Vulkan,
        MediaPreviewBackend::Software,
    ] {
        let mut bytes = Vec::new();
        stream(audio, "520x800", policy, 0, false, &mut bytes).expect("audio under every policy");
        let h = Header::read(&mut Cursor::new(bytes), audio_size(), 0).expect("audio header");
        assert_eq!((h.audio, h.width), (true, 0));
    }
}

fn assert_cover_art_plays_and_seeks(attached: &Path) {
    let (header, frames, _) = decoded(attached, "520x800", 0);
    assert_eq!(
        (header.audio, header.width, header.height, frames.len()),
        (true, 64, 48, 30)
    );
    let pixel = &frames[0].pixels[..4];
    assert!(
        pixel[2] > pixel[0] && pixel[2] > pixel[1],
        "cover frame: {pixel:?}"
    );
    let (_, sought, end) = decoded(attached, "520x800", 15);
    assert_eq!((sought.len(), end), (15, 1_000_000));
    assert_eq!(sought[0].pixels, frames[0].pixels);
    assert!(
        has_audible_samples(&sought),
        "cover-art seeking must retain audio: {attached:?}"
    );
}

#[test]
fn audio_only_and_attached_cover_art_do_not_require_a_hardware_video_decoder() {
    let directory = tempfile::tempdir().expect("fixtures");
    let audio = directory.path().join("tone.flac");
    tone(&audio, 660, "1", &[]);
    assert_audio_plays_under_every_policy(&audio);
    let aac = directory.path().join("tone.m4a");
    success(
        Command::new("ffmpeg")
            .args(["-nostdin", "-v", "error", "-i"])
            .arg(&audio)
            .args(["-c:a", "aac"])
            .arg(&aac),
    );
    let (_, frames, _) = decoded(&aac, "520x800", 0);
    assert!(
        audible(&frames[0].samples[..1024]),
        "zero-position playback must preserve the opening AAC samples"
    );
    let cover = directory.path().join("cover.jpg");
    still_image(&cover, "blue", "64x48");
    let attached = directory.path().join("cover.flac");
    with_cover(&audio, &cover, &attached, &["-c", "copy"]);
    let info = probe(&attached, audio_size(), 0).expect("cover metadata");
    assert!(info.cover);
    let mp3 = directory.path().join("cover.mp3");
    with_cover(
        &audio,
        &cover,
        &mp3,
        &["-c:a", "libmp3lame", "-c:v", "copy"],
    );
    for attached in [&attached, &mp3] {
        assert_cover_art_plays_and_seeks(attached);
    }
}

fn tagged_mp3(path: &Path, art: &Path) {
    success(
        Command::new("ffmpeg")
            .args(["-nostdin", "-v", "error", "-f", "lavfi", "-i"])
            .arg("sine=frequency=440:duration=2")
            .arg("-i")
            .arg(art)
            .args([
                "-map",
                "0:a",
                "-map",
                "1:v",
                "-c:a",
                "libmp3lame",
                "-c:v",
                "png",
            ])
            .args(["-disposition:v", "attached_pic"])
            .args([
                "-metadata",
                "title=Night Drive",
                "-metadata",
                "artist=Layers",
            ])
            .args(["-metadata", "track=3/12"])
            .arg(path),
    );
}

/// Muxes `back` then `front` as attached pictures; only the second may carry the front tag.
fn mp3_with_two_pictures(path: &Path, plain: &Path, [back, front]: [&Path; 2], front_tag: bool) {
    success(
        Command::new("ffmpeg")
            .args(["-nostdin", "-v", "error", "-i"])
            .arg(plain)
            .arg("-i")
            .arg(back)
            .arg("-i")
            .arg(front)
            .args([
                "-map",
                "0:a",
                "-map",
                "1:v",
                "-map",
                "2:v",
                "-c:a",
                "libmp3lame",
                "-c:v",
                "copy",
                "-disposition:v",
                "attached_pic",
                "-metadata:s:v:0",
                "comment=Cover (back)",
                "-metadata:s:v:1",
            ])
            .arg(if front_tag {
                "comment=Cover (front)"
            } else {
                "comment=Other"
            })
            .arg(path),
    );
}

fn assert_artwork_is_not_decoded(tagged: &Path) {
    let (header, mut reader) = header_of(tagged, true);
    assert_eq!(
        (header.audio, header.width, header.height),
        (true, 0, 0),
        "artwork is not decoded"
    );
    let Packet::Frame(frame) = Decoder::new(header).read(&mut reader).expect("frame") else {
        panic!("expected a frame");
    };
    assert!(frame.pixels.is_empty());
}

fn assert_real_video_survives_an_audio_request(video: &Path) {
    fixture(
        video,
        Clip {
            size: "64x48",
            rate: 30,
            seconds: 1,
            audio: false,
        },
    );
    let (header, _) = header_of(video, true);
    assert_eq!((header.width, header.height, header.audio), (64, 48, false));
    assert!(cover(video, 800).expect("no attached picture").is_empty());
}

fn assert_front_artwork_is_preferred(directory: &Path, plain: &Path, art: &Path, png: &[u8]) {
    let back = directory.join("back.png");
    still_image(&back, "red", "64x48");
    for front_tag in [true, false] {
        let multiple = directory.join(format!("multiple-{front_tag}.mp3"));
        mp3_with_two_pictures(&multiple, plain, [&back, art], front_tag);
        let selected = cover(&multiple, 800).expect("exactly one of two attached pictures");
        if front_tag {
            assert_eq!(selected, png, "front artwork wins over the first picture");
        } else {
            assert_eq!(crate::sandbox::png_dimensions(&selected), Some((64, 48)));
        }
    }
}

fn assert_tags_are_extracted(tagged: &Path) {
    let tags = crate::sandbox::metadata::AudioTags::from_json(&audio_tags(tagged).expect("tags"))
        .expect("tag JSON");
    assert_eq!(
        (
            tags.title.as_deref(),
            tags.artist.as_deref(),
            (tags.track, tags.track_total)
        ),
        (Some("Night Drive"), Some("Layers"), (Some(3), Some(12)))
    );
}

fn assert_peaks_cover_the_tone(tagged: &Path, output: &Path) {
    run_peaks(tagged, output).expect("waveform overview");
    let bytes = std::fs::read(output).expect("peaks output");
    let mut reader = bytes.as_slice();
    let duration = crate::media::peaks::read_header(&mut reader).expect("peaks header");
    assert!((1_900_000..2_200_000).contains(&duration), "{duration}");
    let mut runs = crate::media::peaks::RunReader::default();
    let mut levels = Vec::new();
    while let Some((_, run)) = runs.read(&mut reader).expect("peaks run") {
        levels.extend(run);
    }
    assert!(reader.is_empty());
    assert!(levels.len() > 1000, "{} buckets", levels.len());
    let middle = crate::media::peaks::rms(levels[levels.len() / 2]);
    assert!(middle > 0.02, "a steady tone has a visible level: {middle}");
}

#[test]
fn audio_previews_play_without_artwork_and_extract_tags_cover_and_peaks() {
    let directory = tempfile::tempdir().expect("fixtures");
    let art = directory.path().join("cover.png");
    success(
        Command::new("ffmpeg")
            .args(["-nostdin", "-v", "error", "-f", "lavfi", "-i"])
            .arg("color=c=blue:size=1000x600")
            .args(["-frames:v", "1"])
            .arg(&art),
    );
    let tagged = directory.path().join("tagged.mp3");
    tagged_mp3(&tagged, &art);
    let plain = directory.path().join("plain.ogg");
    tone(&plain, 440, "1", &["-c:a", "libvorbis"]);

    assert_artwork_is_not_decoded(&tagged);
    assert_real_video_survives_an_audio_request(&directory.path().join("silent.mkv"));

    let png = cover(&tagged, 800).expect("embedded artwork");
    assert_eq!(crate::sandbox::png_dimensions(&png), Some((800, 480)));
    assert_front_artwork_is_preferred(directory.path(), &plain, &art, &png);
    assert!(cover(&plain, 800).expect("no artwork").is_empty());
    assert!(cover(&directory.path().join("missing"), 800).is_err());

    assert_tags_are_extracted(&tagged);
    assert_peaks_cover_the_tone(&tagged, &directory.path().join("peaks"));
}

#[test]
fn storyboards_sample_keyframes_in_subdivision_order_and_skip_unsuitable_inputs() {
    use crate::media::storyboard::{CellReader, Sheet, subdivision_order};

    let directory = tempfile::tempdir().expect("storyboard fixtures");
    let video = directory.path().join("clip.mkv");
    fixture(
        &video,
        Clip {
            size: "64x48",
            rate: 10,
            seconds: 12,
            audio: false,
        },
    );
    let output = directory.path().join("board");
    run_storyboard(&video, &output, 128).expect("storyboard");
    let mut reader = Cursor::new(std::fs::read(&output).expect("board bytes"));
    let sheet = Sheet::read(&mut reader).expect("sheet");
    assert_eq!(
        sheet,
        Sheet {
            width: 64,
            height: 48,
            count: 8,
            duration_us: 12_000_000,
        },
        "cells are never enlarged beyond the source"
    );
    let mut cells = CellReader::new(sheet);
    let mut indices = Vec::new();
    let mut distinct = std::collections::HashSet::new();
    while let Some((index, pixels)) = cells.read(&mut reader).expect("cell") {
        indices.push(index);
        distinct.insert(pixels);
    }
    assert_eq!(indices, subdivision_order(8));
    assert!(distinct.len() > 1, "cells sample different moments");
    assert_eq!(reader.position() as usize, reader.get_ref().len());
    assert_unsuitable_inputs_are_rejected(directory.path(), &output);
}

fn assert_unsuitable_inputs_are_rejected(directory: &Path, output: &Path) {
    let short = directory.join("short.mkv");
    fixture(
        &short,
        Clip {
            size: "64x48",
            rate: 10,
            seconds: 2,
            audio: false,
        },
    );
    assert!(run_storyboard(&short, output, 128).is_err());
    let animation = directory.join("loop.gif");
    animated_gif(&animation, 1);
    assert!(run_storyboard(&animation, output, 128).is_err());
    let audio = directory.join("tone.wav");
    tone(&audio, 440, "6", &[]);
    assert!(run_storyboard(&audio, output, 128).is_err());
}
