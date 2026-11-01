// SPDX-License-Identifier: MIT

//! Where members land: root markers, nesting, name collisions and unsafe paths.

use super::*;

#[test]
fn every_decoder_shares_nesting_conflicts_and_progress() -> Result<(), Box<dyn Error>> {
    for (format, password) in [
        (ArchiveFormat::Zip, None),
        (ArchiveFormat::Zip, Some("test-password")),
        (ArchiveFormat::SevenZ, None),
        (ArchiveFormat::SevenZ, Some("test-password")),
        (ArchiveFormat::Tar, None),
        (ArchiveFormat::TarGz, None),
    ] {
        let root = tempfile::tempdir()?;
        let source = root.path().join("folder");
        fs::create_dir_all(source.join("nested"))?;
        fs::create_dir(source.join("empty"))?;
        fs::write(source.join("item.txt"), b"contents")?;
        fs::write(source.join("nested/zero.txt"), b"")?;
        let archive = root.path().join("archive");
        write_compression_fixture(&archive, &[source], format, password)?;
        let destination = root.path().join("destination");
        fs::create_dir_all(destination.join("folder"))?;
        fs::write(destination.join("folder/keep.txt"), b"original")?;
        let decoder = Decoder::with_password(format, password);

        assert_eq!(
            decoder.extract(&archive, &destination)?,
            Some("folder (2)".to_owned()),
            "{format:?}"
        );
        assert_eq!(decoder.progress(), 5, "{format:?}");
        assert_contents(
            &destination,
            &[
                ("folder/keep.txt", b"original"),
                ("folder (2)/item.txt", b"contents"),
                ("folder (2)/nested/zero.txt", b""),
            ],
            "",
        )?;
        assert!(destination.join("folder (2)/empty").is_dir());
    }
    Ok(())
}

#[test]
fn seven_z_extraction_preserves_all_file_contents() -> Result<(), Box<dyn Error>> {
    let entries = [
        ("folder/one.txt", b"first contents".as_slice()),
        ("folder/two.txt", b"second contents".as_slice()),
        ("folder/nested/three.txt", b"third contents".as_slice()),
    ];
    for solid in [true, false] {
        let root = tempfile::tempdir()?;
        let archive_path = root.path().join("files.7z");
        let destination = root.path().join("extracted");
        fs::create_dir(&destination)?;
        let mut writer = sevenz_rust2::ArchiveWriter::create(&archive_path)?;
        push_7z_files(&mut writer, &entries, solid)?;
        writer.finish()?;
        let reader = sevenz_rust2::ArchiveReader::new(
            fs::File::open(&archive_path)?,
            sevenz_rust2::Password::empty(),
        )?;
        assert_eq!(reader.archive().is_solid, solid);
        let decoder = Decoder::new(ArchiveFormat::SevenZ).named("archive");

        assert_eq!(
            decoder.extract(&archive_path, &destination)?,
            Some("folder".to_owned())
        );
        assert_contents(&destination, &entries, "")?;
        assert_eq!(decoder.progress(), entries.len());
    }
    Ok(())
}

#[test]
fn seven_z_extraction_preserves_all_empty_files_and_directories() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let archive_path = root.path().join("empty-entries.7z");
    let destination = root.path().join("extracted");
    fs::create_dir(&destination)?;
    let mut writer = sevenz_rust2::ArchiveWriter::create(&archive_path)?;
    for entry in [
        sevenz_rust2::ArchiveEntry::new_directory("folder"),
        sevenz_rust2::ArchiveEntry::new_file("folder/one.txt"),
        sevenz_rust2::ArchiveEntry::new_directory("folder/empty"),
        sevenz_rust2::ArchiveEntry::new_file("folder/two.txt"),
    ] {
        writer.push_archive_entry::<Cursor<&[u8]>>(entry, None)?;
    }
    writer.finish()?;
    let decoder = Decoder::new(ArchiveFormat::SevenZ).named("archive");

    assert_eq!(
        decoder.extract(&archive_path, &destination)?,
        Some("folder".to_owned())
    );
    assert!(destination.join("folder/empty").is_dir());
    assert_contents(
        &destination,
        &[("folder/one.txt", b""), ("folder/two.txt", b"")],
        "",
    )?;
    assert_eq!(decoder.progress(), 4);
    Ok(())
}

