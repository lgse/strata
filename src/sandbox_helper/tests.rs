// SPDX-License-Identifier: MIT

use std::{
    process::Command,
    time::{Duration, Instant},
};

use gdk_pixbuf::prelude::*;

use super::{
    bounded_output, bounded_output_with_timeout, bounded_surface_dimensions, pdf_render_request,
    read_limited, render_pixbuf, render_raw, render_raw_thumbnail, render_simple_dcraw, run,
    scale_embedded_thumbnail,
};

#[test]
fn timed_bounded_commands_stop_and_report_failure_at_their_deadline() {
    let started = Instant::now();
    let result = bounded_output_with_timeout(
        Command::new("sleep").arg("5"),
        1_024,
        Duration::from_millis(50),
    );

    assert!(result.expect("run timed command").is_none());
    assert!(started.elapsed() < Duration::from_secs(2));
    let output = bounded_output_with_timeout(
        Command::new("sh").args(["-c", "printf ok"]),
        2,
        Duration::from_secs(1),
    )
    .expect("run successful command")
    .expect("command completed before timeout");
    assert!(output.status.success());
    assert_eq!(output.stdout, b"ok");

    let oversized = bounded_output_with_timeout(
        Command::new("sh").args(["-c", "head -c 1025 /dev/zero"]),
        1_024,
        Duration::from_secs(1),
    );
    assert!(oversized.is_err());
}

#[test]
fn pdf_preview_requests_carry_a_bounded_page_and_viewport() {
    assert_eq!(
        pdf_render_request("12:640x800"),
        Ok((12, crate::sandbox::PdfRenderSize::new(640, 800)))
    );
    assert_eq!(
        pdf_render_request("0:99999x1"),
        Ok((0, crate::sandbox::PdfRenderSize::new(99999, 1)))
    );
    assert!(pdf_render_request("12").is_err());
    assert!(pdf_render_request("12:0").is_err());
    assert!(pdf_render_request("page:640x800").is_err());
    assert!(pdf_render_request("12:wide").is_err());
}

#[test]
fn pdf_surface_dimensions_stay_inside_the_parent_pixel_limit() {
    let source_width = 1_000.0;
    let source_height = 1_280.0;
    let (width, height, scale) =
        bounded_surface_dimensions(source_width, source_height, 1_400.0, 1_800.0, 2_500_000.0);

    assert!(width <= 1_400);
    assert!(height <= 1_800);
    assert!(i64::from(width) * i64::from(height) <= 2_500_000);
    assert!(source_width * scale <= f64::from(width));
    assert!(source_height * scale <= f64::from(height));
}

#[test]
fn provider_output_is_bounded_without_buffering_stderr() {
    let exact = bounded_output(Command::new("sh").args(["-c", "printf 1234"]), 4)
        .expect("read output at the limit");
    assert_eq!(exact.stdout, b"1234");

    let oversized = bounded_output(
        Command::new("sh").args(["-c", "head -c 1025 /dev/zero"]),
        1024,
    );
    assert!(oversized.is_err());

    let noisy = bounded_output(
        Command::new("sh").args(["-c", "head -c 1048576 /dev/zero >&2; printf ok"]),
        2,
    )
    .expect("discard provider stderr");
    assert_eq!(noisy.stdout, b"ok");
    assert!(noisy.stderr.is_empty());
}

#[test]
fn file_reads_stop_before_exceeding_the_output_limit() {
    let directory = tempfile::tempdir().expect("tempdir");
    let path = directory.path().join("thumb.jpg");

    std::fs::write(&path, b"1234").expect("write exact");
    let exact = read_limited(std::fs::File::open(&path).expect("open exact"), 4)
        .expect("read file at the limit");
    assert_eq!(exact, b"1234");

    std::fs::write(&path, vec![0_u8; 1025]).expect("write oversized");
    assert!(read_limited(std::fs::File::open(&path).expect("open oversized"), 1024).is_err());
}

#[test]
fn embedded_thumbnails_scale_to_the_requested_size() {
    let source = gdk_pixbuf::Pixbuf::new(gdk_pixbuf::Colorspace::Rgb, false, 8, 80, 60)
        .expect("allocate thumbnail");
    source.fill(0x3366_99ff);
    let jpeg = source
        .save_to_bufferv("jpeg", &[])
        .expect("encode thumbnail");

    let png = scale_embedded_thumbnail(&jpeg, 32).expect("scale thumbnail");
    let loader = gdk_pixbuf::PixbufLoader::new();
    loader.write(&png).expect("load scaled png");
    loader.close().expect("finish scaled png");
    let scaled = loader.pixbuf().expect("decode scaled png");

    assert_eq!((scaled.width(), scaled.height()), (32, 24));
}

