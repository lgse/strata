// SPDX-License-Identifier: MIT

use super::super::super::{
    CopyInto, CopyRecord, GioNames, NamedChildren, Planned, copy_recursively_with_progress,
    mount_profile::FolderNames,
};
use super::*;
use crate::adapters::volume::{NameEncoding, NameProfile, Reserved};
use crate::services::RenameCause::{self, Alias, Invalid, TooLong};

type Renamed = (String, String, RenameCause);

fn owned(renames: &[(&str, &str, RenameCause)]) -> Vec<Renamed> {
    renames
        .iter()
        .map(|(name, saved_as, cause)| ((*name).to_owned(), (*saved_as).to_owned(), *cause))
        .collect()
}

fn renamed(folder: &Path, events: &[OperationEvent]) -> Vec<Renamed> {
    renamed_beneath(folder, &followed_report(events))
}

/// Two folders whose names clash once FAT fixes them, a FAT `stick` that
/// already holds device names, and more than eight files for a bulk copy.
struct FatPaste {
    one: PathBuf,
    two: PathBuf,
    stick: PathBuf,
    earlier: PathBuf,
    bulk_items: Vec<PathBuf>,
}

impl FatPaste {
    fn create(root: &Path) -> io::Result<Self> {
        let (one, two, stick) = (root.join("one"), root.join("two"), root.join("stick"));
        let earlier = stick.join("earlier");
        for folder in [&one, &two, &earlier.join("kept")] {
            fs::create_dir_all(folder)?;
        }
        for (path, contents) in [
            (one.join("a:b"), "colon"),
            (two.join("a_b"), "kept"),
            (one.join("x"), "lower"),
            (two.join("X"), "upper"),
            (earlier.join("CON"), "device"),
            (earlier.join("kept/CON"), "nested device"),
        ] {
            fs::write(path, contents)?;
        }
        // More than eight files are copied in parallel.
        let bulk = root.join("bulk");
        fs::create_dir_all(stick.join("bulk"))?;
        let mut bulk_items = vec![bulk.join("b?c")];
        bulk_items.extend((0..8).map(|index| bulk.join(format!("file {index}"))));
        fs::create_dir(&bulk)?;
        for item in &bulk_items {
            fs::write(item, "bulk")?;
        }
        Ok(Self {
            one,
            two,
            stick,
            earlier,
            bulk_items,
        })
    }

    fn selected(&self) -> [PathBuf; 6] {
        [
            self.one.join("a:b"),
            self.two.join("a_b"),
            self.one.join("x"),
            self.two.join("X"),
            self.earlier.join("CON"),
            self.earlier.join("kept"),
        ]
    }
}

#[test]
fn a_paste_fixes_the_names_it_brings_onto_fat_and_keeps_those_already_there()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let fixture = FatPaste::create(root.path())?;
    let stick = &fixture.stick;
    let _stick = FsQuirks::install(stick, Quirks::vfat());

    let events = paste_into(stick, &fixture.selected());
    let bulk_events = paste_into(&stick.join("bulk"), &fixture.bulk_items);

    assert_files(
        stick,
        &[
            ("a_b (1)", "colon"),
            ("a_b", "kept"),
            ("x (1)", "lower"),
            ("X", "upper"),
            ("CON", "device"),
            ("kept/CON", "nested device"),
            ("bulk/b_c", "bulk"),
        ],
    )?;
    assert_eq!(names(stick)?.len(), 8);
    assert_eq!(
        renamed(stick, &events),
        owned(&[("a:b", "a_b (1)", Invalid), ("x", "x (1)", Alias)])
    );
    assert_eq!(
        renamed(&stick.join("bulk"), &bulk_events),
        owned(&[("b?c", "b_c", Invalid)])
    );
    Ok(())
}

#[test]
fn a_fat_copy_fixes_an_invalid_name_instead_of_discarding_the_whole_tree()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    let stick = root.path().join("stick");
    let target = stick.join("target");
    fs::create_dir_all(source.join("nested"))?;
    fs::create_dir(&stick)?;
    fs::write(source.join("ok.txt"), b"fine")?;
    fs::write(
        source.join("nested/sc_macroorganizer?tabid:short=1.png"),
        b"cached icon",
    )?;
    let _stick = FsQuirks::install(&stick, Quirks::vfat());

    let result = glib::MainContext::default().block_on(copy_recursively_fixing(
        gio::File::for_path(&source),
        gio::File::for_path(&target),
        false,
        gio::Cancellable::new(),
    ));

    assert!(result.is_ok(), "{result:?}");
    assert_files(
        &target,
        &[
            ("ok.txt", "fine"),
            ("nested/sc_macroorganizer_tabid_short=1.png", "cached icon"),
        ],
    )?;
    assert_eq!(names(&target.join("nested"))?.len(), 1);
    Ok(())
}

