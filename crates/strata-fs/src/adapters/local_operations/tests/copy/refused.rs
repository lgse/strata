// SPDX-License-Identifier: MIT

use super::*;

use crate::services::RenameCause;

type Refusal = fn(&OsStr) -> Option<Errno>;

fn names_a_colon(name: &OsStr) -> bool {
    name.as_encoded_bytes().contains(&b':')
}

fn refuses_colons_as_bad_messages(name: &OsStr) -> Option<Errno> {
    names_a_colon(name).then_some(Errno::BADMSG)
}

fn refuses_colons_as_missing(name: &OsStr) -> Option<Errno> {
    names_a_colon(name).then_some(Errno::NOENT)
}

/// The names `scope` was asked for that `refuses` refuses, in call order.
fn refused_names(scope: &FsQuirks, refuses: Refusal) -> Vec<String> {
    scope
        .calls()
        .iter()
        .flat_map(|call| call.names.clone())
        .filter(|name| refuses(name).is_some())
        .map(|name| name.to_string_lossy().into_owned())
        .collect()
}

/// Each `(path, contents)` beneath `root` reads as its contents.
fn assert_paths(root: &Path, files: &[(PathBuf, &str)], label: &str) -> io::Result<()> {
    for (name, contents) in files {
        let read = fs::read(root.join(name))?;
        assert_eq!(read, contents.as_bytes(), "{label}: {}", name.display());
    }
    Ok(())
}

struct Refusing {
    label: &'static str,
    quirks: Preset,
    /// A refused name, and a sibling that has the name stricter rules give it.
    first: [&'static str; 2],
    /// A refused folder.
    second: &'static str,
    /// A refused item at the top, a folder or a file, and an item already
    /// there with the name stricter rules give it.
    top: (&'static str, bool, &'static str),
    /// Pasted later: a refused name and one ending in a dot in a folder,
    /// and a refused name at the top.
    later: [&'static str; 3],
    /// What `first`, `second`, `top` and `later` are saved as.
    saved: [&'static str; 6],
    /// The names the mount is asked for and refuses, in each paste.
    refused: [&'static [&'static str]; 2],
}

#[test]
fn a_refused_name_takes_the_first_stricter_rules_it_passes_and_teaches_only_a_mount_that_learns()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let unknown_daemon = || Quirks {
        mount_type: Some(("fuse.strictfs", "rw,user_id=0,group_id=0")),
        invalid_name: Some(refuses_colons_as_bad_messages),
        ..Quirks::default()
    };
    let ntfs_3g_without_udev = || Quirks {
        device_format: None,
        ..Quirks::ntfs_3g()
    };
    // A mount whose rules are known never learns from what it refuses.
    let strict_ext4 = || Quirks {
        invalid_name: Some(has_colon),
        ..Quirks::ext4()
    };
    // Each mount places the top item its own way: a staged folder, a
    // staged file, and a file linked from an unnamed one.
    let rows = [
        Refusing {
            label: "unknown FUSE daemon",
            quirks: unknown_daemon,
            first: ["a:b", "a_b"],
            second: "c:d",
            top: ("x:y", true, "x_y"),
            later: ["e:f", "dot.", "g:h"],
            saved: ["a_b (1)", "c_d", "x_y (1)", "e_f", "dot.", "g_h"],
            refused: [&["a:b", "x:y"], &[]],
        },
        // The top name is fixed for Windows first, and then refused.
        Refusing {
            label: "ntfs-3g that udev cannot name",
            quirks: ntfs_3g_without_udev,
            first: ["aux.c", "aux_.c"],
            second: "con.h",
            top: ("prn.txt?", false, "prn_.txt_"),
            later: ["com1.log", "dot.", "lpt1.ini"],
            saved: [
                "aux_ (1).c",
                "con_.h",
                "prn_ (1).txt_",
                "com1_.log",
                "dot",
                "lpt1_.ini",
            ],
            refused: [&["aux.c", "prn.txt_"], &[]],
        },
        Refusing {
            label: "ext4",
            quirks: strict_ext4,
            first: ["a:b", "a_b"],
            second: "c:d",
            top: ("x:y", false, "x_y"),
            later: ["e:f", "dot.", "g:h"],
            saved: ["a_b (1)", "c_d", "x_y (1)", "e_f", "dot.", "g_h"],
            refused: [&["a:b", "c:d", "x:y"], &["e:f", "g:h"]],
        },
    ];
    for row in &rows {
        check_stricter_rules(row)?;
    }
    Ok(())
}

impl Refusing {
    /// The top item's file, beneath itself when it is a folder.
    fn top_file(&self, top: &str) -> PathBuf {
        let (_, top_folder, _) = self.top;
        if top_folder {
            Path::new(top).join("top")
        } else {
            PathBuf::from(top)
        }
    }