#[test]
fn image_previews_preserve_small_sources_and_bound_large_decodes() {
    let directory = tempfile::tempdir().expect("image fixture");
    let path = directory.path().join("image.png");
    for (width, height, expected) in [(80, 40, (80, 40)), (1200, 600, (800, 400))] {
        let source = gdk_pixbuf::Pixbuf::new(gdk_pixbuf::Colorspace::Rgb, false, 8, width, height)
            .expect("source image");
        source.fill(0x3366_99ff);
        source.savev(&path, "png", &[]).expect("save source");
        let png = render_raw(&path, 800).expect("render image preview");
        let loader = gdk_pixbuf::PixbufLoader::new();
        loader.write(&png).expect("load preview");
        loader.close().expect("finish preview");
        let preview = loader.pixbuf().expect("decoded preview");
        assert_eq!((preview.width(), preview.height()), expected);
    }
}

#[test]
fn preview_image_uses_raw_fallbacks() {
    let directory = tempfile::tempdir().expect("tempdir");
    let input = directory.path().join("photo.ARW");
    let output = directory.path().join("result.png");
    std::fs::write(&input, b"not a camera file").expect("write stub");

    let pixbuf = render_pixbuf(&input, 800).expect_err("stub must fail pixbuf");
    let raw = render_raw(&input, 800);
    let preview = run(&[
        "preview-image".into(),
        input.to_string_lossy().into_owned(),
        output.to_string_lossy().into_owned(),
        "800".into(),
        "software".into(),
    ]);

    match raw {
        Ok(_) => preview.expect("preview-image should use RAW fallbacks"),
        Err(raw) => {
            assert_ne!(pixbuf, raw);
            assert_eq!(preview.expect_err("stub should fail RAW fallbacks"), raw);
        }
    }
}

#[test]
fn concurrent_raw_fallbacks_do_not_share_staging_files() {
    let directory = tempfile::tempdir().expect("tempdir");
    let input = directory.path().join("photo.ARW");
    std::fs::write(&input, b"not a camera file").expect("write stub");
    let expected = render_simple_dcraw(&input, 256).expect_err("invalid RAW file");

    std::thread::scope(|scope| {
        let workers: Vec<_> = (0..8)
            .map(|_| scope.spawn(|| render_simple_dcraw(&input, 256)))
            .collect();
        for worker in workers {
            assert_eq!(worker.join().expect("worker"), Err(expected.clone()));
        }
    });
}

#[test]
fn thumbnail_raw_uses_embedded_preview_fallbacks() {
    let directory = tempfile::tempdir().expect("tempdir");
    let input = directory.path().join("photo.ARW");
    let output = directory.path().join("result.png");
    std::fs::write(&input, b"not a camera file").expect("write stub");

    let pixbuf = render_pixbuf(&input, 256).expect_err("stub must fail pixbuf");
    let thumbnail = render_raw_thumbnail(&input, 256);
    let helper = run(&[
        "thumbnail-raw".into(),
        input.to_string_lossy().into_owned(),
        output.to_string_lossy().into_owned(),
        "256".into(),
        "software".into(),
    ]);

    match thumbnail {
        Ok(_) => helper.expect("thumbnail-raw should use embedded preview fallbacks"),
        Err(thumbnail) => {
            assert_ne!(pixbuf, thumbnail);
            assert_eq!(
                helper.expect_err("stub should fail RAW fallbacks"),
                thumbnail
            );
        }
    }
}

fn archive_list_output(
    directory: &tempfile::TempDir,
    input: &std::path::Path,
    format: &str,
    password: Option<&[u8]>,
) -> Vec<u8> {
    let output = directory.path().join("result.archive.json");
    let secret =
        password.map(|password| crate::sandbox::stage_secret_anon(password).expect("stage secret"));
    let mut arguments = vec![
        "archive-list".to_owned(),
        input.to_string_lossy().into_owned(),
        output.to_string_lossy().into_owned(),
        format.to_owned(),
        "software".to_owned(),
    ];
    if let Some(secret) = &secret {
        use std::os::fd::AsRawFd;
        arguments.push(secret.as_raw_fd().to_string());
    }
    run(&arguments).expect("helper runs");
    std::fs::read(&output).expect("read listing output")
}

