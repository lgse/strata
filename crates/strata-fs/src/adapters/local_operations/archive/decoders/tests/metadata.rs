// SPDX-License-Identifier: MIT

//! Links, modes and times: what each format restores beyond file contents.

use super::*;

const STORED_MODES: [(&str, u32); 6] = [
    ("bin/run.sh", 0o755),
    ("data.txt", 0o600),
    ("suid", 0o4755),
    ("readonly.txt", 0o444),
    ("ro/child.txt", 0o644),
    ("ro", 0o555),
];

fn mode_members() -> [FixtureMember<'static>; 6] {
    let stored = Some(STORED_TIME);
    [
        FixtureMember::File {
            name: "bin/run.sh",
            contents: b"#!/bin/sh\necho ok\n",
            mode: Some(0o755),
            modified: stored,
        },
        FixtureMember::File {
            name: "data.txt",
            contents: b"private",
            mode: Some(0o600),
            modified: stored,
        },
        FixtureMember::File {
            name: "suid",
            contents: b"setuid",
            mode: Some(0o4755),
            modified: stored,
        },
        FixtureMember::File {
            name: "readonly.txt",
            contents: b"read only",
            mode: Some(0o444),
            modified: stored,
        },
        FixtureMember::Directory {
            name: "ro",
            mode: Some(0o555),
            modified: stored,
        },
        FixtureMember::File {
            name: "ro/child.txt",
            contents: b"child",
            mode: Some(0o644),
            modified: stored,
        },
    ]
}

/// A file with symlinks to it, optionally a second `lnk` member and a hard link.
fn link_members(duplicate: bool, hard: bool) -> Vec<FixtureMember<'static>> {
    let mut members = vec![
        FixtureMember::File {
            name: "data.txt",
            contents: b"payload",
            mode: None,
            modified: None,
        },
        FixtureMember::Symlink {
            name: "lnk",
            target: b"data.txt",
        },
        FixtureMember::Symlink {
            name: "abs",
            target: b"/etc/hostname",
        },
        FixtureMember::Symlink {
            name: "nested/up",
            target: b"../data.txt",
        },
    ];
    if duplicate {
        members.push(FixtureMember::Symlink {
            name: "lnk",
            target: b"abs",
        });
    }
    if hard {
        members.push(FixtureMember::HardLink {
            name: "hard",
            target: "data.txt",
        });
    }
    members
}

fn assert_links_extracted(
    output: &Path,
    duplicate: bool,
    hard: bool,
    context: &str,
) -> Result<(), Box<dyn Error>> {
    assert_symlink(&output.join("lnk"), "data.txt", context)?;
    assert_symlink(&output.join("abs"), "/etc/hostname", context)?;
    assert_symlink(&output.join("nested/up"), "../data.txt", context)?;
    assert_contents(
        output,
        &[("lnk", b"payload"), ("nested/up", b"payload")],
        context,
    )?;
    if duplicate {
        assert_symlink(&output.join("lnk (2)"), "abs", context)?;
    }
    if hard {
        assert_hard_link(&output.join("hard"), &output.join("data.txt"), context)?;
        assert_contents(output, &[("hard", b"payload")], context)?;
    }
    Ok(())
}

/// A TAR whose hard links name byte-distinct, non-UTF-8 targets.
fn write_native_hard_links(archive: &Path, names: &[&[u8]]) -> Result<(), Box<dyn Error>> {
    use std::os::unix::ffi::OsStrExt;

    let mut builder = tar::Builder::new(fs::File::create(archive)?);
    for (index, name) in names.iter().enumerate() {
        let mut header = tar::Header::new_gnu();
        header.set_path(Path::new(OsStr::from_bytes(name)))?;
        header.set_mode(0o644);
        header.set_size(1);
        header.set_cksum();
        builder.append(&header, &[index as u8][..])?;
    }
    for (index, target) in names.iter().enumerate() {
        let mut header = tar::Header::new_gnu();
        header.set_path(format!("hard-{index}"))?;
        header.set_entry_type(tar::EntryType::Link);
        header.set_link_name_literal(target)?;
        header.set_mode(0o644);
        header.set_size(0);
        header.set_cksum();
        builder.append(&header, io::empty())?;
    }
    builder.finish()?;
    Ok(())
}

