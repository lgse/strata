// SPDX-License-Identifier: MIT

use super::*;
use std::io::Cursor;

const METADATA: WireMetadata = WireMetadata {
    mode: Some(0o100_644),
    modified: Some(WireTime::FileTime(130_830_000_680_000_000)),
};

fn raw_record(kind: u32, text: &[u8], size: u64, mode: u32, time: (u32, u64)) -> Vec<u8> {
    let mut buffer = Vec::new();
    buffer.extend_from_slice(&kind.to_le_bytes());
    buffer.extend_from_slice(&(text.len() as u32).to_le_bytes());
    buffer.extend_from_slice(&size.to_le_bytes());
    buffer.extend_from_slice(&mode.to_le_bytes());
    buffer.extend_from_slice(&time.0.to_le_bytes());
    buffer.extend_from_slice(&time.1.to_le_bytes());
    buffer.extend_from_slice(text);
    buffer
}

#[test]
fn directory_and_end_records_round_trip() {
    let mut buffer = Vec::new();
    write_directory(&mut buffer, "photos", METADATA).expect("wire protocol round trip");
    write_directory(&mut buffer, "plain", WireMetadata::default())
        .expect("wire protocol round trip");
    let dos = WireMetadata {
        mode: None,
        modified: Some(WireTime::DosLocal(0x4707_8AA4)),
    };
    write_directory(&mut buffer, "dos", dos).expect("wire protocol round trip");
    write_end(&mut buffer).expect("wire protocol round trip");
    let mut reader = Cursor::new(buffer);
    assert_eq!(
        read_record(&mut reader).expect("wire protocol round trip"),
        Record::Directory("photos".to_owned(), METADATA)
    );
    assert_eq!(
        read_record(&mut reader).expect("wire protocol round trip"),
        Record::Directory("plain".to_owned(), WireMetadata::default())
    );
    assert_eq!(
        read_record(&mut reader).expect("wire protocol round trip"),
        Record::Directory("dos".to_owned(), dos)
    );
    assert_eq!(
        read_record(&mut reader).expect("wire protocol round trip"),
        Record::End
    );
}

#[test]
fn file_record_and_ok_trailer_round_trip() {
    let mut buffer = Vec::new();
    write_file_header(&mut buffer, "report.txt", 4, METADATA).expect("wire protocol round trip");
    write_chunk(&mut buffer, b"data").expect("wire protocol round trip");
    write_file_ok(&mut buffer).expect("wire protocol round trip");
    let mut reader = Cursor::new(buffer);
    assert_eq!(
        read_record(&mut reader).expect("wire protocol round trip"),
        Record::File("report.txt".to_owned(), 4, METADATA)
    );
    let mut body = FileBody::new(&mut reader, 4);
    let mut content = Vec::new();
    body.read_to_end(&mut content)
        .expect("wire protocol round trip");
    assert_eq!(content, b"data");
}

#[test]
fn file_trailer_failure_carries_the_message() {
    let mut buffer = Vec::new();
    write_file_failed(&mut buffer, &Failure::from("CRC mismatch"))
        .expect("wire protocol round trip");
    let mut reader = Cursor::new(buffer);
    assert!(FileBody::new(&mut reader, 4).read(&mut [0u8; 4]).is_err());
}

#[test]
fn failed_member_with_no_body_reports_error_before_any_file_bytes() {
    for kind in [
        FailureKind::Other,
        FailureKind::PasswordRequired,
        FailureKind::IncorrectPassword,
    ] {
        let failure = Failure::new(kind, "member failed");
        let mut buffer = Vec::new();
        write_file_failed(&mut buffer, &failure).expect("fixture stream");
        let mut reader = Cursor::new(buffer);
        let mut body = FileBody::new(&mut reader, 18);
        let mut contents = Vec::new();
        let error = body
            .read_to_end(&mut contents)
            .expect_err("member must fail");
        assert_eq!(error.to_string(), "member failed");
        assert_eq!(Failure::from_io(error), failure);
        assert!(contents.is_empty());
    }
}

