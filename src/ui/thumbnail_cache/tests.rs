// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    ffi::OsString,
    os::unix::{
        ffi::OsStringExt,
        fs::{PermissionsExt, symlink},
    },
    path::{Path, PathBuf},
};

use super::{
    cache_key, ensure_cache_dir, glib_md5_hex, lookup, normalize_to_canonical, png_dimensions,
    read_thumb_tags, set_cache_dir_override, shared_cache_dir, store,
};
use crate::test_support::TestMutex;

static CACHE_TEST_LOCK: TestMutex = TestMutex::new();

struct BucketGuard {
    _lock: std::sync::MutexGuard<'static, ()>,
    previous_umask: Option<u32>,
}

impl BucketGuard {
    fn unique(label: &str) -> (Self, PathBuf) {
        let lock = CACHE_TEST_LOCK
            .lock()
            .expect("the cache test lock should not be poisoned");
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("the system clock should be after the Unix epoch")
            .as_nanos();
        let bucket = std::env::temp_dir().join(format!(
            "strata-thumb-cache-{label}-{unique}-{}",
            std::process::id()
        ));
        let bucket = bucket.join("thumbnails").join("large");
        set_cache_dir_override(Some(bucket.clone()));
        (
            Self {
                _lock: lock,
                previous_umask: None,
            },
            bucket,
        )
    }

    fn permissive_umask(&mut self) {
        let previous = rustix::process::umask(rustix::fs::Mode::from_bits_truncate(0o022));
        self.previous_umask = Some(previous.bits());
    }
}

impl Drop for BucketGuard {
    fn drop(&mut self) {
        set_cache_dir_override(None);
        if let Some(mask) = self.previous_umask.take() {
            rustix::process::umask(rustix::fs::Mode::from_bits_truncate(mask));
        }
    }
}

