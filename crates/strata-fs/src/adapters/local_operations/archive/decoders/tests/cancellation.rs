// SPDX-License-Identifier: MIT

//! What a cancelled decoder leaves behind and reports as pending.

use super::*;

struct CancelAfterMembers<'a> {
    file: fs::File,
    progress: &'a AtomicUsize,
    cancelled: &'a AtomicBool,
    after: usize,
}

impl Read for CancelAfterMembers<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.progress.load(Ordering::Relaxed) >= self.after {
            self.cancelled.store(true, Ordering::Relaxed);
        }
        self.file.read(buffer)
    }
}

impl Seek for CancelAfterMembers<'_> {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.file.seek(position)
    }
}

/// Members of the mixed fixture in archive order: a directory, a streamless file, two
/// duplicates and a later file with streams, then an empty directory and a zero-byte file.
const MIXED_MEMBERS: [&str; 7] = [
    "folder",
    "folder/same.txt",
    "folder/same.txt",
    "folder/same.txt",
    "folder/later.txt",
    "folder/empty",
    "folder/zero.txt",
];

const MIXED_COMPLETED: [(&str, &[u8]); 2] = [
    ("mixed/folder/same.txt", b"one"),
    ("mixed/folder/same (2).txt", b"two"),
];

fn write_mixed_7z(path: &Path, solid: bool) -> Result<(), Box<dyn Error>> {
    let mut writer = sevenz_rust2::ArchiveWriter::create(path)?;
    writer.set_content_methods(vec![sevenz_rust2::EncoderConfiguration::new(
        sevenz_rust2::EncoderMethod::COPY,
    )]);
    for entry in [
        sevenz_rust2::ArchiveEntry::new_directory("folder"),
        sevenz_rust2::ArchiveEntry::new_file("folder/same.txt"),
    ] {
        writer.push_archive_entry::<Cursor<&[u8]>>(entry, None)?;
    }
    push_7z_files(
        &mut writer,
        &[
            ("folder/same.txt", b"one"),
            ("folder/same.txt", b"two"),
            ("folder/later.txt", b"later"),
        ],
        solid,
    )?;
    for entry in [
        sevenz_rust2::ArchiveEntry::new_directory("folder/empty"),
        sevenz_rust2::ArchiveEntry::new_file("folder/zero.txt"),
    ] {
        writer.push_archive_entry::<Cursor<&[u8]>>(entry, None)?;
    }
    writer.finish()?;
    Ok(())
}

fn interrupted_after(after: usize) -> &'static str {
    if after == 1 {
        "mixed/folder/same (2).txt"
    } else {
        "mixed/folder/later.txt"
    }
}

/// The interrupted member first, then every member the header still owes.
fn pending_after(after: usize) -> Vec<&'static str> {
    let mut pending = vec![
        interrupted_after(after),
        "mixed/folder",
        "mixed/folder/same.txt",
    ];
    if after == 1 {
        pending.push("mixed/folder/later.txt");
    }
    pending.extend(["mixed/folder/empty", "mixed/folder/zero.txt"]);
    pending
}

fn assert_mixed_output(destination: &Path, after: usize) -> Result<(), Box<dyn Error>> {
    assert_contents(destination, &[("folder/keep.txt", b"original")], "")?;
    assert_contents(destination, &MIXED_COMPLETED[..after], "")?;
    assert_entry_count(&destination.join("mixed/folder"), after, "")?;
    assert_entry_count(&destination.join("folder"), 1, "")?;
    assert!(!destination.join(interrupted_after(after)).exists());
    Ok(())
}

#[test]
fn zip_extraction_stops_and_drops_incomplete_output_when_cancelled() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let destination = root.path().join("destination");
    fs::create_dir(&destination)?;
    let archive_path = root.path().join("content.zip");
    write_zip(
        &archive_path,
        &[("first.bin", b"early"), ("second.txt", b"late")],
    )?;

    let outcome = Decoder::new(ArchiveFormat::Zip)
        .named("archive")
        .cancelled()
        .run(&archive_path, &destination)?;

    assert_eq!(not_attempted_of(outcome, "").len(), 2);
    assert_empty(&destination, "")?;
    Ok(())
}