fn decoded_archive_list(output: &[u8]) -> Result<crate::adapters::ArchiveListing, String> {
    crate::adapters::decode_archive_listing(output)
}

fn write_tar_fixture(path: &std::path::Path, members: &[(&str, &[u8])]) {
    let file = std::fs::File::create(path).expect("create tar");
    let mut builder = tar::Builder::new(file);
    for (name, contents) in members {
        let mut header = tar::Header::new_gnu();
        header.set_size(contents.len() as u64);
        header.set_entry_type(tar::EntryType::Regular);
        header.set_mode(0o644);
        builder
            .append_data(&mut header, name, *contents)
            .expect("append member");
    }
    builder.into_inner().expect("finish tar");
}

fn write_encrypted_zip_fixture(
    directory: &tempfile::TempDir,
    name: &str,
    password: &str,
) -> std::path::PathBuf {
    use crate::{adapters::write_compression_fixture, services::ArchiveFormat};

    let source = directory.path().join("folder");
    std::fs::create_dir_all(&source).expect("create source");
    std::fs::write(source.join("item.txt"), b"contents").expect("write source");
    let path = directory.path().join(name);
    write_compression_fixture(&path, &[source], ArchiveFormat::Zip, Some(password))
        .expect("write encrypted zip");
    path
}

#[test]
fn archive_list_lists_plain_tar_members() {
    use crate::adapters::ArchiveListingStatus;

    let directory = tempfile::tempdir().expect("tempdir");
    let input = directory.path().join("docs.tar");
    write_tar_fixture(
        &input,
        &[("readme.txt", b"hi"), ("src/main.rs", b"fn main(){}")],
    );
    let output = archive_list_output(&directory, &input, "tar", None);
    let listing = decoded_archive_list(&output).expect("decode listing");
    assert_eq!(listing.status, ArchiveListingStatus::Open);
    assert_eq!(
        listing
            .entries
            .iter()
            .map(|entry| entry.name.clone())
            .collect::<Vec<_>>(),
        vec!["readme.txt".to_owned(), "src/main.rs".to_owned()]
    );
}

#[test]
fn archive_list_needs_password_without_one() {
    use crate::adapters::ArchiveListingStatus;

    let directory = tempfile::tempdir().expect("tempdir");
    let input = write_encrypted_zip_fixture(&directory, "secret.zip", "s3cret");
    let output = archive_list_output(&directory, &input, "zip", None);
    let listing = decoded_archive_list(&output).expect("decode listing");
    assert_eq!(listing.status, ArchiveListingStatus::NeedsPassword);
    assert!(
        !listing.entries.is_empty(),
        "encrypted zip names stay listable without a password"
    );
}

#[test]
fn archive_list_unlocks_with_staged_password() {
    use crate::adapters::ArchiveListingStatus;

    let directory = tempfile::tempdir().expect("tempdir");
    let password = "päss wörd $HOME `id`!";
    let input = write_encrypted_zip_fixture(&directory, "secret.zip", password);
    let output = archive_list_output(&directory, &input, "zip", Some(password.as_bytes()));
    let listing = decoded_archive_list(&output).expect("decode listing");
    assert_eq!(listing.status, ArchiveListingStatus::Open);
    assert!(
        listing
            .entries
            .iter()
            .any(|entry| entry.name == "folder/item.txt"),
        "unicode/spaces/metacharacter password round-trips byte-exactly"
    );
}

#[test]
fn archive_list_rejects_wrong_and_empty_passwords() {
    use crate::adapters::ArchiveListingStatus;

    let directory = tempfile::tempdir().expect("tempdir");
    let input = write_encrypted_zip_fixture(&directory, "secret.zip", "s3cret");
    for password in ["wrong", ""] {
        let output = archive_list_output(&directory, &input, "zip", Some(password.as_bytes()));
        let listing = decoded_archive_list(&output).expect("decode listing");
        assert_eq!(
            listing.status,
            ArchiveListingStatus::WrongPassword,
            "password {password:?} must be rejected"
        );
    }
}

#[test]
fn archive_list_rejects_unknown_format() {
    let directory = tempfile::tempdir().expect("tempdir");
    let input = directory.path().join("docs.tar");
    write_tar_fixture(&input, &[("readme.txt", b"hi")]);
    let output = directory.path().join("result.archive.json");
    let error = run(&[
        "archive-list".to_owned(),
        input.to_string_lossy().into_owned(),
        output.to_string_lossy().into_owned(),
        "rar".to_owned(),
        "software".to_owned(),
    ])
    .expect_err("unknown format must fail");
    assert_eq!(error, "Unknown archive format for preview.");
    assert!(
        !output.exists(),
        "rejected formats must not produce an output file"
    );
}

