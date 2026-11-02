// SPDX-License-Identifier: MIT

//! Renames, the names a location refuses, and undoing a rename.

use super::*;

/// An `original` and a `current` folder under a root, with the item's paths in each.
fn rename_folders() -> io::Result<(tempfile::TempDir, PathBuf, PathBuf)> {
    let root = tempfile::tempdir()?;
    let original_parent = root.path().join("original");
    let current_parent = root.path().join("current");
    fs::create_dir_all(&original_parent)?;
    fs::create_dir_all(&current_parent)?;
    Ok((root, original_parent, current_parent))
}

#[test]
fn rename_undo_restores_a_native_name_byte_for_byte() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let (_root, original_parent, current_parent) = rename_folders()?;
    let original_name = OsString::from_vec(b"original-\xff.txt".to_vec());
    let original = original_parent.join(&original_name);
    let current = current_parent.join("renamed.txt");
    fs::write(&current, b"contents")?;

    let events = undo_rename(43, &current, &original);

    assert_renamed(&events);
    expect_gone(&[&current]);
    expect_contents(&[(original.clone(), b"contents")]);
    assert_eq!(
        original_name.as_encoded_bytes(),
        original
            .file_name()
            .expect("original test path should have a file name")
            .as_encoded_bytes()
    );
    Ok(())
}

#[test]
fn rename_undo_refuses_an_occupied_destination_and_can_retry() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let (_root, original_parent, current_parent) = rename_folders()?;
    let original = original_parent.join("item.txt");
    let current = current_parent.join("renamed.txt");
    fs::write(&original, b"occupant")?;
    fs::write(&current, b"renamed")?;

    let events = undo_rename(44, &current, &original);

    assert!(matches!(events.last(), Some(OperationEvent::Failed { .. })));
    expect_contents(&[
        (original.clone(), b"occupant"),
        (current.clone(), b"renamed"),
    ]);

    fs::remove_file(&original)?;
    let events = undo_rename(44, &current, &original);

    assert_renamed(&events);
    expect_gone(&[&current]);
    expect_contents(&[(original, b"renamed")]);
    Ok(())
}

#[test]
fn cancelled_rename_undo_reports_cancellation_without_moving_the_item() -> Result<(), Box<dyn Error>>
{
    let _serial = serial()?;
    let (_root, original_parent, current_parent) = rename_folders()?;
    let original = original_parent.join("item.txt");
    let current = current_parent.join("renamed.txt");
    fs::write(&current, b"renamed")?;
    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let operation = LocalOperationProvider.undo_rename(
        UndoRenameRequest {
            id: OperationRequestId(45),
            current: Location::local(&current),
            original: Location::local(&original),
        },
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    );
    drop(operation);
    drive_until_rename_settles(&events);

    assert!(matches!(
        events.borrow().last(),
        Some(OperationEvent::Cancelled { .. })
    ));
    assert!(current.exists());
    expect_gone(&[&original]);
    Ok(())
}

/// Asserts the rename's only event is its success and the item kept its inode.
fn expect_renamed_keeping(events: &[OperationEvent], path: &Path, original: u64) {
    assert!(
        matches!(events, [OperationEvent::Renamed { .. }]),
        "{events:?}"
    );
    assert_eq!(inode(path), original);
}

#[test]
fn renames_and_their_undo_keep_the_item_where_rename_flags_or_links_are_missing()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let linked_before_a_probe = || Quirks {
        rename_flags: Support::Rejected(Errno::NOSYS),
        ..Quirks::sshfs()
    };
    let ntfs_3g = || Quirks {
        rename_flags: Support::Rejected(Errno::INVAL),
        ..Quirks::ntfs_3g()
    };
    let cut = "x".repeat(240);
    let cases: [(Preset, &str); 5] = [
        (Quirks::without_rename_flags, "renamed"),
        (Quirks::vboxsf, "renamed"),
        (Quirks::sshfs, "renamed"),
        (linked_before_a_probe, "renamed"),
        (ntfs_3g, &cut),
    ];
    for (quirks, name) in cases {
        for folder in [false, true] {
            let root = tempfile::tempdir()?;
            let _scope = FsQuirks::install(root.path(), quirks());
            let source = root.path().join("item");
            let renamed = root.path().join(name);
            if folder {
                fs::create_dir(&source)?;
            } else {
                fs::write(&source, b"data")?;
            }
            let original = inode(&source);

            expect_renamed_keeping(&rename(&source, name), &renamed, original);
            expect_renamed_keeping(&undo_rename(47, &renamed, &source), &source, original);
            expect_names(&[(root.path(), &["item"])])?;
        }
    }
    Ok(())
}

