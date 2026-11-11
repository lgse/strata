// SPDX-License-Identifier: MIT

use super::{
    ARCHIVE_TOO_LARGE_MESSAGE, ArchiveListingStatus, MAX_ARCHIVE_ENTRIES, WireStatus,
    archive_payload_valid, decode_archive_listing, decode_archive_payload, encode_archive_result,
};

#[test]
fn archive_wire_contract_round_trips_all_statuses() {
    use crate::services::ArchiveFileEntry;

    let entries = vec![ArchiveFileEntry {
        name: "folder/item.txt".to_owned(),
        directory: false,
        size: 8,
    }];
    for status in [
        ArchiveListingStatus::Open,
        ArchiveListingStatus::NeedsPassword,
        ArchiveListingStatus::WrongPassword,
        ArchiveListingStatus::Unsupported,
    ] {
        let listing = super::ArchiveListing {
            status,
            entries: entries.clone(),
        };
        let decoded =
            decode_archive_listing(&encode_archive_result(&Ok(listing))).expect("round trip");
        assert_eq!(decoded.status, status);
        assert_eq!(decoded.entries, entries);
    }
}

#[test]
fn archive_payload_validation_accepts_every_known_status() {
    for payload in [
        encode_archive_result(&Err(ARCHIVE_TOO_LARGE_MESSAGE.to_owned())),
        encode_archive_result(&Err(super::INVALID_ARCHIVE.to_owned())),
    ] {
        assert!(archive_payload_valid(&payload));
        let decoded = decode_archive_payload(&payload).expect("valid payload");
        assert!(decoded.entries.is_empty());
    }
    let too_large = decode_archive_payload(&encode_archive_result(&Err(
        ARCHIVE_TOO_LARGE_MESSAGE.to_owned()
    )))
    .expect("valid payload");
    assert_eq!(too_large.status, WireStatus::TooLarge);
    assert!(too_large.message.is_none());
    assert!(!archive_payload_valid(b"not json"));
    assert!(!archive_payload_valid(
        b"{\"status\":\"bogus\",\"entries\":[]}"
    ));
}

#[test]
fn archive_wire_contract_carries_limit_errors_without_entries() {
    match decode_archive_listing(&encode_archive_result(&Err(
        ARCHIVE_TOO_LARGE_MESSAGE.to_owned()
    ))) {
        Err(message) => assert_eq!(message, ARCHIVE_TOO_LARGE_MESSAGE),
        Ok(_) => panic!("limit errors decode to errors"),
    }
}

#[test]
fn archive_wire_contract_rejects_malformed_payloads() {
    assert!(decode_archive_listing(b"not json").is_err());
    assert!(decode_archive_listing(b"{\"status\":\"open\"}").is_err());
    assert!(decode_archive_listing(b"{\"status\":\"nope\",\"entries\":[]}").is_err());
    assert!(
        decode_archive_listing(b"{\"status\":\"error\",\"entries\":[],\"message\":\"\"}").is_err()
    );
    assert!(
        decode_archive_listing(
            b"{\"status\":\"open\",\"entries\":[{\"name\":1,\"directory\":false,\"size\":1}]}"
        )
        .is_err()
    );
    assert!(
        decode_archive_listing(
            b"{\"status\":\"open\",\"entries\":[{\"name\":\"a\",\"directory\":false,\"size\":-1}]}"
        )
        .is_err()
    );
}

#[test]
fn archive_wire_contract_rejects_over_cap_payloads() {
    let entries = vec![
        serde_json::json!({"name": "f.txt", "directory": false, "size": 1 });
        MAX_ARCHIVE_ENTRIES + 1
    ];
    let payload = serde_json::json!({"status": "open", "entries": entries, "message": null})
        .to_string()
        .into_bytes();
    assert!(decode_archive_listing(&payload).is_err());
}

#[test]
fn archive_wire_contract_rejects_pathological_names_as_too_large() {
    let pathological = [
        "a/".repeat(10_000) + "file.txt",
        "ab/".repeat(5000) + "opaque.txt",
        "x".repeat(20_000) + "blob.txt",
    ];
    for name in pathological {
        let listing = super::ArchiveListing {
            status: ArchiveListingStatus::Open,
            entries: vec![crate::services::ArchiveFileEntry {
                name,
                directory: false,
                size: 1,
            }],
        };
        let payload = encode_archive_result(&Ok(listing));
        assert!(super::archive_payload_valid(&payload));
        match decode_archive_listing(&payload) {
            Err(message) => assert_eq!(message, ARCHIVE_TOO_LARGE_MESSAGE),
            Ok(listing) => panic!("pathological name must not list: {:?}", listing.entries),
        }
    }
}

#[test]
fn archive_wire_contract_rejects_excessive_cumulative_names_as_too_large() {
    let prefix = "a/".repeat(400);
    let entries = (0..MAX_ARCHIVE_ENTRIES)
        .map(|index| crate::services::ArchiveFileEntry {
            name: format!("{prefix}f{index:05}.txt"),
            directory: false,
            size: 1,
        })
        .collect();
    let listing = super::ArchiveListing {
        status: ArchiveListingStatus::Open,
        entries,
    };
    let payload = encode_archive_result(&Ok(listing));
    assert!(super::archive_payload_valid(&payload));
    match decode_archive_listing(&payload) {
        Err(message) => assert_eq!(message, ARCHIVE_TOO_LARGE_MESSAGE),
        Ok(listing) => panic!(
            "over-budget names must not list: {} entries decoded",
            listing.entries.len()
        ),
    }
}
