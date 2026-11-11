// SPDX-License-Identifier: MIT

//! Damaged archives: truncated headers, lying sizes, bad checksums and gzip trailers.

use super::*;

fn write_two_member_tar_gz(archive: &Path, gzip: bool) -> Result<(), Box<dyn Error>> {
    write_tar_entries(
        archive,
        &[
            (tar::EntryType::Regular, "a.txt", b"a"),
            (tar::EntryType::Regular, "b.txt", b"b"),
        ],
        gzip,
    )
}

struct FailsOnceAt {
    data: Vec<u8>,
    position: usize,
    fail_at: usize,
    failed: bool,
}

impl Read for FailsOnceAt {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let available = io::BufRead::fill_buf(self)?;
        let count = available.len().min(buffer.len());
        buffer[..count].copy_from_slice(&available[..count]);
        io::BufRead::consume(self, count);
        Ok(count)
    }
}

impl io::BufRead for FailsOnceAt {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        if self.position == self.fail_at && !self.failed {
            self.failed = true;
            return Err(io::ErrorKind::Interrupted.into());
        }
        let end = if self.position < self.fail_at {
            self.fail_at
        } else {
            self.data.len()
        };
        Ok(&self.data[self.position..end])
    }

    fn consume(&mut self, amount: usize) {
        self.position += amount;
    }
}

#[test]
fn malformed_archives_remain_failures_not_cancellations() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let archive = root.path().join("archive");
    fs::write(&archive, b"not an archive")?;
    for format in ALL_FORMATS {
        let destination = tempfile::tempdir()?;
        let decoder = Decoder::new(format);
        assert!(
            matches!(
                decoder.run(&archive, destination.path()),
                Err(ArchiveError::Failed(_))
            ),
            "{format:?}"
        );
        assert_eq!(decoder.progress(), 0);
        assert_empty(destination.path(), "")?;
    }
    Ok(())
}

#[test]
fn zip_member_lying_about_its_size_is_refused_before_it_expands() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let archive = root.path().join("bomb.zip");
    write_zip(&archive, &[("zeros.bin", &vec![0u8; 8 << 20])])?;
    patch_zip_uncompressed_size(&archive, 16)?;
    assert!(
        fs::metadata(&archive)?.len() < 64 << 10,
        "fixture should be a small archive that expands to 8 MiB"
    );
    let destination = tempfile::tempdir()?;

    let error = extract_zip(&archive, destination.path())
        .expect_err("a member exceeding its declared size should fail");

    assert!(
        error.contains("“zeros.bin” declared 16 bytes but produced more"),
        "{error}"
    );
    assert_empty(destination.path(), "the partial member should be removed")?;
    Ok(())
}

#[test]
fn zip_member_declaring_more_than_it_contains_is_refused() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let archive = root.path().join("short.zip");
    write_zip(&archive, &[("note.txt", b"hello")])?;
    patch_zip_uncompressed_size(&archive, 1000)?;
    let destination = tempfile::tempdir()?;

    let error = extract_zip(&archive, destination.path())
        .expect_err("a member shorter than its declared size should fail");

    assert!(
        error.contains("“note.txt” declared 1,000 bytes but produced 5 bytes"),
        "{error}"
    );
    assert_empty(destination.path(), "the truncated member should be removed")?;
    Ok(())
}

#[test]
fn truncated_headers_have_clear_errors() -> Result<(), Box<dyn Error>> {
    for format in ALL_FORMATS {
        let root = tempfile::tempdir()?;
        let source = root.path().join("file.txt");
        fs::write(&source, b"harmless contents")?;
        let archive = root.path().join("archive");
        write_compression_fixture(&archive, &[source], format, None)?;
        fs::OpenOptions::new()
            .write(true)
            .open(&archive)?
            .set_len(12)?;
        let destination = tempfile::tempdir()?;

        let result = Decoder::new(format).run(&archive, destination.path());

        let Err(error) = result else {
            panic!("accepted truncated {format:?}")
        };
        assert_eq!(error.to_string(), INVALID_ARCHIVE, "{format:?}");
        assert_empty(destination.path(), "")?;
    }
    Ok(())
}

