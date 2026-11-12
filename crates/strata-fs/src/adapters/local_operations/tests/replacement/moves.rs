// SPDX-License-Identifier: MIT

//! A replacing move on one filesystem places the source itself over its target.

use super::*;

#[test]
fn same_filesystem_replacing_move_places_the_source_without_a_stage() -> Result<(), Box<dyn Error>>
{
    let _serial = serial()?;
    let mounts: [(FsWord, Preset); 3] = [
        (NFS_SUPER_MAGIC, Quirks::default),
        (FUSE_SUPER_MAGIC, Quirks::sshfs),
        (FUSE_SUPER_MAGIC, Quirks::vboxsf),
    ];
    for (fs_type, quirks) in mounts {
        for directory in [false, true] {
            let root = tempfile::tempdir()?;
            let sources = root.path().join("sources");
            let destination = root.path().join("destination");
            fs::create_dir(&sources)?;
            fs::create_dir(&destination)?;
            let source = sources.join("item");
            let target = destination.join("item");
            write_item(&source, directory, b"new")?;
            write_item(&target, directory, b"old")?;
            let moved = inode(&source)?;
            let quirks = FsQuirks::install(root.path(), without_trash(fs_type, quirks()));

            let (result, replaced) = replace_reporting(&source, &target, true);

            assert!(result.is_ok(), "{result:?}");
            assert_eq!(replaced, [ReplacedOriginal::Deleted]);
            assert_eq!(inode(&target)?, moved);
            expect_left(&target, directory, b"new", &["item"]);
            assert!(!source.exists());
            assert!(!quirks.used_name(".strata-copy-*"));
        }
    }
    Ok(())
}

#[test]
fn same_filesystem_replacing_move_refuses_a_substituted_source() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    for quirks in [Quirks::default, Quirks::without_rename_flags] {
        let root = tempfile::tempdir()?;
        let source = root.path().join("source");
        let preserved = root.path().join("preserved");
        let target = root.path().join("target");
        fs::write(&source, b"incoming")?;
        fs::write(&target, b"old")?;
        let original = inode(&target)?;
        let (racer_source, racer_preserved) = (source.clone(), preserved.clone());
        let racer: Racer = Box::new(move || {
            let _ = fs::rename(&racer_source, &racer_preserved);
            let _ = fs::write(&racer_source, b"new arrival");
        });
        let _quirks = FsQuirks::install(
            root.path(),
            without_trash(
                NFS_SUPER_MAGIC,
                Quirks {
                    before: vec![(Op::Rename, "target", 0, racer)],
                    ..quirks()
                },
            ),
        );

        let (result, replaced) = replace_reporting(&source, &target, true);

        let error = result.expect_err("a substituted source is refused");
        assert!(error.message().contains("changed"), "{error}");
        assert!(replaced.is_empty());
        assert_eq!(inode(&target)?, original);
        expect_contents(&[
            (target, b"old"),
            (source, b"new arrival"),
            (preserved, b"incoming"),
        ]);
        expect_names(root.path(), &["preserved", "source", "target"], "")?;
    }
    Ok(())
}

#[test]
fn same_filesystem_replacing_move_restores_the_original_when_placement_fails()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    let target = root.path().join("target");
    fs::write(&source, b"incoming")?;
    fs::write(&target, b"old")?;
    let original = inode(&target)?;
    let quirks = FsQuirks::install(
        root.path(),
        without_trash(
            NFS_SUPER_MAGIC,
            Quirks {
                fail: vec![(Op::Rename, "source", 0, Errno::IO)],
                ..Quirks::default()
            },
        ),
    );

    let (result, replaced) = replace_reporting(&source, &target, true);

    assert!(result.is_err());
    assert!(replaced.is_empty());
    expect_set_aside(&quirks, root.path(), &["source", "target"])?;
    assert_eq!(inode(&target)?, original);
    expect_contents(&[(target, b"old"), (source, b"incoming")]);
    Ok(())
}

#[test]
fn a_replacing_move_on_ntfs_3g_renames_names_it_would_cut_or_puts_the_original_back()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let (incoming, name) = ("y".repeat(237), "x".repeat(237));
    for (directory, placing_fails) in [(true, false), (false, false), (true, true)] {
        let case = format!("folder: {directory}, placing fails: {placing_fails}");
        let root = tempfile::tempdir()?;
        let sources = root.path().join("sources");
        let destination = root.path().join("destination");
        fs::create_dir(&sources)?;
        fs::create_dir(&destination)?;
        let source = sources.join(&incoming);
        let target = destination.join(&name);
        write_item(&source, directory, b"new")?;
        write_item(&target, directory, b"old")?;
        let (moved, original) = (inode(&source)?, inode(&target)?);
        // Skips the refused RENAME_NOREPLACE, failing the rename once the name is found free.
        let fail = if placing_fails {
            vec![(Op::Rename, "y*", 1, Errno::ACCESS)]
        } else {
            Vec::new()
        };
        let quirks = FsQuirks::install(
            root.path(),
            without_trash(
                FUSE_SUPER_MAGIC,
                Quirks {
                    rename_flags: Support::Rejected(Errno::INVAL),
                    fail,
                    ..Quirks::ntfs_3g()
                },
            ),
        );

        let (result, replaced) = replace_reporting(&source, &target, true);

        assert_eq!(result.is_err(), placing_fails, "{case}: {result:?}");
        let (kept, contents, reported) = if placing_fails {
            (original, b"old", Vec::new())
        } else {
            (moved, b"new", vec![ReplacedOriginal::Deleted])
        };
        assert_eq!(replaced, reported, "{case}");
        assert_eq!(inode(&target)?, kept, "{case}");
        expect_left(&target, directory, contents, &[name.as_str()]);
        assert_eq!(source.exists(), placing_fails, "{case}");
        assert!(!quirks.used_name(".strata-copy-*"), "{case}");
    }
    Ok(())
}

#[test]
fn a_true_alias_is_a_no_op_but_a_hard_link_elsewhere_is_replaced() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let photo = root.path().join("Photo.txt");
    let link = root.path().join("link.txt");
    fs::write(&photo, b"photo")?;
    fs::hard_link(&photo, &link)?;
    let original = inode(&photo)?;
    let _quirks = FsQuirks::install(
        root.path(),
        without_trash(
            NFS_SUPER_MAGIC,
            Quirks {
                case_insensitive: true,
                ..Quirks::default()
            },
        ),
    );

    let (aliased, aliased_replaced) =
        replace_reporting(&photo, &root.path().join("photo.txt"), false);
    let (linked, link_replaced) = replace_reporting(&photo, &link, false);

    assert!(aliased.is_ok(), "{aliased:?}");
    assert!(aliased_replaced.is_empty());
    assert!(linked.is_ok(), "{linked:?}");
    let photo_left = (inode(&photo)?, fs::symlink_metadata(&photo)?.nlink());
    assert_eq!(link_replaced, [ReplacedOriginal::Deleted]);
    assert_eq!(photo_left, (original, 1));
    assert_ne!(inode(&link)?, original);
    expect_left(&link, false, b"photo", &["Photo.txt", "link.txt"]);
    Ok(())
}
