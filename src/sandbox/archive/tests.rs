// SPDX-License-Identifier: MIT

use super::*;
use std::io::Cursor;

fn fixture_stream(build: impl FnOnce(&mut Vec<u8>)) -> Cursor<Vec<u8>> {
    let mut bytes = Vec::new();
    wire::write_magic(&mut bytes).expect("fixture stream");
    build(&mut bytes);
    Cursor::new(bytes)
}

#[test]
fn drive_dispatches_directory_and_file_members_in_archive_order() {
    let reader = fixture_stream(|bytes| {
        wire::write_directory(bytes, "photos").expect("fixture stream");
        wire::write_file_header(bytes, "notes.txt", 5).expect("fixture stream");
        bytes.extend_from_slice(b"hello");
        wire::write_file_ok(bytes).expect("fixture stream");
        wire::write_end(bytes).expect("fixture stream");
    });

    let mut seen = Vec::new();
    drive(reader, |name, member| {
        match member {
            Member::Directory => seen.push((name.to_owned(), None)),
            Member::File { size, body } => {
                let mut content = Vec::new();
                body.read_to_end(&mut content).expect("fixture stream");
                seen.push((name.to_owned(), Some((size, content))));
            }
        }
        Ok(())
    })
    .expect("a well-formed stream must succeed");

    assert_eq!(
        seen,
        vec![
            ("photos".to_owned(), None),
            ("notes.txt".to_owned(), Some((5, b"hello".to_vec()))),
        ]
    );
}

#[test]
fn drive_propagates_a_top_level_error_record_and_stops() {
    let reader = fixture_stream(|bytes| {
        wire::write_error(bytes, "Unable to open RAR archive").expect("fixture stream");
    });
    let mut calls = 0;
    let error = drive(reader, |_, _| {
        calls += 1;
        Ok(())
    })
    .expect_err("an error record must fail the stream");
    assert_eq!(error, "Unable to open RAR archive");
    assert_eq!(calls, 0, "no member callback runs after a top-level error");
}

#[test]
fn drive_propagates_a_file_trailer_failure_and_stops_before_end() {
    let reader = fixture_stream(|bytes| {
        wire::write_file_header(bytes, "broken.bin", 3).expect("fixture stream");
        bytes.extend_from_slice(b"abc");
        wire::write_file_failed(bytes, "CRC mismatch").expect("fixture stream");
        // A well-behaved child never writes more after a failed member, but
        // even if it did, drive() must stop at the trailer, not read this.
        wire::write_end(bytes).expect("fixture stream");
    });
    let mut calls = 0;
    let error = drive(reader, |_, _| {
        calls += 1;
        Ok(())
    })
    .expect_err("a failed trailer must fail the stream");
    assert_eq!(error, "CRC mismatch");
    assert_eq!(calls, 1, "the file member callback still runs once");
}

#[test]
fn drive_stops_immediately_when_on_member_fails() {
    let reader = fixture_stream(|bytes| {
        wire::write_directory(bytes, "first").expect("fixture stream");
        wire::write_directory(bytes, "second").expect("fixture stream");
        wire::write_end(bytes).expect("fixture stream");
    });
    let mut calls = 0;
    let error = drive(reader, |name, _| {
        calls += 1;
        Err(format!("rejected {name}"))
    })
    .expect_err("an on_member failure must stop the stream");
    assert_eq!(error, "rejected first");
    assert_eq!(calls, 1, "drive must not continue to the next record");
}

#[test]
fn drive_drains_a_body_the_callback_left_unread_before_the_trailer() {
    // on_member can stop reading partway through a member (its own error);
    // drive() must still consume the rest of the declared body so the
    // trailer that follows lines up on the wire.
    let reader = fixture_stream(|bytes| {
        wire::write_file_header(bytes, "big.bin", 5).expect("fixture stream");
        bytes.extend_from_slice(b"hello");
        wire::write_file_ok(bytes).expect("fixture stream");
        wire::write_end(bytes).expect("fixture stream");
    });
    let mut seen_end = false;
    drive(reader, |_, member| {
        if let Member::File { body, .. } = member {
            let mut first_byte = [0u8; 1];
            body.read_exact(&mut first_byte).expect("fixture stream");
            assert_eq!(&first_byte, b"h");
        }
        Ok(())
    })
    .map(|()| seen_end = true)
    .expect("draining the unread body must not desync the stream");
    assert!(seen_end);
}

#[test]
fn rar_extraction_command_binds_the_archive_read_only_with_no_output_directory() {
    let command = rar_extraction_command(
        Path::new("/usr/bin/bwrap"),
        Path::new("/tmp/strata"),
        Path::new("/home/alice/Downloads/untrusted.rar"),
        None,
    )
    .expect("command construction must succeed without a password");
    assert_eq!(command.get_program(), Path::new("/usr/bin/bwrap"));
    let joined = command
        .get_args()
        .map(|argument| argument.to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join(" ");
    assert!(joined.contains("--unshare-all"));
    assert!(joined.contains("--ro-bind /home/alice/Downloads/untrusted.rar /input.rar"));
    assert!(joined.contains("--ro-bind /tmp/strata /app/strata"));
    assert!(joined.contains("--fsize=0"));
    assert!(joined.contains(&format!("--as={ADDRESS_SPACE_LIMIT_BYTES}")));
    assert!(joined.contains(&format!("--cpu={RAR_CPU_TIME_LIMIT_SECS}")));
    assert!(joined.contains("--preview-helper extract-rar /input.rar"));
    // The defining security property: no writable bind at all, unlike every
    // other sandboxed operation which binds a private /output directory.
    assert!(!joined.contains("--bind"));
    assert!(!joined.contains("/output"));
    assert!(!joined.contains("--share-net"));
}

#[test]
fn rar_extraction_command_threads_the_password_without_leaking_it_on_argv() {
    let command = rar_extraction_command(
        Path::new("/usr/bin/bwrap"),
        Path::new("/tmp/strata"),
        Path::new("/home/alice/Downloads/secret.rar"),
        Some("s3cret"),
    )
    .expect("command construction must succeed with a password");
    let joined = command
        .get_args()
        .map(|argument| argument.to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join(" ");
    assert!(!joined.contains("s3cret"));
    // The trailing "0" tells the sandboxed helper to read the secret from
    // the duplicated stdin descriptor instead.
    assert!(joined.trim_end().ends_with(" 0"));
}