type Batches = Vec<Vec<(OsString, gio::FileType, Planned)>>;

/// Lists `folder` in the batches a GIO copy plans its names in.
fn planned_batches(folder: &Path, rules: NameProfile) -> Result<Batches, glib::Error> {
    glib::MainContext::default().block_on(async {
        let source = gio::File::for_path(folder);
        let cancellable = gio::Cancellable::new();
        let enumerator = await_cancellable(&source, &cancellable, |source, cancellable, result| {
            source.enumerate_children_async(
                "standard::name,standard::type",
                gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
                glib::Priority::DEFAULT,
                Some(cancellable),
                move |output| result.resolve(output),
            );
        })
        .await?;
        let names = FolderNames {
            rules,
            fixing: true,
            links: true,
        };
        let mut listed = NamedChildren::new(&enumerator, names);
        let mut batches = Vec::new();
        loop {
            let batch = listed.next(&cancellable).await?;
            if batch.is_empty() {
                return Ok(batches);
            }
            batches.push(batch);
        }
    })
}

fn check_listing_batches(
    folder: &Path,
    rules: NameProfile,
    listed_first: bool,
) -> Result<(), Box<dyn Error>> {
    let batches = planned_batches(folder, rules)?;

    let fixed = batches
        .iter()
        .flatten()
        .filter(|(_, _, planned)| planned.cause.is_some())
        .map(|(name, _, planned)| (name.clone(), planned.target.clone(), planned.cause))
        .collect::<Vec<_>>();
    assert_eq!(
        fixed,
        [("a:b".into(), "a_b (1)".into(), Some(Invalid))],
        "{rules:?}"
    );
    assert_eq!(batches.iter().map(Vec::len).sum::<usize>(), 102);
    if listed_first {
        assert_eq!(batches.len(), 1, "{rules:?}");
    } else {
        assert!(batches[0].len() <= 64, "{rules:?}");
        assert_eq!(batches.last().map(Vec::len), Some(1), "{rules:?}");
    }
    Ok(())
}

#[test]
fn a_gio_copy_lists_a_whole_folder_first_only_where_the_mount_takes_two_names_for_one()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let folder = root.path().join("folder");
    fs::create_dir(&folder)?;
    for name in ["a:b", "a_b"] {
        fs::write(folder.join(name), name)?;
    }
    // More than one batch of 64 children.
    for index in 0..100 {
        fs::write(folder.join(format!("pad {index:03}")), b"")?;
    }
    let ntfs3 = NameProfile::windows(Reserved::Bare, NameEncoding::Utf8);
    let fat = NameProfile {
        folds: true,
        ..ntfs3
    };
    for (rules, listed_first) in [(ntfs3, false), (fat, true)] {
        check_listing_batches(&folder, rules, listed_first)?;
    }
    Ok(())
}

fn gio_copy(
    source: gio::File,
    target: gio::File,
    cancellable: gio::Cancellable,
    progress: Rc<TransferProgressTracker>,
) -> Pin<Box<dyn Future<Output = Result<(), glib::Error>>>> {
    copy_recursively_with_progress(
        source,
        target,
        CopyInto::New,
        cancellable,
        CopyRecord::Nothing,
        Some(progress),
        GioNames::new(fixing(), Default::default()),
    )
}

fn engine_copy(
    source: gio::File,
    target: gio::File,
    cancellable: gio::Cancellable,
    progress: Rc<TransferProgressTracker>,
) -> Pin<Box<dyn Future<Output = Result<(), glib::Error>>>> {
    Box::pin(async move {
        copy_new_recursively_on_filesystem(
            source,
            target,
            Taken::Fail,
            cancellable,
            Some(progress),
            fixing(),
            None,
        )
        .await
        .map(drop)
    })
}

type CopyTree = fn(
    gio::File,
    gio::File,
    gio::Cancellable,
    Rc<TransferProgressTracker>,
) -> Pin<Box<dyn Future<Output = Result<(), glib::Error>>>>;