#[test]
fn tar_extraction_skips_root_directories_and_preserves_contents() -> Result<(), Box<dyn Error>> {
    for gzip in [false, true] {
        for root_entry in [None, Some("."), Some("./")] {
            let root = tempfile::tempdir()?;
            let destination = root.path().join("destination");
            fs::create_dir_all(destination.join("folder"))?;
            fs::write(destination.join("folder/keep.txt"), b"keep")?;
            let archive = root.path().join("content.tar");
            let mut entries = Vec::new();
            if let Some(name) = root_entry {
                entries.push((tar::EntryType::Directory, name, b"".as_slice()));
            }
            entries.extend([
                (tar::EntryType::Directory, "./folder/", b"".as_slice()),
                (tar::EntryType::Regular, "./folder/item.txt", b"contents"),
                (tar::EntryType::Regular, "./empty.txt", b""),
            ]);
            write_tar_entries(&archive, &entries, gzip)?;
            let decoder = Decoder::new(tar_format(gzip));

            assert_eq!(
                decoder.extract(&archive, &destination)?,
                Some("content".to_owned()),
            );
            assert_eq!(decoder.progress(), 3);
            assert_contents(
                &destination,
                &[
                    ("content/folder/item.txt", b"contents"),
                    ("folder/keep.txt", b"keep"),
                    ("content/empty.txt", b""),
                ],
                "",
            )?;
            assert_entry_count(&destination, 2, "")?;
        }
    }
    Ok(())
}

#[test]
fn tar_extraction_root_only_completes_without_a_name_and_respects_cancellation()
-> Result<(), Box<dyn Error>> {
    for gzip in [false, true] {
        let root = tempfile::tempdir()?;
        let destination = root.path().join("destination");
        fs::create_dir(&destination)?;
        let archive = root.path().join("content.tar");
        write_tar_entries(&archive, &[(tar::EntryType::Directory, "./", b"")], gzip)?;
        let decoder = Decoder::new(tar_format(gzip)).named("archive");

        assert_eq!(decoder.extract(&archive, &destination)?, None);
        assert_eq!(decoder.progress(), 0);

        let cancelled = Decoder::new(tar_format(gzip)).named("archive").cancelled();
        assert!(not_attempted_of(cancelled.run(&archive, &destination)?, "").is_empty());
        assert_eq!(cancelled.progress(), 0);
        assert_empty(&destination, "")?;
    }
    Ok(())
}

#[test]
fn tar_extraction_rejects_empty_paths_and_root_file_entries() -> Result<(), Box<dyn Error>> {
    for gzip in [false, true] {
        for (entry_type, name) in [
            (tar::EntryType::Directory, ""),
            (tar::EntryType::Directory, "/"),
            (tar::EntryType::Regular, ""),
            (tar::EntryType::Regular, "."),
            (tar::EntryType::Regular, "./"),
            (tar::EntryType::Regular, "././"),
        ] {
            let root = tempfile::tempdir()?;
            let destination = root.path().join("destination");
            fs::create_dir(&destination)?;
            let archive = root.path().join("content.tar");
            write_tar_entries(&archive, &[(entry_type, name, b"")], gzip)?;
            let decoder = Decoder::new(tar_format(gzip)).named("archive");

            assert!(
                matches!(
                    decoder.run(&archive, &destination),
                    Err(ArchiveError::Failed(_))
                ),
                "accepted {entry_type:?} {name:?}, gzip={gzip}"
            );
            assert_eq!(decoder.progress(), 0);
            assert_empty(&destination, "")?;
        }
    }
    Ok(())
}

#[test]
fn every_archive_format_sanitizes_parent_traversal_without_stopping() -> Result<(), Box<dyn Error>>
{
    let root = tempfile::tempdir()?;
    let archive = root.path().join("archive");
    for format in ALL_FORMATS {
        write_files(
            &archive,
            format,
            &[("../escaped.txt", b"escaped"), ("after.txt", b"after")],
        )?;
        let destination = tempfile::tempdir_in(root.path())?;
        let decoder = Decoder::new(format);

        assert_eq!(
            decoder.extract(&archive, destination.path())?,
            Some("archive".to_owned()),
            "{format:?}"
        );
        assert_contents(
            destination.path(),
            &[
                ("archive/escaped.txt", b"escaped"),
                ("archive/after.txt", b"after"),
            ],
            "",
        )?;
        assert!(!root.path().join("escaped.txt").exists());
        assert_eq!(decoder.progress(), 2);
    }
    Ok(())
}

