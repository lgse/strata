// SPDX-License-Identifier: MIT

use std::{
    process::Command,
    time::{Duration, Instant},
};

use gdk_pixbuf::prelude::*;

use super::{
    bounded_output, bounded_output_with_timeout, bounded_surface_dimensions, read_limited,
    render_pixbuf, render_raw, render_raw_thumbnail, render_simple_dcraw, run,
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

#[test]
fn renders_requested_pdf_pages_within_the_pixel_budget() {
    let path = std::env::temp_dir().join(format!(
        "strata-preview-{}-{}.pdf",
        std::process::id(),
        std::thread::current().name().unwrap_or("test")
    ));
    let surface = cairo::PdfSurface::new(612.0, 792.0, &path).expect("create PDF surface");
    {
        let context = cairo::Context::new(&surface).expect("create PDF context");
        context.set_source_rgb(0.2, 0.4, 0.8);
        context.paint().expect("paint PDF page");
        context.show_page().expect("finish first PDF page");
        context.set_source_rgb(0.8, 0.4, 0.2);
        context.paint().expect("paint second PDF page");
        context.show_page().expect("finish second PDF page");
    }
    surface.finish();

    let output_directory = path.with_extension("output");
    std::fs::create_dir(&output_directory).expect("create output directory");
    let output = output_directory.join("result.png");
    run(&[
        "preview-pdf".to_owned(),
        path.to_string_lossy().into_owned(),
        output.to_string_lossy().into_owned(),
        "1".to_owned(),
        "software".to_owned(),
    ])
    .expect("render second PDF page");
    let png = std::fs::read(&output).expect("read rendered page");
    let metadata =
        std::fs::read_to_string(output_directory.join("result.meta")).expect("read PDF metadata");
    let _removed = std::fs::remove_file(path);
    let _removed = std::fs::remove_dir_all(output_directory);

    assert_eq!(metadata, "1 2");
    assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
}