    /// Lays out the `one`, `two` and `three` folders and the top items
    /// beneath `source`, and what `disk` already holds.
    fn create(&self, source: &Path, disk: &Path) -> io::Result<()> {
        let [first, kept] = self.first;
        let (top, top_folder, existing) = self.top;
        let [later, dotted, later_top] = self.later;
        for folder in [
            Path::new("one"),
            &Path::new("two").join(self.second),
            Path::new("three"),
        ] {
            fs::create_dir_all(source.join(folder))?;
        }
        if top_folder {
            fs::create_dir(source.join(top))?;
        }
        fs::create_dir(disk)?;
        fs::write(disk.join(existing), "earlier")?;
        for (path, contents) in [
            (Path::new("one").join(first), "first"),
            (Path::new("one").join(kept), "kept"),
            (Path::new("two").join(self.second).join("inside"), "second"),
            (self.top_file(top), "top"),
            (Path::new("three").join(later), "later"),
            (Path::new("three").join(dotted), "dotted"),
            (PathBuf::from(later_top), "later top"),
        ] {
            fs::write(source.join(path), contents)?;
        }
        Ok(())
    }

    fn renames(&self) -> Vec<(String, String, RenameCause)> {
        let invalid = RenameCause::Invalid;
        let [first, _] = self.first;
        let (top, _, _) = self.top;
        let [first_saved, second_saved, top_saved, ..] = self.saved;
        let second = self.second;
        let mut renamed = vec![
            (
                format!("one/{first}"),
                format!("one/{first_saved}"),
                invalid,
            ),
            (
                format!("two/{second}"),
                format!("two/{second_saved}"),
                invalid,
            ),
            (top.to_owned(), top_saved.to_owned(), invalid),
        ];
        renamed.sort_by(|left, right| left.0.cmp(&right.0));
        renamed
    }

