// SPDX-License-Identifier: MIT

use std::{
    io::Write,
    os::fd::AsRawFd,
    path::Path,
    process::Command,
    time::{Duration, Instant},
};

use gdk_pixbuf::prelude::*;

use super::{
    audio, books, bounded_output, bounded_output_with_timeout, bounded_surface_dimensions,
    embedded, pdf_render_request, read_limited, render_pixbuf, render_raw, render_raw_thumbnail,
    render_simple_dcraw, run, scale_embedded_thumbnail, text,
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

fn fixture_png(width: i32, height: i32, fill: u32) -> Vec<u8> {
    let source = gdk_pixbuf::Pixbuf::new(gdk_pixbuf::Colorspace::Rgb, false, 8, width, height)
        .expect("fixture image");
    source.fill(fill);
    source.save_to_bufferv("png", &[]).expect("encode fixture")
}

fn decode_png(bytes: &[u8]) -> gdk_pixbuf::Pixbuf {
    let loader = gdk_pixbuf::PixbufLoader::new();
    loader.write(bytes).expect("load png");
    loader.close().expect("finish png");
    loader.pixbuf().expect("decoded png")
}

fn write_zip(path: &Path, entries: &[(&str, Vec<u8>)]) {
    let mut writer = zip::ZipWriter::new(std::fs::File::create(path).expect("create zip"));
    for (name, bytes) in entries {
        writer
            .start_file(*name, zip::write::SimpleFileOptions::default())
            .expect("start entry");
        writer.write_all(bytes).expect("write entry");
    }
    writer.finish().expect("finish zip");
}

#[test]
fn embedded_prefers_saved_document_thumbnails() {
    let directory = tempfile::tempdir().expect("tempdir");
    let input = directory.path().join("report.docx");
    write_zip(
        &input,
        &[
            ("word/media/banner.png", fixture_png(80, 40, 0x3366_99ff)),
            ("docProps/thumbnail.png", fixture_png(64, 64, 0x9933_66ff)),
        ],
    );

    let png = embedded::render(&input, 256).expect("render embedded thumbnail");
    let image = decode_png(&png);
    assert_eq!((image.width(), image.height()), (64, 64));
}

#[test]
fn embedded_renders_document_content_pages() {
    let directory = tempfile::tempdir().expect("tempdir");
    let input = directory.path().join("report.docx");
    write_zip(
        &input,
        &[
            (
                "word/document.xml",
                br#"<w:body><w:p><w:r><w:t>Annual report opening paragraph with enough content to fill a preview page.</w:t></w:r></w:p><w:p><w:r><w:t>Second paragraph of the document body.</w:t></w:r></w:p></w:body>"#.to_vec(),
            ),
            ("docProps/thumbnail.png", fixture_png(64, 64, 0x9933_66ff)),
        ],
    );

    let png = embedded::render(&input, 256).expect("render document content");
    let image = decode_png(&png);
    assert_eq!((image.width(), image.height()), (192, 256));
}

#[test]
fn embedded_prefers_cover_named_and_first_images() {
    let directory = tempfile::tempdir().expect("tempdir");
    let comic = directory.path().join("issue.cbz");
    write_zip(
        &comic,
        &[
            ("page02.png", fixture_png(64, 64, 0x9933_66ff)),
            ("page01.png", fixture_png(80, 40, 0x3366_99ff)),
        ],
    );
    let png = embedded::render(&comic, 256).expect("render comic page");
    assert_eq!(
        (decode_png(&png).width(), decode_png(&png).height()),
        (80, 40)
    );

    let book = directory.path().join("novel.epub");
    write_zip(
        &book,
        &[
            ("OEBPS/aaa.png", fixture_png(80, 40, 0x3366_99ff)),
            ("OEBPS/cover.png", fixture_png(64, 64, 0x9933_66ff)),
        ],
    );
    let png = embedded::render(&book, 256).expect("render epub cover");
    assert_eq!(
        (decode_png(&png).width(), decode_png(&png).height()),
        (64, 64)
    );
}

#[test]
fn embedded_rejects_files_without_image_entries() {
    let directory = tempfile::tempdir().expect("tempdir");
    let input = directory.path().join("plain.txt");
    std::fs::write(&input, b"not an archive").expect("write stub");
    assert!(embedded::render(&input, 256).is_err());

    let empty = directory.path().join("empty.docx");
    write_zip(&empty, &[("word/document.xml", b"<doc/>".to_vec())]);
    assert!(embedded::render(&empty, 256).is_err());
}

#[test]
fn embedded_rejects_contentless_packages() {
    let directory = tempfile::tempdir().expect("tempdir");
    let input = directory.path().join("fallback.pptx");
    write_zip(
        &input,
        &[
            ("[Content_Types].xml", b"<Types/>".to_vec()),
            ("ppt/presentation.xml", b"<p:presentation/>".to_vec()),
        ],
    );

    assert!(embedded::render(&input, 256).is_err());
}

fn wav_fixture(seconds: u32) -> Vec<u8> {
    let sample_rate = 8_000_u32;
    let samples = sample_rate * seconds;
    let data_bytes = samples * 2;
    let mut wav = Vec::with_capacity((44 + data_bytes) as usize);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_bytes).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16_u32.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes());
    wav.extend_from_slice(&sample_rate.to_le_bytes());
    wav.extend_from_slice(&(sample_rate * 2).to_le_bytes());
    wav.extend_from_slice(&2_u16.to_le_bytes());
    wav.extend_from_slice(&16_u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_bytes.to_le_bytes());
    for index in 0..samples {
        let envelope = i32::try_from(index % sample_rate).expect("sample offset") - 4_000;
        let sample = if index % 2 == 0 {
            envelope * 6
        } else {
            -envelope * 6
        };
        wav.extend_from_slice(&(sample.clamp(-24_000, 24_000) as i16).to_le_bytes());
    }
    wav
}

