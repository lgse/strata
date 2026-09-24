// SPDX-License-Identifier: MIT

use super::*;
use std::io::Cursor;

#[test]
fn directory_and_end_records_round_trip() {
    let mut buffer = Vec::new();
    write_directory(&mut buffer, "photos").expect("wire protocol round trip");
    write_end(&mut buffer).expect("wire protocol round trip");
    let mut reader = Cursor::new(buffer);
    assert_eq!(
        read_record(&mut reader).expect("wire protocol round trip"),
        Record::Directory("photos".to_owned())
    );
    assert_eq!(
        read_record(&mut reader).expect("wire protocol round trip"),
        Record::End
    );
}

#[test]
fn file_record_and_ok_trailer_round_trip() {
    let mut buffer = Vec::new();
    write_file_header(&mut buffer, "report.txt", 4).expect("wire protocol round trip");
    buffer.extend_from_slice(b"data");
    write_file_ok(&mut buffer).expect("wire protocol round trip");
    let mut reader = Cursor::new(buffer);
    assert_eq!(
        read_record(&mut reader).expect("wire protocol round trip"),
        Record::File("report.txt".to_owned(), 4)
    );
    let mut body = [0u8; 4];
    reader
        .read_exact(&mut body)
        .expect("wire protocol round trip");
    assert_eq!(&body, b"data");
    assert_eq!(
        read_file_trailer(&mut reader).expect("wire protocol round trip"),
        Ok(())
    );
}

#[test]
fn file_trailer_failure_carries_the_message() {
    let mut buffer = Vec::new();
    write_file_failed(&mut buffer, "CRC mismatch").expect("wire protocol round trip");
    let mut reader = Cursor::new(buffer);
    assert_eq!(
        read_file_trailer(&mut reader).expect("wire protocol round trip"),
        Err("CRC mismatch".to_owned())
    );
}

#[test]
fn error_record_round_trips_the_message() {
    let mut buffer = Vec::new();
    write_error(&mut buffer, "Unable to open RAR archive").expect("wire protocol round trip");
    let mut reader = Cursor::new(buffer);
    assert_eq!(
        read_record(&mut reader).expect("wire protocol round trip"),
        Record::Error("Unable to open RAR archive".to_owned())
    );
}

#[test]
fn magic_rejects_a_mismatched_stream() {
    let mut reader = Cursor::new(b"NOTRARXX".to_vec());
    assert!(read_magic(&mut reader).is_err());
    let mut buffer = Vec::new();
    write_magic(&mut buffer).expect("wire protocol round trip");
    assert!(read_magic(&mut Cursor::new(buffer)).is_ok());
}

#[test]
fn oversized_text_length_is_rejected_before_allocating() {
    let mut buffer = Vec::new();
    buffer.extend_from_slice(&0u32.to_le_bytes()); // kind: directory
    buffer.extend_from_slice(&(16 * 1024 * 1024u32).to_le_bytes()); // claimed name length
    buffer.extend_from_slice(&0u64.to_le_bytes());
    let mut reader = Cursor::new(buffer);
    assert!(read_record(&mut reader).is_err());
}

#[test]
fn end_record_rejects_a_nonempty_name_or_size() {
    let mut buffer = Vec::new();
    buffer.extend_from_slice(&2u32.to_le_bytes()); // kind: end
    buffer.extend_from_slice(&1u32.to_le_bytes());
    buffer.extend_from_slice(&0u64.to_le_bytes());
    buffer.push(b'x');
    assert!(read_record(&mut Cursor::new(buffer)).is_err());
}

#[test]
fn unknown_record_kind_is_rejected() {
    let mut buffer = Vec::new();
    buffer.extend_from_slice(&9u32.to_le_bytes());
    buffer.extend_from_slice(&0u32.to_le_bytes());
    buffer.extend_from_slice(&0u64.to_le_bytes());
    assert!(read_record(&mut Cursor::new(buffer)).is_err());
}

#[test]
fn non_utf8_text_is_rejected() {
    let mut buffer = Vec::new();
    buffer.extend_from_slice(&0u32.to_le_bytes());
    buffer.extend_from_slice(&1u32.to_le_bytes());
    buffer.extend_from_slice(&0u64.to_le_bytes());
    buffer.push(0xff);
    assert!(read_record(&mut Cursor::new(buffer)).is_err());
}
