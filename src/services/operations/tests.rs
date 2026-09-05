// SPDX-License-Identifier: GPL-3.0-or-later

use super::{ArchiveFormat, validate_basename};

#[test]
fn basenames_reject_empty_reserved_nested_absolute_and_nul_names() {
    for name in [
        "",
        ".",
        "..",
        "../escaped",
        "nested/child",
        "/tmp/absolute",
        "nul\0name",
    ] {
        assert!(
            validate_basename(name).is_err(),
            "{name:?} should be rejected"
        );
    }
}

#[test]
fn basenames_accept_single_native_and_unicode_components() {
    for name in ["report.txt", "folder name", ".config", "résumé"] {
        assert!(
            validate_basename(name).is_ok(),
            "{name:?} should be accepted"
        );
    }
}

#[test]
fn archive_formats_are_detected_by_extension() {
    assert_eq!(
        ArchiveFormat::from_extension("photos.zip"),
        Some(ArchiveFormat::Zip)
    );
    assert_eq!(
        ArchiveFormat::from_extension("backup.tar.gz"),
        Some(ArchiveFormat::TarGz)
    );
    assert_eq!(
        ArchiveFormat::from_extension("archive.TGZ"),
        Some(ArchiveFormat::TarGz)
    );
    assert_eq!(
        ArchiveFormat::from_extension("data.tar"),
        Some(ArchiveFormat::Tar)
    );
    assert_eq!(ArchiveFormat::from_extension("document.pdf"), None);
    assert_eq!(ArchiveFormat::from_extension("no_extension"), None);
}

#[test]
fn archive_format_extensions_round_trip() {
    for format in [ArchiveFormat::Zip, ArchiveFormat::TarGz, ArchiveFormat::Tar] {
        let name = format!("test.{}", format.extension());
        assert_eq!(ArchiveFormat::from_extension(&name), Some(format));
    }
}

#[test]
fn conversion_formats_expose_extensions_labels_and_types() {
    use super::{ConversionFormat, ConversionScale};

    assert_eq!(ConversionFormat::Webp.extension(), "webp");
    assert_eq!(ConversionFormat::Webp.label(), "WebP");
    assert!(ConversionFormat::Webp.is_image());
    assert!(!ConversionFormat::Webp.is_video());
    assert!(!ConversionFormat::Webp.is_audio());

    assert_eq!(ConversionFormat::Avif.extension(), "avif");
    assert_eq!(ConversionFormat::Avif.label(), "AVIF");
    assert!(ConversionFormat::Avif.is_image());

    assert_eq!(ConversionFormat::Png.extension(), "png");
    assert_eq!(ConversionFormat::Jpeg.extension(), "jpg");
    assert_eq!(ConversionFormat::Pdf.extension(), "pdf");
    assert!(!ConversionFormat::Pdf.is_image());

    assert_eq!(ConversionFormat::Ico.extension(), "ico");
    assert_eq!(ConversionFormat::Tiff.extension(), "tiff");
    assert_eq!(ConversionFormat::Bmp.extension(), "bmp");
    assert!(ConversionFormat::Ico.is_image());

    assert_eq!(ConversionFormat::Mp4.extension(), "mp4");
    assert_eq!(ConversionFormat::Mp4.label(), "MP4");
    assert!(ConversionFormat::Mp4.is_video());
    assert!(!ConversionFormat::Mp4.is_image());

    assert_eq!(ConversionFormat::Webm.extension(), "webm");
    assert_eq!(ConversionFormat::Mkv.extension(), "mkv");
    assert_eq!(ConversionFormat::Mov.extension(), "mov");
    assert_eq!(ConversionFormat::Gif.extension(), "gif");
    assert!(ConversionFormat::Gif.is_video());

    assert_eq!(ConversionFormat::Mp3.extension(), "mp3");
    assert_eq!(ConversionFormat::Mp3.label(), "MP3");
    assert!(ConversionFormat::Mp3.is_audio());
    assert!(!ConversionFormat::Mp3.is_video());

    assert_eq!(ConversionFormat::Opus.extension(), "opus");
    assert_eq!(ConversionFormat::Aac.extension(), "m4a");
    assert_eq!(ConversionFormat::Flac.extension(), "flac");
    assert_eq!(ConversionFormat::Wav.extension(), "wav");
    assert_eq!(ConversionFormat::Ogg.extension(), "ogg");
    assert!(ConversionFormat::Aac.is_audio());
    assert!(ConversionFormat::Flac.is_audio());

    assert_eq!(ConversionScale::Original.label(), "100%");
    assert_eq!(ConversionScale::Percent75.label(), "75%");
    assert_eq!(ConversionScale::Percent50.label(), "50%");
    assert_eq!(ConversionScale::Percent25.label(), "25%");
}