#[test]
fn audio_thumbnails_render_a_five_second_spectrum_mask() {
    if Command::new("ffmpeg").arg("-version").output().is_err() {
        eprintln!("skipping audio spectrum test: ffmpeg is not installed");
        return;
    }
    let directory = tempfile::tempdir().expect("tempdir");
    let input = directory.path().join("voice.wav");
    std::fs::write(&input, wav_fixture(6)).expect("write wav");

    let png = audio::render(&input, 256).expect("render audio spectrum");
    let image = decode_png(&png);
    assert_eq!((image.width(), image.height()), (256, 256));
    assert!(image.has_alpha());
    let pixels = image.read_pixel_bytes();
    let channels = image.n_channels() as usize;
    assert!(
        pixels
            .as_ref()
            .chunks_exact(channels)
            .any(|pixel| pixel[3] == 0)
    );
    assert!(
        pixels
            .as_ref()
            .chunks_exact(channels)
            .any(|pixel| pixel[3] > 0)
    );
}

#[test]
fn audio_thumbnails_reject_unreadable_audio() {
    let directory = tempfile::tempdir().expect("tempdir");
    let input = directory.path().join("notes.txt");
    std::fs::write(&input, b"no audio here").expect("write stub");

    assert!(audio::render(&input, 256).is_err());
}

#[test]
fn books_extract_fb2_covers() {
    let directory = tempfile::tempdir().expect("tempdir");
    let input = directory.path().join("novel.fb2");
    let cover = fixture_png(64, 64, 0x9933_66ff);
    let fb2 = format!(
        r##"<?xml version="1.0"?>
<FictionBook><description><title-info><coverpage><image xlink:href="#cover.png"/></coverpage></title-info></description>
<body><section><p>Text</p></section></body>
<binary id="cover.png" content-type="image/png">{}</binary></FictionBook>"##,
        base64_encode(&cover)
    );
    std::fs::write(&input, fb2).expect("write fb2");

    let png = books::render(&input, 256).expect("render fb2 cover");
    assert_eq!(
        (decode_png(&png).width(), decode_png(&png).height()),
        (64, 64)
    );

    let text_only = directory.path().join("plain.fb2");
    std::fs::write(
        &text_only,
        "<FictionBook><body><p>no cover</p></body></FictionBook>",
    )
    .expect("write coverless fb2");
    assert!(books::render(&text_only, 256).is_err());
}

fn base64_encode(data: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut output = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let value = chunk
            .iter()
            .fold(0u32, |acc, byte| (acc << 8) | u32::from(*byte))
            << (8 * (3 - chunk.len()));
        for index in 0..4 {
            let shift = 18 - index * 6;
            output.push(if index < chunk.len() + 1 {
                ALPHABET[(value >> shift) as usize & 63] as char
            } else {
                '='
            });
        }
    }
    output
}

