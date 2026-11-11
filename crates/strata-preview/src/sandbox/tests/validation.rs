// SPDX-License-Identifier: MIT

use super::{MEDIA_PREVIEW, PDF_PREVIEW, assert_validity};
use crate::sandbox::{OutputError, ParseOperation, archive_output_error};

fn png(width: u32, height: u32) -> Vec<u8> {
    let mut data = b"\x89PNG\r\n\x1a\n".to_vec();
    data.extend_from_slice(&13u32.to_be_bytes());
    data.extend_from_slice(b"IHDR");
    data.extend_from_slice(&width.to_be_bytes());
    data.extend_from_slice(&height.to_be_bytes());
    data
}

#[test]
fn archive_listings_accept_only_valid_listing_payloads() {
    let operation = ParseOperation::ArchiveList {
        format: crate::model::ArchiveFormat::Tar,
        password: None,
    };
    let payload = serde_json::json!({
        "status": "open",
        "entries": [{"name": "a.txt", "directory": false, "size": 1}],
        "message": null,
    })
    .to_string()
    .into_bytes();
    assert_validity(
        &operation,
        &[
            (&payload, true),
            (b"not json", false),
            (b"{\"status\":\"open\"}", false),
            (b"{\"status\":\"bogus\",\"entries\":[]}", false),
            (
                b"{\"status\":\"too-large\",\"entries\":[],\"message\":null}",
                true,
            ),
            (
                b"{\"status\":\"error\",\"entries\":[],\"message\":\"broken\"}",
                true,
            ),
            (
                b"{\"status\":\"error\",\"entries\":[],\"message\":\"\"}",
                false,
            ),
        ],
    );
}

#[test]
fn archive_output_failures_map_to_archive_errors_not_image_errors() {
    assert_eq!(
        archive_output_error(&OutputError::Oversize),
        crate::sandbox::archive_listing::ARCHIVE_TOO_LARGE_MESSAGE
    );
    for error in [
        OutputError::Missing,
        OutputError::NonRegular,
        OutputError::Empty,
        OutputError::Io("boom".to_owned()),
    ] {
        assert_eq!(
            archive_output_error(&error),
            crate::sandbox::archive_listing::INVALID_ARCHIVE
        );
    }
    for (error, message) in [
        (
            OutputError::Missing,
            "The preview renderer produced no output",
        ),
        (
            OutputError::NonRegular,
            "The preview renderer produced a non-regular output",
        ),
        (
            OutputError::Empty,
            "The preview renderer produced an invalid output size",
        ),
        (
            OutputError::Oversize,
            "The preview renderer produced an invalid output size",
        ),
    ] {
        assert_eq!(error.message(), message, "{error:?}");
    }
}

#[test]
fn accepts_only_bounded_png_outputs_and_never_compressed_media() {
    assert_validity(
        &ParseOperation::AudioCover,
        &[
            (b"null", true),
            (b"", false),
            (b"null trailing", false),
            (&png(800, 800), true),
            (&png(801, 800), false),
        ],
    );
    assert_validity(
        &ParseOperation::ThumbnailImage,
        &[(&png(256, 256), true), (&png(257, 1), false)],
    );
    assert_validity(
        &ParseOperation::PreviewImage,
        &[
            (b"null", false),
            (&png(800, 800), true),
            (&png(801, 1), false),
            (b"\x89PNG\r\n\x1a\n", false),
        ],
    );
    assert_validity(
        &PDF_PREVIEW,
        &[
            (&png(640, 800), true),
            (&png(641, 799), false),
            (&png(640, 801), false),
            (&png(0, 100), false),
        ],
    );
    assert_validity(
        &MEDIA_PREVIEW,
        &[
            (b"\x1a\x45\xdf\xa3content", false),
            (b"\0\0\0\x18ftypisom", false),
            (b"", false),
            (b"unrelated data", false),
        ],
    );
}