/// `ok.txt` followed by a member of `entry_type`, or a hard link to a missing file.
fn write_unsupported_member(
    archive: &Path,
    format: ArchiveFormat,
    name: &str,
    entry_type: Option<tar::EntryType>,
) -> Result<(), Box<dyn Error>> {
    match entry_type {
        Some(entry_type) => write_tar_entries(
            archive,
            &[
                (tar::EntryType::Regular, "ok.txt", b"ok"),
                (entry_type, name, b""),
            ],
            format == ArchiveFormat::TarGz,
        ),
        None => write_members(
            archive,
            format,
            &[
                FixtureMember::File {
                    name: "ok.txt",
                    contents: b"ok",
                    mode: None,
                    modified: None,
                },
                FixtureMember::HardLink {
                    name,
                    target: "missing.txt",
                },
            ],
        ),
    }
}

fn now_seconds() -> Result<u64, Box<dyn Error>> {
    Ok(std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs())
}

#[test]
fn links_extract_as_links_in_every_format() -> Result<(), Box<dyn Error>> {
    for format in ALL_FORMATS {
        let context = format!("{format:?}");
        let root = tempfile::tempdir()?;
        let archive = root.path().join(format!("links.{}", format.extension()));
        let hard = matches!(format, ArchiveFormat::Tar | ArchiveFormat::TarGz);
        // The ZIP writer refuses duplicate member names.
        let duplicate = format != ArchiveFormat::Zip;
        let members = link_members(duplicate, hard);
        write_members(&archive, format, &members)?;
        let destination = root.path().join("destination");
        fs::create_dir(&destination)?;
        let decoder = Decoder::new(format);

        let first_name = decoder.extract(&archive, &destination)?;

        assert_eq!(first_name.as_deref(), Some("links"), "{context}");
        assert_eq!(decoder.progress(), members.len(), "{context}");
        assert_links_extracted(&destination.join("links"), duplicate, hard, &context)?;
        assert_eq!(names_in(&destination)?, ["links"], "{context}");
    }
    Ok(())
}

#[test]
fn tar_hard_links_preserve_distinct_native_target_names() -> Result<(), Box<dyn Error>> {
    use std::os::unix::ffi::OsStrExt;

    let root = tempfile::tempdir()?;
    let archive = root.path().join("native.tar");
    let names: [&[u8]; 2] = [b"a\xff", b"a\xfe"];
    write_native_hard_links(&archive, &names)?;
    let destination = root.path().join("destination");
    fs::create_dir(&destination)?;

    Decoder::new(ArchiveFormat::Tar).extract(&archive, &destination)?;

    let output = destination.join("native");
    for (index, name) in names.iter().enumerate() {
        let original = output.join(OsStr::from_bytes(name));
        let linked = output.join(format!("hard-{index}"));
        assert_eq!(fs::read(&linked)?, [index as u8]);
        assert_hard_link(&linked, &original, "")?;
    }
    Ok(())
}