#[test]
fn books_extract_mobi_covers() {
    let directory = tempfile::tempdir().expect("tempdir");
    let input = directory.path().join("novel.mobi");
    let cover = fixture_png(64, 64, 0x9933_66ff);

    // Minimal PalmDB: header, 2-entry record table, MOBI+EXTH record, cover record.
    let mobi_length = 232usize;
    let mut record0 = vec![0u8; 16 + mobi_length + 12 + 12];
    record0[16..20].copy_from_slice(b"MOBI");
    record0[20..24].copy_from_slice(&(mobi_length as u32).to_be_bytes());
    record0[108..112].copy_from_slice(&1u32.to_be_bytes()); // first image = record 1
    record0[128..132].copy_from_slice(&0x40u32.to_be_bytes()); // EXTH present
    let exth = 16 + mobi_length;
    record0[exth..exth + 4].copy_from_slice(b"EXTH");
    record0[exth + 4..exth + 8].copy_from_slice(&24u32.to_be_bytes());
    record0[exth + 8..exth + 12].copy_from_slice(&1u32.to_be_bytes());
    record0[exth + 12..exth + 16].copy_from_slice(&201u32.to_be_bytes());
    record0[exth + 16..exth + 20].copy_from_slice(&12u32.to_be_bytes());
    record0[exth + 20..exth + 24].copy_from_slice(&0u32.to_be_bytes()); // coveroffset

    let record0_offset = 78 + 16;
    let record1_offset = record0_offset + record0.len();
    let mut file = vec![0u8; 78];
    file[60..68].copy_from_slice(b"BOOKMOBI");
    file[76..78].copy_from_slice(&2u16.to_be_bytes());
    file.extend_from_slice(&(record0_offset as u32).to_be_bytes());
    file.extend_from_slice(&[0; 4]);
    file.extend_from_slice(&(record1_offset as u32).to_be_bytes());
    file.extend_from_slice(&[0; 4]);
    file.extend_from_slice(&record0);
    file.extend_from_slice(&cover);
    std::fs::write(&input, file).expect("write mobi");

    let png = books::render(&input, 256).expect("render mobi cover");
    assert_eq!(
        (decode_png(&png).width(), decode_png(&png).height()),
        (64, 64)
    );
}

#[test]
fn books_render_djvu_first_pages() {
    if Command::new("cjb2")
        .arg("-version")
        .output()
        .map(|output| !output.status.success())
        .unwrap_or(true)
    {
        eprintln!("skipping djvu test: cjb2/ddjvu not installed");
        return;
    }
    let directory = tempfile::tempdir().expect("tempdir");
    let input = directory.path().join("scan.djvu");
    let pbm = directory.path().join("page.pbm");
    std::fs::write(
        &pbm,
        b"P4\n16 16\n"
            .iter()
            .chain([0xffu8; 32].iter())
            .copied()
            .collect::<Vec<u8>>(),
    )
    .expect("write pbm");
    assert!(
        Command::new("cjb2")
            .arg(&pbm)
            .arg(&input)
            .status()
            .expect("run cjb2")
            .success()
    );

    let png = books::render(&input, 256).expect("render djvu page");
    let image = decode_png(&png);
    assert!(image.width() > 0 && image.height() > 0);
}

#[test]
fn empty_code_thumbnails_render_without_language_badges() {
    let directory = tempfile::tempdir().expect("tempdir");
    let rust = directory.path().join("main.rs");
    let python = directory.path().join("main.py");
    let plain = directory.path().join("notes.txt");
    for path in [&rust, &python, &plain] {
        std::fs::write(path, b"").expect("write empty source");
    }

    let rust_png = text::render(&rust, 256).expect("render Rust language thumbnail");
    let python_png = text::render(&python, 256).expect("render Python language thumbnail");
    assert_eq!(
        (
            decode_png(&rust_png).width(),
            decode_png(&rust_png).height()
        ),
        (192, 256)
    );
    assert_eq!(rust_png, python_png);
    assert!(text::render(&plain, 256).is_err());
}

#[test]
fn code_thumbnails_classify_common_syntax_roles() {
    let source = "fn main() {\n    let count: i32 = 42;\n    let message = \"hello\"; // note\n}\n";
    let spans = text::syntax_spans(source, crate::sandbox::CodeLanguage::Rust);
    let role_for = |needle: &str| {
        let offset = source.find(needle).expect("syntax fixture token");
        spans
            .iter()
            .find(|span| span.start <= offset && span.end >= offset + needle.len())
            .map(|span| span.role)
    };

    assert_eq!(role_for("fn"), Some(text::SyntaxRole::Keyword));
    assert_eq!(role_for("i32"), Some(text::SyntaxRole::Type));
    assert_eq!(role_for("42"), Some(text::SyntaxRole::Constant));
    assert_eq!(role_for("\"hello\""), Some(text::SyntaxRole::String));
    assert_eq!(role_for("// note"), Some(text::SyntaxRole::Comment));
}