#[test]
fn archive_list_rejects_overlong_password() {
    let directory = tempfile::tempdir().expect("tempdir");
    let input = directory.path().join("docs.tar");
    write_tar_fixture(&input, &[("readme.txt", b"hi")]);
    let output = directory.path().join("result.archive.json");
    let secret = crate::sandbox::stage_secret_anon(&vec![
        b'x';
        crate::adapters::MAX_ARCHIVE_PASSWORD_BYTES
            + 1
    ])
    .expect("stage password");
    use std::os::fd::AsRawFd;
    let error = run(&[
        "archive-list".to_owned(),
        input.to_string_lossy().into_owned(),
        output.to_string_lossy().into_owned(),
        "tar".to_owned(),
        "software".to_owned(),
        secret.as_raw_fd().to_string(),
    ])
    .expect_err("overlong password must fail");
    assert_eq!(error, "Archive password is too long.");
}

#[test]
fn archive_list_rejects_non_utf8_password() {
    let directory = tempfile::tempdir().expect("tempdir");
    let input = directory.path().join("docs.tar");
    write_tar_fixture(&input, &[("readme.txt", b"hi")]);
    let output = directory.path().join("result.archive.json");
    let secret = crate::sandbox::stage_secret_anon(&[0xFF, 0xFE]).expect("stage password");
    use std::os::fd::AsRawFd;
    let error = run(&[
        "archive-list".to_owned(),
        input.to_string_lossy().into_owned(),
        output.to_string_lossy().into_owned(),
        "tar".to_owned(),
        "software".to_owned(),
        secret.as_raw_fd().to_string(),
    ])
    .expect_err("non-UTF8 password must fail");
    assert_eq!(error, "Archive password is not valid text.");
}

#[test]
fn archive_list_rejects_an_unreadable_secret_descriptor() {
    let directory = tempfile::tempdir().expect("tempdir");
    let input = directory.path().join("docs.tar");
    write_tar_fixture(&input, &[("readme.txt", b"hi")]);
    let output = directory.path().join("result.archive.json");
    let number = i32::MAX;
    let error = run(&[
        "archive-list".to_owned(),
        input.to_string_lossy().into_owned(),
        output.to_string_lossy().into_owned(),
        "tar".to_owned(),
        "software".to_owned(),
        number.to_string(),
    ])
    .expect_err("unreadable secret must fail");
    assert!(
        error.starts_with("Unable to read the preview secret: "),
        "unexpected error: {error:?}"
    );
    assert!(
        !output.exists(),
        "failed listings must not produce an output file, got {error:?}"
    );
}

#[test]
fn archive_list_rejects_a_malformed_secret_descriptor() {
    let directory = tempfile::tempdir().expect("tempdir");
    let input = directory.path().join("docs.tar");
    write_tar_fixture(&input, &[("readme.txt", b"hi")]);
    let output = directory.path().join("result.archive.json");
    for descriptor in ["not-a-number", "-1"] {
        let error = run(&[
            "archive-list".to_owned(),
            input.to_string_lossy().into_owned(),
            output.to_string_lossy().into_owned(),
            "tar".to_owned(),
            "software".to_owned(),
            descriptor.to_owned(),
        ])
        .expect_err("malformed descriptor must fail");
        assert_eq!(error, "Invalid preview helper secret descriptor");
    }
}

#[test]
fn archive_list_rejects_a_trailing_argument_on_other_operations() {
    let error = run(&[
        "preview-image".to_owned(),
        "/tmp/input".to_owned(),
        "/tmp/output.png".to_owned(),
        "800".to_owned(),
        "software".to_owned(),
        "3".to_owned(),
    ])
    .expect_err("extra argument must fail");
    assert_eq!(error, "Invalid preview helper arguments");
}

#[test]
fn archive_list_reports_corrupt_input_without_crashing() {
    let directory = tempfile::tempdir().expect("tempdir");
    let input = directory.path().join("junk.zip");
    std::fs::write(&input, b"not an archive at all").expect("write junk");
    let output = archive_list_output(&directory, &input, "zip", None);
    match decoded_archive_list(&output) {
        Err(message) => assert_eq!(message, crate::adapters::INVALID_ARCHIVE),
        Ok(_) => panic!("corrupt input must not list"),
    }
}