fn unique_source(label: &str) -> PathBuf {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("the system clock should be after the Unix epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "strata-thumb-source-{label}-{unique}-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("the source dir should exist");
    dir
}

fn solid_png(width: i32, height: i32) -> Vec<u8> {
    let pixbuf =
        gtk::gdk_pixbuf::Pixbuf::new(gtk::gdk_pixbuf::Colorspace::Rgb, false, 8, width, height)
            .expect("a solid test pixbuf should allocate");
    pixbuf.fill(0x80402000);
    pixbuf
        .save_to_bufferv("png", &[])
        .expect("a solid test pixbuf should encode")
        .to_vec()
}

fn stored_dimensions(bucket: &Path, path: &Path) -> Option<(u32, u32)> {
    let (_, name) = cache_key(path)?;
    let bytes = std::fs::read(bucket.join(name)).ok()?;
    png_dimensions(&bytes)
}

fn glib_md5(input: &str) -> String {
    glib_md5_hex(input.as_bytes()).expect("GLib should hash the vector")
}

#[test]
fn md5_matches_reference_vectors() {
    let vectors = [
        ("", "d41d8cd98f00b204e9800998ecf8427e"),
        ("a", "0cc175b9c0f1b6a831c399e269772661"),
        ("abc", "900150983cd24fb0d6963f7d28e17f72"),
        ("message digest", "f96b697d7cb7938d525a2f31aaf161d0"),
        (
            "abcdefghijklmnopqrstuvwxyz",
            "c3fcd3d76192e4007dfb496cca67e13b",
        ),
        (
            "file:///home/kaperala/picture.jpg",
            "aca2162ac461307f56a89d41dccb4fde",
        ),
    ];
    for (input, expected) in vectors {
        assert_eq!(glib_md5(input), expected, "input {input:?}");
    }
}

#[test]
fn tag_reader_rejects_non_png_and_truncation() {
    assert_eq!(read_thumb_tags(b"definitely not a png"), None);
    assert_eq!(read_thumb_tags(b"\x89PNG\r\n\x1a\n"), None);
    assert_eq!(read_thumb_tags(b"\x89PNG\r\n\x1a\n\x00\x00"), None);
}

#[test]
fn tag_reader_reads_a_minimal_tagged_png() {
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    png.extend_from_slice(&[0, 0, 0, 0]);
    png.extend_from_slice(b"IHDR");
    png.extend_from_slice(&[0, 0, 0, 0]);
    let text = b"Thumb::URI\0file:///x/y.png";
    png.extend_from_slice(&(text.len() as u32).to_be_bytes());
    png.extend_from_slice(b"tEXt");
    png.extend_from_slice(text);
    png.extend_from_slice(&[0, 0, 0, 0]);
    let mut mtime = b"Thumb::MTime\0".to_vec();
    mtime.extend_from_slice(b"12345678");
    png.extend_from_slice(&(mtime.len() as u32).to_be_bytes());
    png.extend_from_slice(b"tEXt");
    png.extend_from_slice(&mtime);
    png.extend_from_slice(&[0, 0, 0, 0]);
    png.extend_from_slice(&[0, 0, 0, 0]);
    png.extend_from_slice(b"IEND");
    png.extend_from_slice(&[0, 0, 0, 0]);
    assert_eq!(
        read_thumb_tags(&png),
        Some(("file:///x/y.png".to_owned(), "12345678".to_owned()))
    );
}

#[test]
fn stored_entries_are_canonical_large() {
    let (_guard, bucket) = BucketGuard::unique("canonical");
    let source = unique_source("canonical");
    let path = source.join("photo.jpg");
    std::fs::write(&path, b"source").expect("the fixture should be written");

    store(&path, 111, &solid_png(300, 200));
    assert_eq!(stored_dimensions(&bucket, &path), Some((256, 171)));

    store(&path, 111, &solid_png(20, 10));
    assert_eq!(stored_dimensions(&bucket, &path), Some((256, 128)));
    assert!(lookup(&path, 111).is_some());
}

#[test]
fn stale_entries_are_misses() {
    let (_guard, _bucket) = BucketGuard::unique("stale");
    let source = unique_source("stale");
    let path = source.join("photo.jpg");
    std::fs::write(&path, b"source").expect("the fixture should be written");
    store(&path, 111, &solid_png(64, 64));
    assert!(lookup(&path, 111).is_some());
    assert_eq!(lookup(&path, 222), None);

    let other = source.join("other.jpg");
    std::fs::write(&other, b"source").expect("the fixture should be written");
    let (_, name) = cache_key(&other).expect("the other path should key");
    let dir = shared_cache_dir().expect("the cache dir should resolve");
    let ours = {
        let (_, name) = cache_key(&path).expect("the path should key");
        std::fs::read(dir.join(name)).expect("our entry should exist")
    };
    std::fs::create_dir_all(&dir).expect("the cache dir should exist");
    std::fs::write(dir.join(name), ours).expect("the foreign copy should be written");
    assert_eq!(lookup(&other, 111), None);
}

#[test]
fn oversized_and_sparse_entries_are_misses_without_huge_reads() {
    let (_guard, bucket) = BucketGuard::unique("oversized");
    std::fs::create_dir_all(&bucket).expect("the bucket should exist");
    let source = unique_source("oversized");

    let path = source.join("huge.jpg");
    std::fs::write(&path, b"source").expect("the fixture should be written");
    let (_, name) = cache_key(&path).expect("the path should key");
    std::fs::write(bucket.join(&name), vec![0x41u8; 3 * 1024 * 1024])
        .expect("the oversized entry should be written");
    assert_eq!(lookup(&path, 111), None);

    let sparse = source.join("sparse.jpg");
    std::fs::write(&sparse, b"source").expect("the fixture should be written");
    let (_, sparse_name) = cache_key(&sparse).expect("the sparse path should key");
    let file =
        std::fs::File::create(bucket.join(&sparse_name)).expect("the sparse entry is created");
    file.set_len(100 * 1024 * 1024)
        .expect("the sparse entry should be sized");
    drop(file);
    assert_eq!(lookup(&sparse, 111), None);
}

#[test]
fn fifo_symlink_and_malformed_entries_are_safe_misses() {
    let (_guard, bucket) = BucketGuard::unique("hostile");
    std::fs::create_dir_all(&bucket).expect("the bucket should exist");
    let source = unique_source("hostile");

    let fifo_path = source.join("fifo.jpg");
    std::fs::write(&fifo_path, b"source").expect("the fixture should be written");
    let (_, fifo_name) = cache_key(&fifo_path).expect("the fifo path should key");
    rustix::fs::mkfifoat(
        rustix::fs::CWD,
        bucket.join(&fifo_name),
        rustix::fs::Mode::from_bits_truncate(0o600),
    )
    .expect("the fifo should be created");
    assert_eq!(lookup(&fifo_path, 111), None);

    let canary = source.join("canary.txt");
    std::fs::write(&canary, b"canary").expect("the canary should be written");
    let link_path = source.join("linked.jpg");
    std::fs::write(&link_path, b"source").expect("the fixture should be written");
    let (_, link_name) = cache_key(&link_path).expect("the link path should key");
    symlink(&canary, bucket.join(&link_name)).expect("the symlink should be created");
    assert_eq!(lookup(&link_path, 111), None);
    assert_eq!(
        std::fs::read(&canary).expect("the canary should survive"),
        b"canary"
    );
    store(&link_path, 111, &solid_png(64, 64));
    assert_eq!(
        std::fs::read(&canary).expect("the canary should survive the store"),
        b"canary"
    );

    let bad_path = source.join("bad.jpg");
    std::fs::write(&bad_path, b"source").expect("the fixture should be written");
    let (_, bad_name) = cache_key(&bad_path).expect("the bad path should key");
    std::fs::write(bucket.join(&bad_name), b"definitely not a png")
        .expect("the garbage should be written");
    assert_eq!(lookup(&bad_path, 111), None);
    std::fs::write(bucket.join(&bad_name), b"\x89PNG\r\n\x1a\n\x00\x00")
        .expect("the truncation should be written");
    assert_eq!(lookup(&bad_path, 111), None);
    let mut extreme = b"\x89PNG\r\n\x1a\n".to_vec();
    extreme.extend_from_slice(&13u32.to_be_bytes());
    extreme.extend_from_slice(b"IHDR");
    extreme.extend_from_slice(&100_000u32.to_be_bytes());
    extreme.extend_from_slice(&100_000u32.to_be_bytes());
    extreme.extend_from_slice(&[8, 2, 0, 0, 0]);
    extreme.extend_from_slice(&[0, 0, 0, 0]);
    std::fs::write(bucket.join(&bad_name), &extreme).expect("the extreme entry is written");
    assert_eq!(lookup(&bad_path, 111), None);
}

#[test]
fn bucket_and_entries_keep_strict_modes_under_a_permissive_umask() {
    let (mut guard, bucket) = BucketGuard::unique("modes");
    guard.permissive_umask();
    let source = unique_source("modes");
    let path = source.join("photo.jpg");
    std::fs::write(&path, b"source").expect("the fixture should be written");
    store(&path, 111, &solid_png(64, 64));

    let dir_mode = std::fs::metadata(&bucket)
        .expect("the bucket should exist")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(dir_mode, 0o700);
    let (_, name) = cache_key(&path).expect("the path should key");
    let file_mode = std::fs::metadata(bucket.join(name))
        .expect("the entry should exist")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(file_mode, 0o600);
}

#[test]
fn uri_keys_cover_tricky_names() {
    let (_guard, _bucket) = BucketGuard::unique("urikeys");
    let source = unique_source("urikeys");
    let tricky = [
        "sp ace.jpg",
        "hash#tag.jpg",
        "what?.jpg",
        "at@home.jpg",
        "ünïcödé.jpg",
    ];
    for name in tricky {
        let path = source.join(name);
        std::fs::write(&path, b"source").expect("the fixture should be written");
        let (uri, key_name) = cache_key(&path).expect("the tricky name should key");
        assert_eq!(format!("{}.png", glib_md5(&uri)), key_name);
        store(&path, 111, &solid_png(64, 64));
        assert!(
            lookup(&path, 111).is_some(),
            "name {name:?} should round-trip"
        );
        assert_eq!(lookup(&path, 222), None);
    }

    let raw_path = source.join(OsString::from_vec(b"raw-\xff.jpg".to_vec()));
    std::fs::write(&raw_path, b"source").expect("the raw fixture should be written");
    store(&raw_path, 111, &solid_png(64, 64));
    let hit = lookup(&raw_path, 111);
    assert!(hit.is_some() || cache_key(&raw_path).is_none());
}

#[test]
fn normalizer_rejects_degenerate_renders() {
    assert!(normalize_to_canonical(b"", "file:///x.jpg", 1).is_err());
    assert!(normalize_to_canonical(b"not a png", "file:///x.jpg", 1).is_err());
    assert!(ensure_cache_dir(&PathBuf::from("/proc/strata-nope/thumbnails")).is_err());
}