    fn first_saved(&self) -> [(PathBuf, &'static str); 5] {
        let [_, kept] = self.first;
        let (_, _, existing) = self.top;
        let [first_saved, second_saved, top_saved, ..] = self.saved;
        [
            (Path::new("one").join(kept), "kept"),
            (Path::new("one").join(first_saved), "first"),
            (Path::new("two").join(second_saved).join("inside"), "second"),
            (self.top_file(top_saved), "top"),
            (PathBuf::from(existing), "earlier"),
        ]
    }

    fn later_saved(&self) -> [(PathBuf, &'static str); 3] {
        let [.., later_saved, dotted_saved, later_top_saved] = self.saved;
        [
            (Path::new("three").join(later_saved), "later"),
            (Path::new("three").join(dotted_saved), "dotted"),
            (PathBuf::from(later_top_saved), "later top"),
        ]
    }
}

fn check_stricter_rules(row: &Refusing) -> Result<(), Box<dyn Error>> {
    let label = row.label;
    let (top, _, _) = row.top;
    let [_, _, later_top] = row.later;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    let disk = root.path().join("disk");
    row.create(&source, &disk)?;
    let quirks = (row.quirks)();
    let refuses = quirks.invalid_name.ok_or("no refusal")?;
    let scope = FsQuirks::install(&disk, quirks);

    let events = paste_into(
        &disk,
        &[source.join("one"), source.join("two"), source.join(top)],
    );

    assert_eq!(
        renamed_beneath(&disk, &followed_report(&events)),
        row.renames(),
        "{label}"
    );
    assert_paths(&disk, &row.first_saved(), label)?;
    // Names planned before the mount learned are refused once each.
    assert_eq!(refused_names(&scope, refuses), row.refused[0], "{label}");
    scope.clear_calls();

    paste_into(&disk, &[source.join("three"), source.join(later_top)]);

    assert_paths(&disk, &row.later_saved(), label)?;
    assert_eq!(refused_names(&scope, refuses), row.refused[1], "{label}");
    Ok(())
}

fn refuses_colons_and_aux(name: &OsStr) -> Option<Errno> {
    let name = name.as_encoded_bytes();
    (name.contains(&b':') || name.to_ascii_lowercase().starts_with(b"aux.")).then_some(Errno::INVAL)
}

fn refuses_trailing_dots(name: &OsStr) -> Option<Errno> {
    name.as_encoded_bytes()
        .ends_with(b".")
        .then_some(Errno::INVAL)
}

#[test]
fn a_name_refused_for_its_trailing_dot_takes_the_name_without_it_unless_a_sibling_has_it()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    for (siblings, saved) in [(&["x."][..], &["x"][..]), (&["x.", "x"], &["x", "x (1)"])] {
        let root = tempfile::tempdir()?;
        let source = root.path().join("dots");
        let disk = root.path().join("disk");
        fs::create_dir_all(&source)?;
        fs::create_dir(&disk)?;
        for name in siblings {
            fs::write(source.join(name), name)?;
        }
        let _disk = FsQuirks::install(
            &disk,
            Quirks {
                invalid_name: Some(refuses_trailing_dots),
                ..Quirks::ext4()
            },
        );

        paste_into(&disk, std::slice::from_ref(&source));

        assert_eq!(names(&disk.join("dots"))?, saved, "{siblings:?}");
        assert_eq!(
            fs::read(disk.join("dots").join(saved.last().ok_or("none")?))?,
            b"x."
        );
    }
    Ok(())
}

#[test]
fn a_mount_that_learns_keeps_only_the_rules_its_refused_names_needed() -> Result<(), Box<dyn Error>>
{
    let _serial = serial()?;
    let rows: [Learned<'_>; 2] = [
        // Neither rule alone gives a name the mount takes.
        (
            refuses_colons_and_aux,
            "aux.c:",
            "aux_.c_",
            &["aux.c:", "aux.c_", "aux_.c:"],
            ["b:c", "aux.txt", "x."],
            ["aux_.txt", "b_c", "x."],
            true,
        ),
        (
            refuses_trailing_dots,
            "x.",
            "x",
            &["x."],
            ["a:b", "aux.txt", "y."],
            ["a:b", "aux.txt", "y"],
            false,
        ),
    ];
    for row in rows {
        check_learned_rules(row)?;
    }
    Ok(())
}

/// What the mount refuses, a first name it refuses and what that is saved
/// as, the names asked for and refused, three later names and what they are
/// saved as, and whether a colon typed in a rename is then refused.
type Learned<'a> = (
    Refusal,
    &'a str,
    &'a str,
    &'a [&'a str],
    [&'a str; 3],
    [&'a str; 3],
    bool,
);

fn check_learned_rules(
    (refuses, first, saved, refused, later, later_saved, typed_colon_refused): Learned<'_>,
) -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    let disk = root.path().join("disk");
    for folder in ["one", "two"] {
        fs::create_dir_all(source.join(folder))?;
    }
    fs::create_dir(&disk)?;
    fs::write(source.join("one").join(first), first)?;
    for name in later {
        fs::write(source.join("two").join(name), name)?;
    }
    let scope = FsQuirks::install(
        &disk,
        Quirks {
            mount_type: Some(("fuse.strictfs", "rw,user_id=0,group_id=0")),
            invalid_name: Some(refuses),
            ..Quirks::default()
        },
    );

    paste_into(&disk, &[source.join("one")]);

    assert_eq!(fs::read(disk.join("one").join(saved))?, first.as_bytes());
    assert_eq!(refused_names(&scope, refuses), refused, "{first}");
    scope.clear_calls();

    paste_into(&disk, &[source.join("two")]);

    assert_eq!(names(&disk.join("two"))?, later_saved, "{first}");
    assert_eq!(refused_names(&scope, refuses), [] as [&str; 0], "{first}");

    let typed = rename(&disk.join("two").join(later_saved[1]), "c:d");

    assert_typed_colon(&typed, typed_colon_refused, first);
    Ok(())
}

fn assert_typed_colon(typed: &[OperationEvent], refused: bool, first: &str) {
    if !refused {
        assert!(
            matches!(typed, [OperationEvent::Renamed { .. }]),
            "{first}: {typed:?}"
        );
        return;
    }
    let refusal = rust_i18n::t!(
        "Names in this location can’t contain %{characters}",
        characters = crate::i18n::quoted(":")
    );
    assert!(
        matches!(typed, [OperationEvent::Failed { message, .. }] if *message == refusal),
        "{first}: {typed:?}"
    );
}

#[test]
fn a_kept_copy_the_mount_refused_is_reported_with_the_name_it_took() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    let disk = root.path().join("disk");
    fs::create_dir_all(&source)?;
    fs::create_dir(&disk)?;
    fs::write(source.join("a:b"), b"incoming")?;
    // As the server made it.
    fs::write(disk.join("a:b"), b"earlier")?;
    let _disk = FsQuirks::install(
        &disk,
        Quirks {
            mount_type: Some(("fuse.strictfs", "rw,user_id=0,group_id=0")),
            invalid_name: Some(refuses_colons_as_bad_messages),
            ..Quirks::default()
        },
    );