const COPIES: [(&str, CopyTree); 2] = [("engine", engine_copy), ("GIO", gio_copy)];

/// Copies `source` to `target` through `copy`, returning the renames it
/// reported.
fn copy_tree(
    copy: CopyTree,
    source: &Path,
    target: &Path,
) -> (Result<(), glib::Error>, Vec<Renamed>) {
    let (progress, events) = recording_progress();
    let result = glib::MainContext::default().block_on(copy(
        gio::File::for_path(source),
        gio::File::for_path(target),
        gio::Cancellable::new(),
        progress,
    ));
    (result, renamed(target, &events.borrow()))
}

#[test]
fn a_gio_copy_that_fails_reports_the_fixed_name_of_a_folder_it_made() -> Result<(), Box<dyn Error>>
{
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    let destination = root.path().join("destination");
    let target = destination.join("target");
    fs::create_dir_all(source.join("x:y"))?;
    fs::create_dir(&destination)?;
    // GIO copies no named pipe.
    make_fifo(&source.join("x:y/pipe"), 0o644)?;
    let _destination = FsQuirks::install(&destination, Quirks::vfat());

    let (result, renames) = copy_tree(gio_copy, &source, &target);

    assert!(result.is_err());
    assert_eq!(names(&target)?, ["x_y"]);
    assert_eq!(renames, owned(&[("x:y", "x_y", Invalid)]));
    Ok(())
}

/// Names FAT must fix or tell apart, among more siblings than GIO lists
/// in one batch.
fn fat_family_source(source: &Path) -> io::Result<()> {
    fs::create_dir_all(source)?;
    fs::write(source.join("a?.txt"), b"question")?;
    // Enough siblings that GIO lists them in several batches.
    for index in 0..130 {
        fs::write(source.join(format!("pad {index:03}")), b"")?;
    }
    for (name, contents) in [
        ("a:.txt", "colon"),
        ("A*.txt", "star"),
        ("a_.txt", "kept"),
        ("README.txt", "upper"),
        ("Readme.txt", "mixed"),
        ("CON", "device"),
    ] {
        fs::write(source.join(name), contents)?;
    }
    for folder in ["Photos.2024", "photos.2024"] {
        fs::create_dir(source.join(folder))?;
        fs::write(source.join(folder).join("photo"), folder)?;
    }
    Ok(())
}

fn check_fat_family(row: &str, copy: CopyTree) -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    let destination = root.path().join("destination");
    let target = destination.join("target");
    fat_family_source(&source)?;
    fs::create_dir(&destination)?;
    let _destination = FsQuirks::install(&destination, Quirks::vfat());

    let (result, renames) = copy_tree(copy, &source, &target);

    assert!(result.is_ok(), "{row}: {result:?}");
    let expected = owned(&[
        ("A*.txt", "A_ (1).txt", Invalid),
        ("CON", "CON_", Invalid),
        ("Readme.txt", "Readme (1).txt", Alias),
        ("a:.txt", "a_ (2).txt", Invalid),
        ("a?.txt", "a_ (3).txt", Invalid),
        ("photos.2024", "photos.2024 (1)", Alias),
    ]);
    assert_eq!(renames, expected, "{row}");
    assert_files(
        &target,
        &[
            ("a_.txt", "kept"),
            ("A_ (1).txt", "star"),
            ("a_ (2).txt", "colon"),
            ("a_ (3).txt", "question"),
            ("README.txt", "upper"),
            ("Readme (1).txt", "mixed"),
            ("CON_", "device"),
            ("Photos.2024/photo", "Photos.2024"),
            ("photos.2024 (1)/photo", "photos.2024"),
        ],
    )?;
    assert_eq!(names(&target)?.len(), 139, "{row}");
    Ok(())
}

#[test]
fn fat_family_copies_number_fixed_names_past_the_names_their_siblings_keep()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    for (row, copy) in COPIES {
        check_fat_family(row, copy)?;
    }
    Ok(())
}

