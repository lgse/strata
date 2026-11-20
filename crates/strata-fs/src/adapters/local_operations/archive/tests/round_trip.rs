// SPDX-License-Identifier: MIT

//! Archives Strata makes, extracted again through the provider.

use super::*;

const SOURCE_TIME: i64 = 1_000_000_000;

/// The source paths given their own modes, `""` being the source folder itself.
const MODES: [(&str, u32); 5] = [
    ("run.sh", 0o755),
    ("secret.txt", 0o600),
    ("inner/note.txt", 0o644),
    ("inner", 0o700),
    ("", 0o750),
];

/// A `source` folder with its `MODES` and, when asked, a `link`, all at `SOURCE_TIME`.
fn write_moded_source(root: &Path, with_link: bool) -> Result<PathBuf, Box<dyn Error>> {
    let source = root.join("source");
    fs::create_dir_all(source.join("inner"))?;
    fs::write(source.join("run.sh"), b"#!/bin/sh\necho ok\n")?;
    fs::write(source.join("secret.txt"), b"secret")?;
    fs::write(source.join("inner/note.txt"), b"note")?;
    if with_link {
        std::os::unix::fs::symlink("run.sh", source.join("link"))?;
        set_times_without_following(&source.join("link"), SOURCE_TIME)?;
    }
    for (name, mode) in MODES {
        let path = source.join(name);
        fs::set_permissions(&path, fs::Permissions::from_mode(mode))?;
        set_times_without_following(&path, SOURCE_TIME)?;
    }
    Ok(source)
}

fn expect_link_restored(output: &Path, context: &str) -> Result<(), Box<dyn Error>> {
    let link = output.join("link");
    let metadata = fs::symlink_metadata(&link)?;
    assert!(
        metadata.file_type().is_symlink(),
        "{context}: `source/link` extracted as a {} of {} bytes, not as a symlink",
        if metadata.is_dir() {
            "directory"
        } else {
            "regular file"
        },
        metadata.len()
    );
    assert_eq!(fs::read_link(&link)?, Path::new("run.sh"), "{context}");
    assert_eq!(metadata.mtime(), SOURCE_TIME, "{context}: link mtime");
    Ok(())
}

fn expect_modes_restored(output: &Path, context: &str) -> Result<(), Box<dyn Error>> {
    for (name, mode) in MODES {
        let metadata = fs::metadata(output.join(name))?;
        assert_eq!(
            format!("{:o}", metadata.permissions().mode() & 0o7777),
            format!("{:o}", expected_mode(mode)),
            "{context}: mode of `source/{name}`"
        );
        assert_eq!(
            metadata.mtime(),
            SOURCE_TIME,
            "{context}: mtime of `source/{name}`"
        );
    }
    Ok(())
}

fn round_trip(format: ArchiveFormat) -> Result<(), Box<dyn Error>> {
    let context = format!("{format:?}");
    let root = tempfile::tempdir()?;
    let with_link = format != ArchiveFormat::SevenZ;
    let source = write_moded_source(root.path(), with_link)?;
    let archives = root.path().join("archives");
    fs::create_dir(&archives)?;
    let events = run_compression(CompressRequest {
        format,
        ..zip_request(&source, &archives, "source", TransferConflict::FailIfExists)
    });
    assert!(
        events
            .iter()
            .any(|event| matches!(event, OperationEvent::Compressed { .. })),
        "{context}: {events:?}"
    );
    let extracted = root.path().join("extracted");
    fs::create_dir(&extracted)?;

    let archive = archives.join(format!("source.{}", format.extension()));
    let events = run_extraction(extract_request(2, &archive, &extracted));

    expect_extracted_as(&events, "source");
    let output = extracted.join("source");
    if with_link {
        expect_link_restored(&output, &context)?;
    }
    expect_modes_restored(&output, &context)
}