#[test]
fn tar_extraction_stops_without_scanning_remaining_entries() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let destination = root.path().join("destination");
    fs::create_dir(&destination)?;
    let archive_path = root.path().join("content.tar");
    write_tar_entries(
        &archive_path,
        &[
            (tar::EntryType::Regular, "first.bin", b"early"),
            (tar::EntryType::Regular, "second.txt", b"late"),
        ],
        false,
    )?;

    let outcome = Decoder::new(ArchiveFormat::Tar)
        .named("archive")
        .cancelled()
        .run(&archive_path, &destination)?;

    assert_eq!(
        not_attempted_of(outcome, ""),
        locations(&destination, &["first.bin"])
    );
    assert_empty(&destination, "")?;
    Ok(())
}

#[test]
fn sevenz_cancellation_keeps_streamless_and_duplicate_members_pending() -> Result<(), Box<dyn Error>>
{
    for solid in [false, true] {
        let root = tempfile::tempdir()?;
        let archive = root.path().join("mixed.7z");
        write_mixed_7z(&archive, solid)?;
        let destination = tempfile::tempdir()?;
        let decoder = Decoder::new(ArchiveFormat::SevenZ)
            .named("archive")
            .cancelled();

        let outcome = decoder.run(&archive, destination.path())?;

        assert_eq!(
            not_attempted_of(outcome, "expected cancellation before the first callback"),
            locations(destination.path(), &MIXED_MEMBERS),
            "solid={solid}"
        );
        assert_eq!(decoder.progress(), 0);
        assert_empty(destination.path(), "")?;
    }
    Ok(())
}

#[test]
fn sevenz_mid_copy_cancellation_tracks_header_identity_and_destination_renames()
-> Result<(), Box<dyn Error>> {
    for solid in [false, true] {
        for after in [1, 2] {
            let context = format!("after={after}, solid={solid}");
            let root = tempfile::tempdir()?;
            let archive = root.path().join("mixed.7z");
            write_mixed_7z(&archive, solid)?;
            let destination = tempfile::tempdir()?;
            fs::create_dir(destination.path().join("folder"))?;
            fs::write(destination.path().join("folder/keep.txt"), b"original")?;
            let progress = Arc::new(AtomicUsize::new(0));
            let cancelled = AtomicBool::new(false);
            let reader = CancelAfterMembers {
                file: fs::File::open(&archive)?,
                progress: &progress,
                cancelled: &cancelled,
                after,
            };

            let outcome = extract_7z_from_reader(
                reader,
                destination.path(),
                "mixed.7z",
                sevenz_rust2::Password::empty(),
                &progress,
                &cancelled,
                &mut TransferReport::default(),
            )?;

            let ArchiveOutcome::Cancelled {
                completed,
                failed,
                not_attempted,
            } = outcome
            else {
                panic!("expected cancellation after {after} members, solid={solid}");
            };
            let completed_names: Vec<_> = MIXED_COMPLETED[..after]
                .iter()
                .map(|(name, _)| *name)
                .collect();
            assert_eq!(completed, locations(destination.path(), &completed_names));
            assert!(failed.is_empty());
            assert_eq!(
                not_attempted,
                locations(destination.path(), &pending_after(after)),
                "{context}"
            );
            assert_mixed_output(destination.path(), after)?;
            assert_eq!(completed.len() + not_attempted.len(), 7);
            assert_eq!(progress.load(Ordering::Relaxed), after);
        }
    }
    Ok(())
}

#[test]
fn pending_unsafe_names_are_sanitized_by_every_decoder() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let archive = root.path().join("archive");
    for format in ALL_FORMATS {
        write_files(&archive, format, &[("../outside", b"contents")])?;
        let destination = tempfile::tempdir()?;
        let decoder = Decoder::new(format).named("archive").cancelled();
        let context = format!("{format:?}");

        let outcome = decoder.run(&archive, destination.path())?;

        assert_eq!(
            not_attempted_of(outcome, &context),
            locations(destination.path(), &["outside"]),
            "{context}"
        );
        assert_eq!(decoder.progress(), 0);
        assert_empty(destination.path(), &context)?;
    }
    Ok(())
}

#[test]
fn sevenz_extraction_reports_remaining_entries_when_cancelled() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let destination = root.path().join("destination");
    fs::create_dir(&destination)?;
    let archive_path = root.path().join("content.7z");
    write_7z_entries(
        &archive_path,
        &[("first.bin", b"early"), ("second.txt", b"late")],
    )?;

    let outcome = Decoder::new(ArchiveFormat::SevenZ)
        .named("archive")
        .cancelled()
        .run(&archive_path, &destination)?;

    assert_eq!(
        not_attempted_of(outcome, ""),
        locations(&destination, &["first.bin", "second.txt"])
    );
    assert_empty(&destination, "")?;
    Ok(())
}