#[test]
fn unsupported_tar_members_are_refused_with_their_name() -> Result<(), Box<dyn Error>> {
    for format in [ArchiveFormat::Tar, ArchiveFormat::TarGz] {
        for (name, entry_type) in [
            ("pipe", Some(tar::EntryType::Fifo)),
            ("char-device", Some(tar::EntryType::Char)),
            ("block-device", Some(tar::EntryType::Block)),
            ("dangling", None),
        ] {
            let context = format!("{format:?} {name}");
            let root = tempfile::tempdir()?;
            let archive = root.path().join(format!("special.{}", format.extension()));
            write_unsupported_member(&archive, format, name, entry_type)?;
            let destination = root.path().join("destination");
            fs::create_dir(&destination)?;
            let decoder = Decoder::new(format);

            let result = decoder.run(&archive, &destination);

            let output = destination.join("special");
            let message = match result {
                Err(ArchiveError::Failed(message)) => message,
                other => panic!(
                    "{context}: expected a failure naming `{name}`, got {other:?}; \
                     `special/{name}` is {}",
                    kind_of(&output.join(name))
                ),
            };
            assert!(message.contains(name), "{context}: {message}");
            assert!(
                message.contains("remain in “special”"),
                "{context}: {message}"
            );
            assert_contents(&output, &[("ok.txt", b"ok")], &context)?;
            assert!(
                fs::symlink_metadata(output.join(name)).is_err(),
                "{context}"
            );
            assert!(!destination.join("ok.txt").exists(), "{context}");
            assert_eq!(decoder.progress(), 1, "{context}");
        }
    }
    Ok(())
}

#[test]
fn a_member_cannot_be_written_through_an_extracted_symlink() -> Result<(), Box<dyn Error>> {
    for format in [
        ArchiveFormat::Zip,
        ArchiveFormat::SevenZ,
        ArchiveFormat::Tar,
    ] {
        let context = format!("{format:?}");
        let root = tempfile::tempdir()?;
        let external = root.path().join("external");
        fs::create_dir(&external)?;
        let target = external.to_str().ok_or("non-UTF-8 temporary path")?;
        let archive = root.path().join(format!("escape.{}", format.extension()));
        write_members(
            &archive,
            format,
            &[
                FixtureMember::Symlink {
                    name: "lnk",
                    target: target.as_bytes(),
                },
                FixtureMember::File {
                    name: "lnk/escape.txt",
                    contents: b"escaped",
                    mode: None,
                    modified: None,
                },
            ],
        )?;
        let destination = root.path().join("destination");
        fs::create_dir(&destination)?;

        let result = Decoder::new(format).run(&archive, &destination);

        let Err(ArchiveError::Failed(message)) = &result else {
            panic!(
                "{context}: {result:?}; destination holds {:?}",
                names_in(&destination)?
            );
        };
        assert!(
            message.starts_with("Refusing to extract through the symbolic link “lnk”."),
            "{context}: {message}"
        );
        assert!(!external.join("escape.txt").exists(), "{context}");
        assert_symlink(&destination.join("escape/lnk"), target, &context)?;
    }
    Ok(())
}