#[test]
fn destination_symlinks_are_never_followed_and_colliding_roots_are_suffixed()
-> Result<(), Box<dyn Error>> {
    for format in ALL_FORMATS {
        for (name, published, written) in [
            ("dangling", "dangling (2)", "dangling (2)"),
            ("redirect/marker", "redirect (2)", "redirect (2)/marker"),
        ] {
            let root = tempfile::tempdir()?;
            let destination = root.path().join("destination");
            let external = root.path().join("external");
            fs::create_dir(&destination)?;
            fs::create_dir(&external)?;
            std::os::unix::fs::symlink(root.path().join("missing"), destination.join("dangling"))?;
            std::os::unix::fs::symlink(&external, destination.join("redirect"))?;
            let archive = root.path().join("archive");
            write_files(&archive, format, &[(name, b"escaped")])?;
            let context = format!("{format:?} {name}");

            assert_eq!(
                Decoder::new(format).extract(&archive, &destination)?,
                Some(published.to_owned()),
                "{context}"
            );
            assert_contents(&destination, &[(written, b"escaped")], &context)?;
            assert!(!root.path().join("missing").exists());
            assert!(!external.join("marker").exists());
            assert_symlink(
                &destination.join("dangling"),
                root.path().join("missing"),
                &context,
            )?;
            assert_symlink(&destination.join("redirect"), &external, &context)?;
        }
    }
    Ok(())
}

#[test]
fn multi_root_zip_keeps_member_names_inside_the_fresh_folder() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let destination = root.path().join("destination");
    fs::create_dir(&destination)?;
    fs::write(destination.join("report.txt"), b"original")?;
    fs::create_dir(destination.join("existing"))?;
    fs::write(destination.join("existing/old.txt"), b"old")?;
    let archive_path = root.path().join("content.zip");
    write_zip(
        &archive_path,
        &[
            ("folder/nested/item.txt", b"nested"),
            ("report.txt", b"replacement"),
            ("existing/new.txt", b"new"),
        ],
    )?;

    assert_eq!(
        extract_zip(&archive_path, &destination)?,
        Some("content".to_owned())
    );
    assert_contents(
        &destination,
        &[
            ("content/folder/nested/item.txt", b"nested"),
            ("content/report.txt", b"replacement"),
            ("content/existing/new.txt", b"new"),
            ("report.txt", b"original"),
            ("existing/old.txt", b"old"),
        ],
        "",
    )?;
    assert_entry_count(&destination.join("existing"), 1, "")?;
    assert_entry_count(&destination, 3, "")?;
    Ok(())
}

#[test]
fn zip_extraction_skips_root_directories_and_preserves_contents() -> Result<(), Box<dyn Error>> {
    for root_entry in ["./", ".\\"] {
        let root = tempfile::tempdir()?;
        let destination = root.path().join("destination");
        fs::create_dir(&destination)?;
        let archive = root.path().join("root-entry.zip");
        write_zip(
            &archive,
            &[
                (root_entry, b""),
                (
                    "./hello.txt",
                    b"Hello from a ZIP with a root directory marker.\n",
                ),
            ],
        )?;
        let decoder = Decoder::new(ArchiveFormat::Zip);
        let context = format!("{root_entry:?}");

        assert_eq!(
            decoder.extract(&archive, &destination)?,
            Some("hello.txt".to_owned()),
            "{context}"
        );
        assert_eq!(decoder.progress(), 1, "{context}");
        assert_contents(
            &destination,
            &[(
                "hello.txt",
                b"Hello from a ZIP with a root directory marker.\n",
            )],
            &context,
        )?;
        assert_entry_count(&destination, 1, &context)?;
    }
    Ok(())
}