fn cannot_contain(characters: &[&str]) -> String {
    rust_i18n::t!(
        "Names in this location can’t contain %{characters}",
        characters = crate::i18n::list(
            characters
                .iter()
                .map(|character| crate::i18n::quoted(character))
        )
    )
    .into_owned()
}

fn reserved(name: &str) -> String {
    rust_i18n::t!("“%{name}” is a reserved name in this location", name = name).into_owned()
}

/// A name typed for `item` under a mount's `backend` folder, and the refusal it earns.
struct TypedName<'a> {
    quirks: Preset,
    backend: &'a str,
    folder: bool,
    typed: &'a str,
    refusal: Option<String>,
}

impl TypedName<'_> {
    fn check(self) -> Result<(), Box<dyn Error>> {
        let root = tempfile::tempdir()?;
        let mount = root.path().join("mount");
        let parent = mount.join(self.backend);
        let item = parent.join("item");
        fs::create_dir_all(&parent)?;
        let entry = if self.folder {
            fs::create_dir(&item)?;
            directory_entry(&item)
        } else {
            fs::write(&item, b"data")?;
            file_entry(&item)
        };
        let _scope = FsQuirks::install(&mount, (self.quirks)());
        let typed = self.typed;

        let events = settle(|emit| {
            LocalOperationProvider.rename(
                RenameRequest {
                    id: OperationRequestId(48),
                    entry,
                    new_name: typed.to_owned(),
                },
                emit,
            )
        });

        match self.refusal {
            Some(message) => {
                assert_eq!(failure(&events), message, "{typed:?}");
                assert_eq!(names(&parent)?, ["item"], "{typed:?}");
            }
            None => {
                assert!(
                    matches!(events.as_slice(), [OperationEvent::Renamed { .. }]),
                    "{typed:?}: {events:?}"
                );
                assert_eq!(names(&parent)?, [typed], "{typed:?}");
            }
        }
        Ok(())
    }
}

#[test]
fn a_typed_name_the_location_would_store_otherwise_is_refused_before_the_rename()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let gvfs_bridge: Preset = || Quirks {
        mount_type: Some(("fuse.gvfsd-fuse", "rw,user_id=1000,group_id=1000")),
        ..Quirks::default()
    };
    let long_folder_name = "f".repeat(214);
    let rows: [(Preset, &str, bool, &str, Option<String>); 6] = [
        (
            Quirks::vfat,
            "",
            false,
            "a:b?",
            Some(cannot_contain(&[":", "?"])),
        ),
        (
            Quirks::vfat,
            "",
            false,
            "notes.",
            Some(crate::i18n::tr(
                "Names in this location can’t end with a space or a period",
            )),
        ),
        (Quirks::vfat, "", true, "CON", Some(reserved("CON"))),
        // Samba makes only new folders under longer temporary names.
        (Quirks::cifs, "", true, &long_folder_name, None),
        (Quirks::ext4, "", false, "a:b.", None),
        (
            gvfs_bridge,
            "mtp:host=Phone",
            false,
            " x",
            Some(crate::i18n::tr(
                "Names in this location can’t start with a space",
            )),
        ),
    ];
    for (quirks, backend, folder, typed, refusal) in rows {
        TypedName {
            quirks,
            backend,
            folder,
            typed,
            refusal,
        }
        .check()?;
    }
    Ok(())
}