#[test]
fn unknown_trailer_statuses_are_rejected() {
    for (status, message) in [(0u32, b"x".as_slice()), (4, b"".as_slice())] {
        let mut buffer = Vec::new();
        buffer.extend_from_slice(&status.to_le_bytes());
        buffer.extend_from_slice(&(message.len() as u32).to_le_bytes());
        buffer.extend_from_slice(message);
        assert!(
            read_file_trailer(&mut Cursor::new(buffer)).is_err(),
            "{status}"
        );
    }
}

#[test]
fn truncated_member_cannot_report_success() {
    let mut buffer = Vec::new();
    write_chunk(&mut buffer, b"abc").expect("fixture stream");
    write_file_ok(&mut buffer).expect("fixture stream");
    let mut reader = Cursor::new(buffer);
    let mut body = FileBody::new(&mut reader, 5);
    let mut contents = Vec::new();
    assert!(body.read_to_end(&mut contents).is_err());
    assert_eq!(contents, b"abc");
}

#[test]
fn error_record_round_trips_the_message_and_kind() {
    for kind in [
        FailureKind::Other,
        FailureKind::PasswordRequired,
        FailureKind::IncorrectPassword,
    ] {
        let failure = Failure::new(kind, "Unable to open RAR archive");
        let mut buffer = Vec::new();
        write_error(&mut buffer, &failure).expect("wire protocol round trip");
        let mut reader = Cursor::new(buffer);
        assert_eq!(
            read_record(&mut reader).expect("wire protocol round trip"),
            Record::Error(failure)
        );
    }
}

#[test]
fn magic_rejects_a_mismatched_stream() {
    for stale in [b"NOTRARXX", b"STRRAR01", b"STRRAR02"] {
        assert!(read_magic(&mut Cursor::new(stale.to_vec())).is_err());
    }
    let mut buffer = Vec::new();
    write_magic(&mut buffer).expect("wire protocol round trip");
    assert!(read_magic(&mut Cursor::new(buffer)).is_ok());
}

#[test]
fn oversized_text_length_is_rejected_before_allocating() {
    let mut buffer = raw_record(0, b"", 0, NO_MODE, (0, 0));
    buffer[4..8].copy_from_slice(&(16 * 1024 * 1024u32).to_le_bytes());
    let mut reader = Cursor::new(buffer);
    assert!(read_record(&mut reader).is_err());
}

#[test]
fn records_reject_malformed_names_sizes_metadata_or_times() {
    assert!(read_record(&mut Cursor::new(raw_record(2, b"", 0, NO_MODE, (0, 0)))).is_ok());
    for record in [
        raw_record(2, b"x", 0, NO_MODE, (0, 0)),
        raw_record(2, b"", 1, NO_MODE, (0, 0)),
        raw_record(2, b"", 0, 0o644, (0, 0)),
        raw_record(3, b"failed", 0, NO_MODE, (1, 1)),
        raw_record(4, b"failed", 1, NO_MODE, (0, 0)),
        raw_record(5, b"failed", 0, 0o644, (0, 0)),
        raw_record(0, b"dir", 0, NO_MODE, (0, 1)),
        raw_record(0, b"dir", 0, NO_MODE, (2, u64::from(u32::MAX) + 1)),
        raw_record(0, b"dir", 0, NO_MODE, (3, 0)),
    ] {
        assert!(read_record(&mut Cursor::new(record)).is_err());
    }
}

#[test]
fn unknown_record_kind_is_rejected() {
    for kind in [6, 9] {
        let buffer = raw_record(kind, b"", 0, NO_MODE, (0, 0));
        assert!(read_record(&mut Cursor::new(buffer)).is_err(), "{kind}");
    }
}

#[test]
fn non_utf8_text_is_rejected() {
    let buffer = raw_record(0, &[0xff], 0, NO_MODE, (0, 0));
    assert!(read_record(&mut Cursor::new(buffer)).is_err());
}