#[test]
fn sevenz_extraction_skips_a_root_directory_and_preserves_contents() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let destination = root.path().join("destination");
    fs::create_dir(&destination)?;
    let archive = root.path().join("content.7z");
    let mut writer = sevenz_rust2::ArchiveWriter::create(&archive)?;
    writer.push_archive_entry::<io::Cursor<&[u8]>>(
        sevenz_rust2::ArchiveEntry::new_directory("."),
        None,
    )?;
    writer.push_archive_entry(
        sevenz_rust2::ArchiveEntry::new_file("./hello.txt"),
        Some(io::Cursor::new(&b"hello\n"[..])),
    )?;
    writer.finish()?;
    let decoder = Decoder::new(ArchiveFormat::SevenZ);

    assert_eq!(
        decoder.extract(&archive, &destination)?,
        Some("hello.txt".to_owned())
    );
    assert_eq!(decoder.progress(), 1);
    assert_eq!(fs::read(destination.join("hello.txt"))?, b"hello\n");
    Ok(())
}

#[test]
fn zip_extraction_rejects_root_files_and_parent_only_directories() -> Result<(), Box<dyn Error>> {
    for name in [".", "../", "foo/../"] {
        let root = tempfile::tempdir()?;
        let destination = root.path().join("destination");
        fs::create_dir(&destination)?;
        let archive = root.path().join("content.zip");
        write_zip(&archive, &[(name, b"payload")])?;
        let decoder = Decoder::new(ArchiveFormat::Zip);

        match decoder.run(&archive, &destination) {
            Err(ArchiveError::Failed(message)) => {
                assert!(
                    message.contains("Refusing empty archive path") && message.contains(name),
                    "{name:?}: {message}"
                );
            }
            other => panic!("accepted {name:?}: {other:?}"),
        }
        assert_eq!(decoder.progress(), 0, "{name:?}");
        assert_empty(
            &destination,
            &format!("{name:?} wrote into the destination"),
        )?;
    }
    Ok(())
}

#[test]
fn tar_extraction_skips_pax_global_headers() -> Result<(), Box<dyn Error>> {
    for gzip in [false, true] {
        let root = tempfile::tempdir()?;
        let destination = root.path().join("destination");
        fs::create_dir_all(&destination)?;
        let archive = root.path().join("project.tar");
        write_tar_entries(
            &archive,
            &[
                (
                    tar::EntryType::XGlobalHeader,
                    "pax_global_header",
                    b"52 comment=0123456789abcdef0123456789abcdef01234567\n".as_slice(),
                ),
                (tar::EntryType::Directory, "project/", b"".as_slice()),
                (tar::EntryType::Regular, "project/README", b"hello"),
            ],
            gzip,
        )?;
        let decoder = Decoder::new(tar_format(gzip)).named("archive");

        assert_eq!(
            decoder.extract(&archive, &destination)?,
            Some("project".to_owned()),
        );
        assert_eq!(decoder.progress(), 2);
        assert!(!destination.join("pax_global_header").exists());
        assert_contents(&destination, &[("project/README", b"hello")], "")?;
        assert_entry_count(&destination, 1, "")?;
    }
    Ok(())
}

#[test]
fn highly_compressible_archives_extract_in_every_format() -> Result<(), Box<dyn Error>> {
    const SIZE: u64 = 16 << 20;
    let zeros = vec![0u8; SIZE as usize];
    for format in [
        ArchiveFormat::Zip,
        ArchiveFormat::SevenZ,
        ArchiveFormat::TarGz,
    ] {
        let root = tempfile::tempdir()?;
        let archive = root.path().join("zeros.archive");
        write_files(&archive, format, &[("zeros.bin", &zeros)])?;
        let ratio = SIZE / fs::metadata(&archive)?.len();
        assert!(
            ratio > 100,
            "{format:?} fixture should compress well, got {ratio}:1"
        );
        let destination = tempfile::tempdir()?;
        let decoder = Decoder::new(format);

        let first_name = decoder.extract(&archive, destination.path())?;

        assert_eq!(first_name.as_deref(), Some("zeros.bin"), "{format:?}");
        assert_eq!(
            fs::metadata(destination.path().join("zeros.bin"))?.len(),
            SIZE,
            "{format:?} should extract the full member"
        );
        assert_eq!(decoder.progress(), 1, "{format:?}");
    }
    Ok(())
}