#[test]
fn gzip_trailer_mismatch_fails_after_the_last_member() -> Result<(), Box<dyn Error>> {
    let large = vec![b'x'; 50_000];
    for (offset_from_end, label) in [(8, "crc32"), (4, "isize")] {
        let root = tempfile::tempdir()?;
        let archive = root.path().join("content.tar.gz");
        write_tar_entries(
            &archive,
            &[
                (tar::EntryType::Regular, "a.txt", &large),
                (tar::EntryType::Regular, "b.txt", b"b"),
            ],
            true,
        )?;
        corrupt_gzip_trailer(&archive, offset_from_end)?;
        let destination = tempfile::tempdir()?;
        fs::write(destination.path().join("b.txt"), b"existing")?;
        let decoder = Decoder::new(ArchiveFormat::TarGz);

        let result = decoder.run(&archive, destination.path());

        assert_failed_keeping(&result, "content", label);
        assert_eq!(decoder.progress(), 2, "{label}");
        assert_eq!(
            fs::metadata(destination.path().join("content/a.txt"))?.len(),
            50_000,
            "{label}"
        );
        assert_contents(
            destination.path(),
            &[("content/b.txt", b"b"), ("b.txt", b"existing")],
            label,
        )?;
        assert_entry_count(destination.path(), 2, label)?;
    }
    Ok(())
}

#[test]
fn truncated_gzip_trailer_is_damaged() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let archive = root.path().join("content.tar.gz");
    write_tar_entries(&archive, &[(tar::EntryType::Regular, "a.txt", b"a")], true)?;
    let length = fs::metadata(&archive)?.len();
    fs::OpenOptions::new()
        .write(true)
        .open(&archive)?
        .set_len(length - 3)?;
    let members = tar::Archive::new(flate2::read::GzDecoder::new(fs::File::open(&archive)?))
        .entries()?
        .count();
    assert_eq!(members, 1, "the TAR stream must stay readable");
    let destination = tempfile::tempdir()?;

    let result = Decoder::new(ArchiveFormat::TarGz).run(&archive, destination.path());

    assert_failed_keeping(&result, "content", "");
    assert_contents(destination.path(), &[("content/a.txt", b"a")], "")?;
    assert_entry_count(destination.path(), 1, "")?;
    Ok(())
}

#[test]
fn gzip_trailer_check_is_cancellable() -> Result<(), Box<dyn Error>> {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(&[b'x'; 4096])?;
    let valid = encoder.finish()?;
    let mut corrupt = valid.clone();
    let crc = corrupt.len() - 8;
    corrupt[crc] ^= 0xff;
    for (label, stream, cancelled, expected) in [
        ("valid", &valid, never_cancelled(), Ok(())),
        (
            "cancelled",
            &valid,
            always_cancelled(),
            Err(ArchiveError::Cancelled),
        ),
        (
            "corrupt",
            &corrupt,
            never_cancelled(),
            Err(ArchiveError::Failed(INVALID_ARCHIVE.to_owned())),
        ),
    ] {
        let members = super::super::GzipMembers::new(Cursor::new(stream));
        assert_eq!(
            super::super::verify_gzip_trailer(members, &cancelled),
            expected,
            "{label}"
        );
    }
    Ok(())
}

#[test]
fn corrupt_members_are_removed_without_losing_completed_or_existing_files()
-> Result<(), Box<dyn Error>> {
    for format in [ArchiveFormat::Zip, ArchiveFormat::SevenZ] {
        let root = tempfile::tempdir()?;
        let archive = root.path().join("archive.zip");
        let entries = [
            ("done.txt", b"done".as_slice()),
            ("broken.txt", b"payload".as_slice()),
        ];
        if format == ArchiveFormat::Zip {
            write_zip_stored(&archive, &entries)?;
        } else {
            write_7z_stored(&archive, &entries)?;
        }
        let bytes = corrupt_stored_payload(&archive, b"payload", 1)?;
        let destination = tempfile::tempdir()?;
        fs::write(destination.path().join("broken.txt"), b"original")?;
        let decoder = Decoder::new(format);
        let context = format!("{format:?}");

        let result = decoder.run(&archive, destination.path());

        assert_failed_keeping(&result, "archive", &context);
        assert_eq!(decoder.progress(), 1);
        assert_contents(
            destination.path(),
            &[("archive/done.txt", b"done"), ("broken.txt", b"original")],
            &context,
        )?;
        assert!(!destination.path().join("archive/broken.txt").exists());
        assert_entry_count(destination.path(), 2, &context)?;
        assert_eq!(fs::read(&archive)?, bytes);
    }
    Ok(())
}