#[test]
fn a_copy_into_a_case_sensitive_folder_keeps_a_cut_name_apart_by_case_only()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    let disk = root.path().join("disk");
    let target = disk.join("target");
    fs::create_dir_all(&source)?;
    fs::create_dir(&disk)?;
    let (long, short, cut) = (
        format!("A{}", "b".repeat(70)),
        format!("a{}", "b".repeat(63)),
        format!("A{}", "b".repeat(63)),
    );
    for (name, contents) in [
        (long.as_str(), "long"),
        (short.as_str(), "short"),
        ("README", "upper"),
        ("Readme", "mixed"),
    ] {
        fs::write(source.join(name), contents)?;
    }
    let _disk = FsQuirks::install(
        &disk,
        Quirks {
            name_max: Some(64),
            ..Quirks::ext4()
        },
    );
    let (progress, events) = recording_progress();

    glib::MainContext::default().block_on(copy_new_recursively_on_filesystem(
        gio::File::for_path(&source),
        gio::File::for_path(&target),
        Taken::Fail,
        gio::Cancellable::new(),
        Some(progress),
        fixing(),
        None,
    ))?;

    assert_eq!(
        renamed(&target, &events.borrow()),
        [(long.clone(), cut.clone(), TooLong)]
    );
    assert_files(
        &target,
        &[
            (cut.as_str(), "long"),
            (short.as_str(), "short"),
            ("README", "upper"),
            ("Readme", "mixed"),
        ],
    )?;
    assert_eq!(names(&target)?.len(), 4);
    Ok(())
}

/// A mount with Windows rules, the name copied there as a file or a
/// folder, and what it is saved as and why.
struct WindowsName<'a> {
    preset: &'static str,
    quirks: Preset,
    name: &'a str,
    is_folder: bool,
    saved: &'a str,
    cause: Option<RenameCause>,
}

const fn file_named<'a>(
    (preset, quirks): (&'static str, Preset),
    name: &'a str,
    saved: &'a str,
    cause: Option<RenameCause>,
) -> WindowsName<'a> {
    WindowsName {
        preset,
        quirks,
        name,
        is_folder: false,
        saved,
        cause,
    }
}

fn check_windows_name(
    copy_label: &str,
    copy: CopyTree,
    case: &WindowsName<'_>,
) -> Result<(), Box<dyn Error>> {
    let WindowsName {
        preset,
        quirks,
        name,
        is_folder,
        saved,
        cause,
    } = *case;
    let row = format!("{copy_label}, {preset}, {name}");
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    let disk = root.path().join("disk");
    let target = disk.join("target");
    fs::create_dir_all(&source)?;
    fs::create_dir(&disk)?;
    if is_folder {
        fs::create_dir(source.join(name))?;
        fs::write(source.join(name).join("inside"), name)?;
    } else {
        fs::write(source.join(name), name)?;
    }
    let _disk = FsQuirks::install(&disk, quirks());

    let (result, renames) = copy_tree(copy, &source, &target);

    assert!(result.is_ok(), "{row}: {result:?}");
    assert_eq!(names(&target)?, [saved], "{row}");
    let contents = if is_folder {
        target.join(saved).join("inside")
    } else {
        target.join(saved)
    };
    assert_eq!(fs::read(contents)?, name.as_bytes(), "{row}");
    let expected = cause
        .map(|cause| (name.to_string(), saved.to_string(), cause))
        .into_iter()
        .collect::<Vec<_>>();
    assert_eq!(renames, expected, "{row}");
    Ok(())
}

#[test]
fn copies_onto_mounts_with_windows_rules_fix_names_and_report_them() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let (folder, cut) = ("d".repeat(220), "d".repeat(213));
    let rows = [
        file_named(("ext4", Quirks::ext4), "a?b:c.txt", "a?b:c.txt", None),
        file_named(
            ("exfat-fuse", Quirks::exfat_fuse),
            "a:b",
            "a_b",
            Some(Invalid),
        ),
        file_named(("ntfs3", Quirks::ntfs3), "CON", "CON_", Some(Invalid)),
        file_named(
            ("ntfs-3g", Quirks::ntfs_3g),
            "con.txt",
            "con_.txt",
            Some(Invalid),
        ),
        file_named(("cifs", Quirks::cifs), "x:y", "x_y", Some(Invalid)),
        // Samba makes a folder under a longer name first.
        WindowsName {
            is_folder: true,
            ..file_named(("cifs", Quirks::cifs), &folder, &cut, Some(TooLong))
        },
        file_named(
            ("cifs nocase", Quirks::cifs_nocase),
            "😀.txt",
            "_.txt",
            Some(Invalid),
        ),
    ];
    for (copy_label, copy) in COPIES {
        for case in &rows {
            check_windows_name(copy_label, copy, case)?;
        }
    }
    Ok(())
}