    let events = run_paste(PasteRequest {
        id: OperationRequestId(1590),
        destination: Location::local(&disk),
        items: vec![PasteItem {
            source: Location::local(source.join("a:b")),
            conflict: TransferConflict::KeepBoth,
            target_name: None,
        }],
        move_sources: false,
    });

    assert_eq!(fs::read(disk.join("a_b (1)"))?, b"incoming");
    assert_eq!(
        renamed_beneath(&disk, &followed_report(&events)),
        [("a:b".to_owned(), "a_b (1)".to_owned(), RenameCause::Invalid)]
    );
    Ok(())
}

#[test]
fn a_bulk_copy_records_for_undo_where_a_refused_name_was_saved() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    let disk = root.path().join("disk");
    fs::create_dir_all(&source)?;
    fs::create_dir(&disk)?;
    // More than eight files are copied in parallel.
    let mut items = vec![source.join("a:b")];
    items.extend((0..8).map(|index| source.join(format!("file {index}"))));
    for item in &items {
        fs::write(item, "bulk")?;
    }
    let _disk = FsQuirks::install(
        &disk,
        Quirks {
            invalid_name: Some(has_colon),
            ..Quirks::ext4()
        },
    );

    let created = run_paste_collecting_created(PasteRequest {
        id: OperationRequestId(1590),
        destination: Location::local(&disk),
        items: items
            .iter()
            .map(|source| PasteItem {
                source: Location::local(source),
                conflict: TransferConflict::FailIfExists,
                target_name: None,
            })
            .collect(),
        move_sources: false,
    })?;

    let mut created = created
        .into_iter()
        .flatten()
        .map(|location| location.native_path().map(Path::to_path_buf))
        .collect::<Option<Vec<_>>>()
        .ok_or("not native")?;
    created.sort();
    let mut expected = names(&disk)?
        .into_iter()
        .map(|name| disk.join(name))
        .collect::<Vec<_>>();
    expected.sort();
    assert!(expected.contains(&disk.join("a_b")), "{expected:?}");
    assert_eq!(created, expected);
    Ok(())
}

#[test]
fn a_refused_name_keeps_unnamed_files_on_and_writes_twice_only_after_enoent()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    if !base_offers(&UNNAMED_FILES)? {
        return Ok(());
    }
    // Names given back are refused as they are.
    for (case, policy, refused, outcome, staged) in [
        (
            "fixed after ENOENT",
            fixing(),
            refuses_colons_as_missing as Refusal,
            Ok("a_b"),
            false,
        ),
        (
            "given back after ENOENT",
            NamePolicy::default(),
            refuses_colons_as_missing,
            Err(None),
            true,
        ),
        (
            "given back after EINVAL",
            NamePolicy::default(),
            has_colon,
            Err(Some(crate::services::INVALID_NAME_REASON)),
            false,
        ),
    ] {
        check_unnamed_files_kept_on(case, policy, refused, (outcome, staged))?;
    }
    Ok(())
}

/// Copied as fixed, or refused for a reason it names or keeps to itself.
type Outcome = Result<&'static str, Option<&'static str>>;

fn assert_copy_outcome(
    copied: Result<gio::File, glib::Error>,
    outcome: Outcome,
    disk: &Path,
    case: &str,
) -> Result<(), Box<dyn Error>> {
    if let Ok(saved) = outcome {
        assert_eq!(copied?.path(), Some(disk.join(saved)), "{case}");
        assert_eq!(fs::read(disk.join(saved))?, b"colon", "{case}");
        return Ok(());
    }
    let error = copied.err().ok_or("a refused name was copied")?;
    if let Err(Some(reason)) = outcome {
        assert!(
            error.message().contains(&crate::i18n::tr(reason)),
            "{case}: {error}"
        );
    }
    assert!(names(disk)?.is_empty(), "{case}");
    Ok(())
}

