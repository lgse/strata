// SPDX-License-Identifier: MIT

use super::*;
use std::{error::Error, sync::atomic::Ordering};

#[test]
fn excessive_native_dictionary_is_rejected() {
    let cancelled = AtomicBool::new(false);
    let result = call(None, &cancelled, None, |state| {
        assert_eq!(
            callback(UCM_LARGEDICT, state, 2 * 1024 * 1024, 1024 * 1024),
            -1
        );
        ERAR_LARGE_DICT
    });
    assert_eq!(
        result
            .expect_err("large dictionary callback must fail")
            .to_string(),
        LARGE_DICTIONARY
    );
    assert_eq!(
        decode_result(ERAR_LARGE_DICT, None)
            .expect_err("large dictionary code must fail")
            .to_string(),
        LARGE_DICTIONARY
    );
}

#[test]
fn callback_streams_bounded_chunks() -> Result<(), Box<dyn Error>> {
    let destination = tempfile::tempdir()?;
    let progress = Arc::new(AtomicUsize::new(0));
    let cancelled = AtomicBool::new(false);
    let mut session = ExtractionSession::open(destination.path(), &progress, &cancelled)?;
    let chunk = [42u8; 65536];
    let mut decode = |sink: &mut MemberSink<'_>| {
        call(None, &cancelled, Some(sink), |state| {
            for _ in 0..1024 {
                assert_eq!(
                    callback(
                        native::UCM_PROCESSDATA,
                        state,
                        chunk.as_ptr() as native::LPARAM,
                        chunk.len() as native::LPARAM
                    ),
                    1
                );
            }
            0
        })?;
        Ok(())
    };
    session.extract_member(
        "large.bin",
        MemberContent::Decoded(&mut decode, 64 * 1024 * 1024),
    )?;
    assert_eq!(
        std::fs::metadata(destination.path().join("large.bin"))?.len(),
        64 * 1024 * 1024
    );
    assert_eq!(progress.load(Ordering::Relaxed), 1);
    Ok(())
}

#[test]
fn callback_cancellation_removes_partial_member() -> Result<(), Box<dyn Error>> {
    let destination = tempfile::tempdir()?;
    let progress = Arc::new(AtomicUsize::new(0));
    let cancelled = AtomicBool::new(false);
    let mut session = ExtractionSession::open(destination.path(), &progress, &cancelled)?;
    let chunk = [42u8; 32];
    let mut decode = |sink: &mut MemberSink<'_>| {
        call(None, &cancelled, Some(sink), |state| {
            assert_eq!(
                callback(
                    native::UCM_PROCESSDATA,
                    state,
                    chunk.as_ptr() as native::LPARAM,
                    32
                ),
                1
            );
            cancelled.store(true, Ordering::Relaxed);
            assert_eq!(
                callback(
                    native::UCM_PROCESSDATA,
                    state,
                    chunk.as_ptr() as native::LPARAM,
                    32
                ),
                -1
            );
            native::ERAR_UNKNOWN
        })?;
        Ok(())
    };
    let result = session.extract_member("partial.bin", MemberContent::Decoded(&mut decode, 64));
    assert_eq!(result, Err(ArchiveError::Cancelled));
    let outcome = session.finish(result, Vec::new)?;
    assert!(
        matches!(outcome, ArchiveOutcome::Cancelled { completed, failed, not_attempted } if completed.is_empty() && failed.is_empty() && not_attempted.len() == 1)
    );
    assert!(!destination.path().join("partial.bin").exists());
    assert_eq!(progress.load(Ordering::Relaxed), 0);
    Ok(())
}

#[test]
fn decoded_members_validate_paths_and_space_before_callback() -> Result<(), Box<dyn Error>> {
    let destination = tempfile::tempdir()?;
    let progress = Arc::new(AtomicUsize::new(0));
    let cancelled = AtomicBool::new(false);
    for (name, size) in [
        ("../escape", 0),
        ("/absolute", 0),
        ("C:\\escape", 0),
        ("oversized", 2),
    ] {
        let mut session = ExtractionSession::open_with_available_bytes(
            destination.path(),
            &progress,
            &cancelled,
            Some(1),
        )?;
        let mut decode = |_: &mut MemberSink<'_>| -> Result<(), ArchiveError> {
            panic!("invalid member must not be decoded")
        };
        assert!(
            session
                .extract_member(name, MemberContent::Decoded(&mut decode, size))
                .is_err()
        );
    }
    assert_eq!(std::fs::read_dir(destination.path())?.count(), 0);
    Ok(())
}

#[test]
fn callback_rejects_excess_output_and_short_members() -> Result<(), Box<dyn Error>> {
    let destination = tempfile::tempdir()?;
    let progress = Arc::new(AtomicUsize::new(0));
    let cancelled = AtomicBool::new(false);
    for declared in [1, 3] {
        let mut session = ExtractionSession::open(destination.path(), &progress, &cancelled)?;
        let bytes = [1u8, 2];
        let mut decode = |sink: &mut MemberSink<'_>| {
            call(None, &cancelled, Some(sink), |state| {
                callback(
                    native::UCM_PROCESSDATA,
                    state,
                    bytes.as_ptr() as native::LPARAM,
                    2,
                )
            })?;
            Ok(())
        };
        assert!(
            session
                .extract_member("invalid.bin", MemberContent::Decoded(&mut decode, declared))
                .is_err()
        );
        assert!(!destination.path().join("invalid.bin").exists());
    }
    Ok(())
}

#[test]
fn rar_destination_symlink_cannot_escape() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let destination = root.path().join("destination");
    std::fs::create_dir(&destination)?;
    let outside = root.path().join("outside");
    std::fs::write(&outside, b"untouched")?;
    std::os::unix::fs::symlink(&outside, destination.join("VERSION"))?;
    let archive = root.path().join("test.rar");
    std::fs::write(&archive, super::super::super::fixtures::RAR_VERSION_FIXTURE)?;
    assert!(
        extract_rar(
            &archive,
            &destination,
            None,
            &Arc::new(AtomicUsize::new(0)),
            &AtomicBool::new(false)
        )
        .is_err()
    );
    assert_eq!(std::fs::read(outside)?, b"untouched");
    Ok(())
}