#[test]
fn strata_archives_round_trip_links_modes_and_times() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    for format in [
        ArchiveFormat::Zip,
        ArchiveFormat::Tar,
        ArchiveFormat::TarGz,
        ArchiveFormat::SevenZ,
    ] {
        round_trip(format)?;
    }
    Ok(())
}

#[test]
fn password_failures_reach_the_event_as_their_kind_and_leave_nothing_behind()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    for format in [ArchiveFormat::Zip, ArchiveFormat::SevenZ] {
        let root = tempfile::tempdir()?;
        let sources = ["first.txt", "second.txt"].map(|name| root.path().join(name));
        for source in &sources {
            fs::write(source, b"contents")?;
        }
        let archive = root.path().join(format!("secret.{}", format.extension()));
        super::super::write_compression_fixture(&archive, &sources, format, Some("test-password"))?;
        for (password, expected) in [
            (None, PasswordFailure::Required),
            (Some("wrong-password"), PasswordFailure::Incorrect),
        ] {
            let destination = tempfile::tempdir()?;

            let events = run_extraction(ExtractRequest {
                password: password.map(str::to_owned),
                ..extract_request(8, &archive, destination.path())
            });

            assert!(
                matches!(
                    events.last(),
                    Some(OperationEvent::Failed { message, password_failure: Some(kind), .. })
                        if *kind == expected && !message.contains("remain")
                ),
                "{format:?} {password:?}: {events:?}"
            );
            assert!(
                entry_names(destination.path())?.is_empty(),
                "{format:?} {password:?}"
            );
        }
    }
    Ok(())
}

/// A `10:00` folder holding `a.txt` and a `lnk` to it, names a FAT stick cannot keep.
fn write_venv(archive: &Path, format: ArchiveFormat) -> Result<(), Box<dyn Error>> {
    write_members(
        archive,
        format,
        &[
            FixtureMember::Directory {
                name: "10:00",
                mode: None,
                modified: None,
            },
            FixtureMember::File {
                name: "10:00/a.txt",
                contents: b"a",
                mode: None,
                modified: None,
            },
            FixtureMember::Symlink {
                name: "10:00/lnk",
                target: b"a.txt",
            },
        ],
    )?;
    Ok(())
}

#[test]
fn an_extraction_reports_what_it_renamed_and_left_out_before_it_ends() -> Result<(), Box<dyn Error>>
{
    let _serial = serial()?;
    for (format, name, damaged, output) in [
        (ArchiveFormat::TarGz, "venv.tar.gz", false, ""),
        (ArchiveFormat::TarGz, "venv.tar.gz", true, "venv"),
        (ArchiveFormat::Zip, "venv.zip", false, ""),
        (ArchiveFormat::SevenZ, "venv.7z", false, ""),
    ] {
        let root = tempfile::tempdir()?;
        let archive = root.path().join(name);
        write_venv(&archive, format)?;
        if damaged {
            corrupt_gzip_trailer(&archive, 8)?;
        }
        let stick = root.path().join("stick");
        fs::create_dir(&stick)?;
        let _quirks = FsQuirks::install(&stick, Quirks::vfat());

        let events = run_extraction(extract_request(9, &archive, &stick));

        let [.., OperationEvent::ItemsReported { report, .. }, ending] = events.as_slice() else {
            panic!("no report came before the end: {events:?}");
        };
        assert_eq!(
            matches!(ending, OperationEvent::Failed { .. }),
            damaged,
            "{name}: {ending:?}"
        );
        let output = stick.join(output);
        assert_eq!(
            report.renamed,
            [RenamedItem {
                requested: Location::local(output.join("10:00")),
                saved_as: Location::local(output.join("10_00")),
                cause: RenameCause::Invalid,
            }],
            "{name}"
        );
        assert_eq!(
            report.skipped,
            [SkippedItem {
                location: Location::local(output.join("10_00/lnk")),
                kind: SpecialKind::Symlink,
            }],
            "{name}"
        );
        assert_eq!(fs::read(output.join("10_00/a.txt"))?, b"a", "{name}");
    }
    Ok(())
}