fn check_unnamed_files_kept_on(
    case: &str,
    policy: NamePolicy,
    refused: Refusal,
    (outcome, staged): (Outcome, bool),
) -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    let disk = root.path().join("disk");
    fs::create_dir_all(&source)?;
    fs::create_dir(&disk)?;
    fs::write(source.join("a:b"), b"colon")?;
    fs::write(source.join("plain"), b"plain")?;
    let scope = FsQuirks::install(
        &disk,
        Quirks {
            invalid_name: Some(refused),
            ..Quirks::ext4()
        },
    );
    let copy = |name: &str| {
        glib::MainContext::default().block_on(copy_new_recursively_on_filesystem(
            gio::File::for_path(source.join(name)),
            gio::File::for_path(disk.join(name)),
            Taken::Fail,
            gio::Cancellable::new(),
            None,
            policy.clone(),
            None,
        ))
    };

    let copied = copy("a:b");

    assert_copy_outcome(copied, outcome, &disk, case)?;
    assert_eq!(
        scope.count(Op::OpenTmpfile),
        1,
        "{case}: {:?}",
        scope.calls()
    );
    assert_eq!(
        scope.used_name(".strata-copy-*"),
        staged,
        "{case}: {:?}",
        scope.calls()
    );
    scope.clear_calls();

    copy("plain")?;

    assert_eq!(fs::read(disk.join("plain"))?, b"plain", "{case}");
    assert_eq!(
        scope.count(Op::OpenTmpfile),
        1,
        "{case}: {:?}",
        scope.calls()
    );
    Ok(())
}

fn refused_as_invalid(name: &OsStr) -> Option<Errno> {
    (name == "refused").then_some(Errno::INVAL)
}

fn refused_as_illegal(name: &OsStr) -> Option<Errno> {
    (name == "refused").then_some(Errno::ILSEQ)
}

fn refused_as_bad_message(name: &OsStr) -> Option<Errno> {
    (name == "refused").then_some(Errno::BADMSG)
}

fn refused_as_missing(name: &OsStr) -> Option<Errno> {
    (name == "refused").then_some(Errno::NOENT)
}

fn refused_as_too_long(name: &OsStr) -> Option<Errno> {
    (name == "refused").then_some(Errno::NAMETOOLONG)
}

#[test]
fn a_name_no_stricter_rules_change_fails_naming_the_item_and_the_reason()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let invalid = crate::services::INVALID_NAME_REASON;
    let too_long = "The name is too long";
    let rows: [(Refusal, bool, &str); 7] = [
        (refused_as_invalid, true, invalid),
        (refused_as_illegal, true, invalid),
        (refused_as_bad_message, true, invalid),
        (refused_as_missing, true, invalid),
        (refused_as_too_long, true, too_long),
        (refused_as_invalid, false, invalid),
        (refused_as_bad_message, false, invalid),
    ];
    for (refusal, nested, reason) in rows {
        let root = tempfile::tempdir()?;
        let source = root.path().join("source");
        let disk = root.path().join("disk");
        fs::create_dir_all(&source)?;
        fs::create_dir(&disk)?;
        fs::write(source.join("refused"), b"refused")?;
        let _disk = FsQuirks::install(
            &disk,
            Quirks {
                invalid_name: Some(refusal),
                ..Quirks::default()
            },
        );
        let item = if nested {
            source.clone()
        } else {
            source.join("refused")
        };
        let target = disk.join(item.file_name().ok_or("no name")?);

        let error = glib::MainContext::default()
            .block_on(copy_new_recursively_on_filesystem(
                gio::File::for_path(&item),
                gio::File::for_path(&target),
                Taken::Fail,
                gio::Cancellable::new(),
                None,
                fixing(),
                None,
            ))
            .expect_err("refused");

        assert_eq!(
            error.message(),
            rust_i18n::t!(
                "Could not copy %{name}: %{error}",
                name = "refused",
                error = crate::services::error_detail(crate::i18n::tr(reason))
            ),
            "{reason}, nested: {nested}"
        );
    }
    Ok(())
}