#[test]
fn a_typed_name_at_a_gvfs_address_is_refused_by_the_rules_of_its_scheme()
-> Result<(), Box<dyn Error>> {
    use super::super::super::typed_names::refused_in;
    let _serial = serial()?;
    let colon = rust_i18n::t!(
        "Names in this location can’t contain %{characters}",
        characters = crate::i18n::quoted(":")
    )
    .into_owned();
    let leading_space = crate::i18n::tr("Names in this location can’t start with a space");
    for (uri, typed, refusal) in [
        ("smb://server/share", "a:b", Some(colon)),
        ("mtp://Phone/Internal", " x", Some(leading_space)),
        ("sftp://host/home", "a:b", None),
    ] {
        let folder = gio::File::for_uri(uri);
        let refused = glib::MainContext::default().block_on(refused_in(&folder, typed, false));
        assert_eq!(refused, refusal, "{uri}");
    }
    Ok(())
}

#[test]
fn a_rename_gio_has_no_backend_for_fails_instead_of_ending_the_process()
-> Result<(), Box<dyn Error>> {
    use super::super::super::{NameKept, restore_name};
    let _serial = serial()?;
    let uri = "smb://server/share/item.txt";
    assert_eq!(
        gio::File::for_uri(uri).type_().name(),
        "GDummyFile",
        "needs GIO without GVfs (GIO_USE_VFS=local), whose stand-in file cannot rename"
    );
    let unsupported = crate::services::backend_unavailable_message(uri);
    let entry = FileEntry {
        location: Location::uri(uri),
        ..file_entry(Path::new("/share/item.txt"))
    };

    let events = settle(|emit| {
        LocalOperationProvider.rename(
            RenameRequest {
                id: OperationRequestId(49),
                entry,
                new_name: "renamed.txt".to_owned(),
            },
            emit,
        )
    });
    let given_back = glib::MainContext::default().block_on(restore_name(
        &Location::uri(uri),
        &Location::uri("smb://server/share/item:1.txt"),
    ));

    assert_eq!(failure(&events), unsupported);
    assert!(
        matches!(&given_back, Err(NameKept::Failed(error))
            if *error == crate::services::error_detail(unsupported.clone())),
        "the name was not kept as unsupported"
    );
    Ok(())
}

#[test]
fn a_rename_never_replaces_an_item_created_meanwhile() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    for quirks in [Quirks::default, Quirks::vboxsf] {
        let root = tempfile::tempdir()?;
        let source = root.path().join("item.txt");
        let taken = root.path().join("new.txt");
        fs::write(&source, b"mine")?;
        let raced = taken.clone();
        let _scope = FsQuirks::install(
            root.path(),
            Quirks {
                before: vec![(
                    Op::Rename,
                    "new.txt",
                    0,
                    Box::new(move || fs::write(&raced, b"theirs").expect("racer")),
                )],
                ..quirks()
            },
        );

        assert_eq!(
            failure(&rename(&source, "new.txt")),
            "\u{201c}new.txt\u{201d} already exists"
        );
        expect_contents(&[(source, b"mine"), (taken, b"theirs")]);
    }
    Ok(())
}

#[test]
fn a_case_only_rename_and_its_undo_keep_the_item_or_say_where_it_was_left()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("report.txt");
    let renamed = root.path().join("Report.txt");
    fs::write(&source, b"data")?;
    let original = inode(&source);
    {
        let _scope = FsQuirks::install(
            root.path(),
            Quirks {
                case_insensitive: true,
                ..Quirks::default()
            },
        );
        expect_renamed_keeping(&rename(&source, "Report.txt"), &renamed, original);
        expect_renamed_keeping(&undo_rename(47, &renamed, &source), &source, original);
    }

    let _scope = FsQuirks::install(
        root.path(),
        Quirks {
            case_insensitive: true,
            // Skips the direct attempts, then fails the second step and the way back.
            fail: vec![
                (Op::Rename, "Report.txt", 1, Errno::IO),
                (Op::Rename, "report.txt", 1, Errno::IO),
            ],
            ..Quirks::default()
        },
    );
    let message = failure(&rename(&source, "Report.txt"));

    let left = names(root.path())?;
    let [originals] = left.as_slice() else {
        panic!("expected one entry, got {left:?}");
    };
    assert!(originals.starts_with(".strata-originals-"), "{originals}");
    assert_eq!(
        inode(&root.path().join(originals).join("report.txt")),
        original
    );
    assert_eq!(
        message,
        format!(
            "Could not rename \u{201c}report.txt\u{201d}; it was left as \u{201c}{originals}/report.txt\u{201d}: A device input/output error occurred"
        )
    );
    Ok(())
}