#[test]
fn unencrypted_zip_checksum_failure_stays_damaged() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let archive = root.path().join("archive.zip");
    write_zip_stored(&archive, &[("file.txt", b"checksum-payload")])?;
    corrupt_stored_payload(&archive, b"checksum-payload", 1)?;
    let destination = tempfile::tempdir()?;

    let result = Decoder::new(ArchiveFormat::Zip).run(&archive, destination.path());

    let Err(error) = result else {
        panic!("accepted a checksum-damaged zip");
    };
    assert_eq!(error.to_string(), INVALID_ARCHIVE);
    assert_empty(destination.path(), "")?;
    Ok(())
}

#[test]
fn every_gzip_member_of_a_tar_gz_is_read_and_verified() -> Result<(), Box<dyn Error>> {
    for corrupt_second_member in [false, true] {
        let root = tempfile::tempdir()?;
        let archive = root.path().join("content.tar.gz");
        write_two_member_tar_gz(&archive, false)?;
        // After `a.txt`'s header and data blocks, so a reader that stops at
        // the first member sees a complete archive.
        split_into_gzip_members(&archive, 1024)?;
        if corrupt_second_member {
            corrupt_gzip_trailer(&archive, 8)?;
        }
        let destination = root.path().join("destination");
        fs::create_dir(&destination)?;

        let result = Decoder::new(ArchiveFormat::TarGz).run(&archive, &destination);

        if corrupt_second_member {
            assert_failed_keeping(&result, "content", "");
        } else {
            assert_eq!(completed_extract(result?)?.as_deref(), Some("content"));
        }
        assert_contents(
            &destination,
            &[("content/a.txt", b"a"), ("content/b.txt", b"b")],
            "",
        )?;
    }
    Ok(())
}

#[test]
fn zero_padding_after_the_last_gzip_member_is_not_damage() -> Result<(), Box<dyn Error>> {
    for (padding, padded) in [
        (vec![0, 0, b'x'], false),
        (vec![0; 10_240], true),
        (vec![0; 1], true),
    ] {
        let root = tempfile::tempdir()?;
        let archive = root.path().join("content.tar.gz");
        write_two_member_tar_gz(&archive, true)?;
        let mut bytes = fs::read(&archive)?;
        bytes.extend_from_slice(&padding);
        fs::write(&archive, bytes)?;
        let destination = root.path().join("destination");
        fs::create_dir(&destination)?;
        let label = format!(
            "{} trailing bytes ending in {:?}",
            padding.len(),
            padding.last()
        );

        let result = Decoder::new(ArchiveFormat::TarGz).run(&archive, &destination);

        if padded {
            assert!(
                matches!(&result, Ok(ArchiveOutcome::Completed(Some(name))) if name == "content"),
                "{label}: {result:?}"
            );
        } else {
            assert_failed_keeping(&result, "content", &label);
        }
        assert_contents(
            &destination,
            &[("content/a.txt", b"a"), ("content/b.txt", b"b")],
            &label,
        )?;
    }
    Ok(())
}

#[test]
fn a_failed_read_between_gzip_members_does_not_end_the_stream() -> Result<(), Box<dyn Error>> {
    let gzip = |contents: &[u8]| -> io::Result<Vec<u8>> {
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(contents)?;
        encoder.finish()
    };
    let first = gzip(b"first member, ")?;
    let mut data = first.clone();
    data.extend(gzip(b"second member")?);
    let mut members = super::super::GzipMembers::new(FailsOnceAt {
        data,
        position: 0,
        fail_at: first.len(),
        failed: false,
    });

    let mut decoded = Vec::new();
    let mut interruptions = 0;
    let mut buffer = [0; 64];
    loop {
        match members.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => decoded.extend_from_slice(&buffer[..count]),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => interruptions += 1,
            Err(error) => return Err(error.into()),
        }
    }

    assert_eq!(interruptions, 1);
    assert_eq!(decoded, b"first member, second member");
    assert_eq!(members.verify_padding(&never_cancelled()), Ok(()));
    Ok(())
}
