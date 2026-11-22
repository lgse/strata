// SPDX-License-Identifier: MIT

use std::io::Cursor;

use image::{ImageFormat, Rgba, RgbaImage};
use strata_core::file_activity;

use super::*;

fn png(red: u8) -> Vec<u8> {
    let img = RgbaImage::from_pixel(2, 2, Rgba([red, 40, 80, 255]));
    let mut out = Cursor::new(Vec::new());
    img.write_to(&mut out, ImageFormat::Png)
        .expect("encode png");
    out.into_inner()
}

#[test]
fn folder_art_prefers_rank_reuses_cache_and_invalidates_replaced_files() {
    clear_cache();
    let dir = tempfile::tempdir().expect("art directory");
    let job = Cancellation::default();
    let decodes = Cell::new(0);
    let parse = |path: &Path, op: ParseOperation, _: &Cancellation| {
        assert_eq!(op, ParseOperation::PreviewImage);
        decodes.set(decodes.get() + 1);
        std::fs::read(path).ok()
    };
    let load = || load_folder_art(dir.path(), &job, &job, &parse).expect("folder lookup");

    assert!(load().is_none());

    // Lower-ranked folder.png is chosen when cover.* is absent; cover.jpg beats it once added.
    let folder_png = dir.path().join("folder.png");
    std::fs::write(&folder_png, png(10)).expect("write folder.png");
    let first = load().expect("folder.png cover");
    assert_eq!(decodes.get(), 1);
    let second = load().expect("cached folder.png cover");
    assert_eq!(decodes.get(), 1, "unchanged file reuses cached decode");
    assert_eq!(first.id, second.id);

    // Identical bytes decode to the same stable content-hash id.
    let copy = decode_cover(&png(10)).expect("decode copy");
    assert_eq!(first.id, copy.id);

    // Higher-ranked cover.jpg takes precedence over folder.png.
    let cover_jpg = dir.path().join("cover.jpg");
    std::fs::write(&cover_jpg, png(120)).expect("write cover.jpg");
    let higher = load().expect("cover.jpg wins over folder.png");
    assert_eq!(decodes.get(), 2);
    assert_ne!(first.id, higher.id);

    // Fenced directories are not listed.
    let activity = file_activity::begin(file_activity::OperationScope::default());
    let fence = activity.fence([dir.path().to_owned()]);
    assert!(
        load_folder_art(dir.path(), &job, &job, &parse).is_none(),
        "fenced folder returns None"
    );
    drop(fence);
    assert_eq!(decodes.get(), 2);

    // Replacing cover.jpg via rename updates the file version and invalidates the cache.
    let replacement = dir.path().join("replacement.tmp");
    std::fs::write(&replacement, png(220)).expect("write replacement");
    std::fs::rename(&replacement, &cover_jpg).expect("replace cover.jpg");
    let replaced = load().expect("replaced cover");
    assert_eq!(decodes.get(), 3);
    assert_ne!(higher.id, replaced.id);

    std::fs::remove_file(&cover_jpg).expect("remove cover.jpg");
    std::fs::remove_file(&folder_png).expect("remove folder.png");
    assert!(load().is_none());
}

#[test]
fn embedded_cover_beats_folder_art_and_null_falls_back_to_folder() {
    clear_cache();
    let dir = tempfile::tempdir().expect("album directory");
    let song = dir.path().join("track.mp3");
    std::fs::write(&song, b"audio").expect("song");
    std::fs::write(dir.path().join("cover.png"), png(30)).expect("folder cover");
    let job = Cancellation::default();
    let embedded_bytes = png(200);
    let expected_embedded_id = content_id(&embedded_bytes);
    let expected_folder_id = content_id(&png(30));

    let (cover, ok) = probe_cover(
        &song,
        Some(dir.path()),
        &job,
        &job,
        &|path, op, _| match op {
            ParseOperation::AudioCover => Some(embedded_bytes.clone()),
            ParseOperation::PreviewImage => std::fs::read(path).ok(),
            _ => None,
        },
    );
    assert!(ok);
    assert_eq!(cover.expect("embedded cover").id, expected_embedded_id);

    let (cover, ok) = probe_cover(
        &song,
        Some(dir.path()),
        &job,
        &job,
        &|path, op, _| match op {
            ParseOperation::AudioCover => Some(b"null".to_vec()),
            ParseOperation::PreviewImage => std::fs::read(path).ok(),
            _ => None,
        },
    );
    assert!(ok);
    assert_eq!(cover.expect("folder fallback").id, expected_folder_id);
}