#[test]
fn member_modes_and_times_are_restored_in_every_format() -> Result<(), Box<dyn Error>> {
    let members = mode_members();
    for format in ALL_FORMATS {
        let context = format!("{format:?}");
        let root = tempfile::tempdir()?;
        let archive = root.path().join(format!("modes.{}", format.extension()));
        write_members(&archive, format, &members)?;
        let destination = root.path().join("destination");
        fs::create_dir(&destination)?;
        let decoder = Decoder::new(format);

        let first_name = decoder.extract(&archive, &destination)?;

        assert_eq!(first_name.as_deref(), Some("modes"), "{context}");
        assert_eq!(decoder.progress(), members.len(), "{context}");
        let output = destination.join("modes");
        let time = i64::try_from(STORED_TIME)?;
        for (name, mode) in STORED_MODES {
            assert_mode_and_time(&output.join(name), mode, time, &context)?;
        }
        fs::set_permissions(output.join("ro"), fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

#[test]
fn members_without_a_usable_mode_keep_the_default_permissions() -> Result<(), Box<dyn Error>> {
    // ZIP external attributes `0x20` carry only the DOS archive bit, so a
    // Unix creator reports mode 0. The 7z junk mode has no regular-file type.
    for (format, mode, zip_attributes) in [
        (ArchiveFormat::Zip, None, Some(0)),
        (ArchiveFormat::Zip, None, Some(0x20)),
        (ArchiveFormat::SevenZ, None, None),
        (ArchiveFormat::SevenZ, Some(0o070_644), None),
    ] {
        let context = format!("{format:?} {mode:?} {zip_attributes:?}");
        let root = tempfile::tempdir()?;
        let archive = root.path().join(format!("plain.{}", format.extension()));
        write_members(
            &archive,
            format,
            &[FixtureMember::File {
                name: "plain.txt",
                contents: b"plain",
                mode,
                modified: None,
            }],
        )?;
        if let Some(attributes) = zip_attributes {
            patch_zip_external_attributes(&archive, attributes)?;
        }
        let destination = root.path().join("destination");
        fs::create_dir(&destination)?;
        let before = now_seconds()?;

        Decoder::new(format).extract(&archive, &destination)?;

        let after = now_seconds()?;
        let extracted = destination.join("plain.txt");
        if format == ArchiveFormat::Zip {
            // The DOS default (1980-01-01 local) is a stored time.
            let dos_default = glib::DateTime::from_local(1980, 1, 1, 0, 0, 0.0)?.to_unix();
            assert_mode_and_time(&extracted, 0o666, dos_default, &context)?;
        } else {
            let modified = u64::try_from(fs::metadata(&extracted)?.mtime())?;
            // File timestamps use the kernel's coarse clock, which can trail `now()`.
            assert!(
                (before.saturating_sub(1)..=after).contains(&modified),
                "{context}"
            );
            assert_eq!(
                fs::metadata(&extracted)?.permissions().mode() & 0o7777,
                expected_mode(0o666),
                "{context}"
            );
        }
    }
    Ok(())
}

#[test]
fn zip_prefers_the_extended_timestamp_over_the_dos_time() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let archive = root.path().join("times.zip");
    let dos = zip::DateTime::from_date_and_time(2000, 1, 1, 0, 0, 0)?;
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored)
        .last_modified_time(dos);
    let mut extended = options.into_full_options();
    extended.add_extra_field(0x5455, zip_extended_timestamp(STORED_TIME)?, false)?;
    let mut writer = zip::ZipWriter::new(fs::File::create(&archive)?);
    writer.start_file("extended.txt", extended)?;
    writer.write_all(b"extended")?;
    writer.start_file("dos.txt", options)?;
    writer.write_all(b"dos")?;
    // A signed UT time of -1 is before 1970, so the DOS time applies.
    let mut negative = options.into_full_options();
    negative.add_extra_field(0x5455, [1, 0xff, 0xff, 0xff, 0xff], false)?;
    writer.start_file("negative.txt", negative)?;
    writer.write_all(b"negative")?;
    writer.finish()?;
    let mut reader = zip::ZipArchive::new(fs::File::open(&archive)?)?;
    assert!(
        reader
            .by_name("extended.txt")?
            .extra_data_fields()
            .any(|field| matches!(
                field,
                zip::ExtraField::ExtendedTimestamp(stamp)
                    if stamp.mod_time().map(u64::from) == Some(STORED_TIME)
            )),
        "fixture lacks the UT field"
    );
    let destination = root.path().join("destination");
    fs::create_dir(&destination)?;

    let first_name = Decoder::new(ArchiveFormat::Zip).extract(&archive, &destination)?;

    assert_eq!(first_name.as_deref(), Some("times"));
    assert_eq!(
        fs::metadata(destination.join("times/extended.txt"))?.mtime(),
        i64::try_from(STORED_TIME)?
    );
    let dos_local = glib::DateTime::from_local(2000, 1, 1, 0, 0, 0.0)?.to_unix();
    for name in ["dos.txt", "negative.txt"] {
        let mtime = fs::metadata(destination.join("times").join(name))?.mtime();
        assert_eq!(mtime, dos_local, "{name}");
    }
    Ok(())
}