#[test]
fn browser_code_operation_highlights_without_a_filename_or_badge() {
    let directory = tempfile::tempdir().expect("tempdir");
    let source = directory.path().join("main.rs");
    std::fs::write(&source, b"fn main() { let answer: i32 = 42; }\n").expect("write source");
    let file = std::fs::File::open(&source).expect("open source");
    let input = Path::new("/proc/thread-self/fd").join(file.as_raw_fd().to_string());

    let response = super::browser_render(
        &input,
        crate::sandbox::browser::wire::Operation::Code(crate::sandbox::CodeLanguage::Rust),
    );
    let image = decode_png(&response.png);
    let pixels = image.read_pixel_bytes();
    let channels = image.n_channels() as usize;
    assert!(
        pixels
            .as_ref()
            .chunks_exact(channels)
            .any(|pixel| { pixel[0] > 240 && pixel[1] < 15 && pixel[2] < 15 && pixel[3] > 240 })
    );
    assert!(
        !pixels
            .as_ref()
            .chunks_exact(channels)
            .any(|pixel| { pixel[0] > 240 && pixel[1] < 15 && pixel[2] > 240 && pixel[3] > 240 })
    );
}

#[test]
fn text_snippets_render_bounded_document_pages() {
    let directory = tempfile::tempdir().expect("tempdir");
    let input = directory.path().join("notes.txt");
    std::fs::write(&input, b"The quick brown fox\njumps over the lazy dog\n").expect("write text");

    let png = text::render(&input, 256).expect("render text snippet");
    let image = decode_png(&png);
    assert_eq!((image.width(), image.height()), (192, 256));

    let binary = directory.path().join("binary.txt");
    std::fs::write(&binary, [0xde, 0xad, 0x00, 0xbe, 0xef]).expect("write binary");
    assert!(text::render(&binary, 256).is_err());

    let empty = directory.path().join("empty.txt");
    std::fs::write(&empty, b"").expect("write empty");
    assert!(text::render(&empty, 256).is_err());
}

#[test]
fn helper_dispatches_embedded_and_text_operations() {
    let directory = tempfile::tempdir().expect("tempdir");
    let output = directory.path().join("result.png");

    let document = directory.path().join("slides.pptx");
    write_zip(
        &document,
        &[("docProps/thumbnail.png", fixture_png(64, 64, 0x3366_99ff))],
    );
    run(&[
        "thumbnail-embedded".into(),
        document.to_string_lossy().into_owned(),
        output.to_string_lossy().into_owned(),
        "256".into(),
        "software".into(),
    ])
    .expect("thumbnail-embedded dispatch");
    let image = decode_png(&std::fs::read(&output).expect("read output"));
    assert_eq!((image.width(), image.height()), (64, 64));

    let notes = directory.path().join("main.rs");
    std::fs::write(&notes, b"fn main() {}\n").expect("write source");
    run(&[
        "thumbnail-text".into(),
        notes.to_string_lossy().into_owned(),
        output.to_string_lossy().into_owned(),
        "256".into(),
        "software".into(),
    ])
    .expect("thumbnail-text dispatch");
    let image = decode_png(&std::fs::read(&output).expect("read output"));
    assert_eq!((image.width(), image.height()), (192, 256));
}

#[test]
fn text_extracts_document_content_from_markup_files() {
    let directory = tempfile::tempdir().expect("tempdir");

    let rtf = directory.path().join("letter.rtf");
    std::fs::write(
        &rtf,
        "{\\rtf1\\ansi{\\fonttbl{\\f0 Arial;}}\\pard Hello {\\b World}\\par Second line}",
    )
    .expect("write rtf");
    assert!(text::render(&rtf, 256).is_ok());

    let headers_only = directory.path().join("headers.rtf");
    std::fs::write(
        &headers_only,
        "{\\rtf1\\ansi{\\fonttbl{\\f0 Arial;}}{\\colortbl;\\red0\\green0\\blue0;}}",
    )
    .expect("write header-only rtf");
    assert!(text::render(&headers_only, 256).is_err());

    let html = directory.path().join("page.html");
    std::fs::write(
        &html,
        "<!doctype html><html><body><p>Hello preview</p></body></html>",
    )
    .expect("write html");
    assert!(text::render(&html, 256).is_ok());
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
