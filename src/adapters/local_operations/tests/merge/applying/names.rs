// SPDX-License-Identifier: MIT

use super::*;

/// Merges `source` into `target` with fixed names through `copy`, staging
/// what it overwrites through `stage`.
fn merge_fixing(
    (source, target): (&Path, &Path),
    copy: CopyStep,
    stage: StageOverwrite,
    on_merged: &dyn Fn(MergePlan),
) -> Result<(), glib::Error> {
    let hooks = hooks(fixing(), copy, stage, on_merged);
    merge_hooked(gio::File::for_path(source), target, hooks)
}

type CopyCheck = fn(&str, CopyStep) -> Result<(), Box<dyn Error>>;

/// Runs `check` with the engine's copy and with GIO's.
fn with_both_copies(check: CopyCheck) -> Result<(), Box<dyn Error>> {
    check("engine", engine_copy(fixing()))?;
    check("GIO", gio_merge_copy(fixing()))
}

fn both_copies() -> [(&'static str, MakeCopy); 2] {
    [
        ("engine", || engine_copy(fixing())),
        ("GIO", || gio_merge_copy(fixing())),
    ]
}

fn check_link_replacement(row: &str, quirks: Preset, links: bool) -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    let target = root.path().join("target");
    fs::create_dir_all(&source)?;
    fs::create_dir_all(&target)?;
    fs::write(source.join("a.txt"), b"a")?;
    std::os::unix::fs::symlink("a.txt", source.join("link"))?;
    fs::write(target.join("link"), b"earlier")?;
    let _target = FsQuirks::install(&target, quirks());
    let (stage, staged) = rename_stage(root.path().join("staging"));
    let (_plans, on_merged) = recorded_plans();

    merge_fixing((&source, &target), engine_copy(fixing()), stage, &on_merged)?;

    assert_eq!(fs::read(target.join("a.txt"))?, b"a", "{row}");
    if links {
        assert_eq!(*staged.borrow(), [Location::local(target.join("link"))]);
        assert_eq!(fs::read_link(target.join("link"))?, Path::new("a.txt"));
    } else {
        assert_eq!(*staged.borrow(), Vec::<Location>::new());
        assert_eq!(fs::read(target.join("link"))?, b"earlier");
    }
    Ok(())
}

#[test]
fn a_merge_replaces_nothing_with_a_link_the_destination_cannot_hold() -> Result<(), Box<dyn Error>>
{
    let _serial = serial()?;
    for (row, quirks, links) in [
        ("vfat", Quirks::vfat as Preset, false),
        ("ext4", Quirks::ext4, true),
    ] {
        check_link_replacement(row, quirks, links)?;
    }
    Ok(())
}

/// What a staging rename put aside, sorted.
fn staged_contents(staging: &Path, count: usize) -> io::Result<Vec<String>> {
    let mut contents = (0..count)
        .map(|index| fs::read_to_string(staging.join(format!("staged-{index}"))))
        .collect::<io::Result<Vec<_>>>()?;
    contents.sort();
    Ok(contents)
}

/// Names FAT must fix in a `source/nested?` folder merged onto a FAT
/// `target/nested_` that already holds some of the fixed names.
fn fat_names_fixture(root: &Path) -> io::Result<(PathBuf, PathBuf)> {
    let source = root.join("source");
    let target = root.join("target");
    fs::create_dir_all(source.join("nested?"))?;
    fs::create_dir_all(target.join("nested_"))?;
    write_files(
        &source.join("nested?"),
        &[
            ("a?.txt", "question"),
            ("a:.txt", "colon"),
            ("b:.txt", "b colon"),
            ("b_.txt", "b kept"),
            ("C.txt", "upper c"),
            ("c.txt", "lower c"),
        ],
    )?;
    // `b_ (1).txt` is what an earlier merge of `b:.txt` saved.
    write_files(
        &target.join("nested_"),
        &[
            ("a_.txt", "original a"),
            ("b_.txt", "original b"),
            ("b_ (1).txt", "earlier b colon"),
        ],
    )?;
    Ok((source, target))
}

fn check_fat_merge(row: &str, copy: CopyStep) -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let (source, target) = fat_names_fixture(root.path())?;
    let _sources = FsQuirks::install(&source, Quirks::default());
    let _target = FsQuirks::install(&target, Quirks::vfat());
    let staging = root.path().join("staging");
    let (stage, staged) = rename_stage(staging.clone());
    let (plans, on_merged) = recorded_plans();
    merge_fixing((&source, &target), copy, stage, &on_merged)?;
    let nested = target.join("nested_");
    assert_files(
        &nested,
        &[
            ("a_.txt", "colon"),
            ("a_ (1).txt", "question"),
            ("b_.txt", "b kept"),
            ("b_ (1).txt", "b colon"),
            ("C.txt", "upper c"),
            ("c (1).txt", "lower c"),
        ],
        row,
    )?;
    assert_eq!(names(&nested)?.len(), 6, "{row}");
    let staged_contents = staged_contents(&staging, staged.borrow().len())?;
    assert_eq!(
        staged_contents,
        ["earlier b colon", "original a", "original b"],
        "{row}"
    );
    let plans = plans.borrow();
    assert_eq!(
        created_set(&plans[0]),
        locations_in(&nested, &["a_ (1).txt", "C.txt", "c (1).txt"]),
        "{row}"
    );
    assert_eq!(plans[0].overwritten, *staged.borrow(), "{row}");
    // A GIO copy proves no creation, so it records nothing for undo to delete.
    let (count, proven) = match row {
        "engine" => (2, plans[0].overwritten.iter().collect()),
        _ => (1, HashSet::new()),
    };
    assert_eq!(plans.len(), count, "{row}");
    let placed = plans[1..]
        .iter()
        .flat_map(|plan| plan.placed.keys())
        .collect::<HashSet<_>>();
    assert_eq!(placed, proven, "{row}");
    Ok(())
}

#[test]
fn fat_merge_overwrites_the_names_it_plans_and_keeps_the_names_that_need_no_fix()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    with_both_copies(check_fat_merge)
}

fn check_merge_off_fat(row: &str, copy: CopyStep) -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    let target = root.path().join("target");
    fs::create_dir_all(source.join("nested"))?;
    fs::create_dir_all(target.join("nested"))?;
    write_files(
        &source.join("nested"),
        &[("a:b", "colon"), ("C.txt", "upper"), ("c.txt", "lower")],
    )?;
    fs::write(target.join("nested/a_b"), "original")?;
    let _target = FsQuirks::install(&target, Quirks::ext4());
    let (stage, staged) = rename_stage(root.path().join("staging"));
    let (plans, on_merged) = recorded_plans();

    merge_fixing((&source, &target), copy, stage, &on_merged)?;

    let nested = target.join("nested");
    assert_files(
        &nested,
        &[
            ("a:b", "colon"),
            ("a_b", "original"),
            ("C.txt", "upper"),
            ("c.txt", "lower"),
        ],
        row,
    )?;
    assert!(staged.borrow().is_empty(), "{row}");
    assert_eq!(
        created_set(&plans.borrow()[0]),
        locations_in(&nested, &["a:b", "C.txt", "c.txt"]),
        "{row}"
    );
    Ok(())
}

#[test]
fn a_merge_off_fat_keeps_the_names_fat_would_fix() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    with_both_copies(check_merge_off_fat)
}

/// A folder of a FAT stick holding another device, unless it is the stick's
/// own, and what the merge leaves in it.
struct InnerDisk {
    own_device: bool,
    holds: &'static [(&'static str, &'static str)],
    overwritten: Option<&'static str>,
}

fn check_fat_stick_folder(
    copy_row: &str,
    copy: MakeCopy,
    disk_case: &InnerDisk,
) -> Result<(), Box<dyn Error>> {
    let InnerDisk {
        own_device,
        holds,
        overwritten,
    } = *disk_case;
    let row = format!("{copy_row}, own device {own_device}");
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    let stick = root.path().join("stick");
    let disk = stick.join("disk");
    fs::create_dir_all(source.join("disk"))?;
    fs::create_dir_all(&disk)?;
    fs::write(source.join("x:y"), "top")?;
    fs::write(source.join("disk/a:b"), "colon")?;
    fs::write(disk.join("a_b"), "original")?;
    let _stick = FsQuirks::install(&stick, Quirks::vfat());
    let _disk = FsQuirks::install(
        &disk,
        Quirks {
            own_device,
            ..Quirks::ext4()
        },
    );
    let (stage, staged) = rename_stage(root.path().join("staging"));
    let (plans, on_merged) = recorded_plans();

    merge_fixing((&source, &stick), copy(), stage, &on_merged)?;

    assert_eq!(fs::read(stick.join("x_y"))?, b"top", "{row}");
    let held: Vec<&str> = holds.iter().map(|(name, _)| *name).collect();
    assert_eq!(names(&disk)?, held, "{row}");
    assert_files(&disk, holds, &row)?;
    let overwritten = Vec::from_iter(overwritten.map(|name| Location::local(disk.join(name))));
    assert_eq!(*staged.borrow(), overwritten, "{row}");
    let plan = &plans.borrow()[0];
    assert_eq!(plan.overwritten, overwritten, "{row}");
    let created = [stick.join("x_y")]
        .into_iter()
        .chain(own_device.then(|| disk.join("a:b")))
        .map(Location::local)
        .collect::<HashSet<_>>();
    assert_eq!(created_set(plan), created, "{row}");
    Ok(())
}

#[test]
fn a_merge_onto_fat_takes_the_rules_of_a_folder_inside_it_only_from_another_device_mounted_there()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    // On the stick's device, the folder's ext4 rules are never looked up.
    let disks = [
        InnerDisk {
            own_device: true,
            holds: &[("a:b", "colon"), ("a_b", "original")],
            overwritten: None,
        },
        InnerDisk {
            own_device: false,
            holds: &[("a_b", "colon")],
            overwritten: Some("a_b"),
        },
    ];
    for (copy_row, copy) in both_copies() {
        for disk in &disks {
            check_fat_stick_folder(copy_row, copy, disk)?;
        }
    }
    Ok(())
}

fn check_mounted_in_source(
    copy_row: &str,
    copy: MakeCopy,
    own_device: bool,
    landed: &str,
) -> Result<(), Box<dyn Error>> {
    let row = format!("{copy_row}, own device {own_device}");
    let root = tempfile::tempdir()?;
    let stick = root.path().join("stick");
    let source = stick.join("a");
    let target = stick.join("b");
    fs::create_dir_all(source.join("inner"))?;
    fs::create_dir_all(target.join("inner"))?;
    fs::write(source.join("inner/a:b"), "colon")?;
    let _stick = FsQuirks::install(
        &stick,
        Quirks {
            own_device: true,
            ..Quirks::ntfs3()
        },
    );
    let _inner = FsQuirks::install(
        &source.join("inner"),
        Quirks {
            own_device,
            ..Quirks::ext4()
        },
    );
    let (stage, _) = rename_stage(root.path().join("staging"));
    let (plans, on_merged) = recorded_plans();

    merge_fixing((&source, &target), copy(), stage, &on_merged)?;

    assert_eq!(names(&target.join("inner"))?, [landed], "{row}");
    assert_eq!(
        plans.borrow()[0].created,
        [Location::local(target.join("inner").join(landed))],
        "{row}"
    );
    Ok(())
}

#[test]
fn a_merge_within_one_disk_fixes_the_names_another_filesystem_mounted_in_its_source_brings()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    for (copy_row, copy) in both_copies() {
        for (own_device, landed) in [(true, "a_b"), (false, "a:b")] {
            check_mounted_in_source(copy_row, copy, own_device, landed)?;
        }
    }
    Ok(())
}

fn check_cut_names(row: &str, copy: CopyStep) -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    let disk = root.path().join("disk");
    fs::create_dir_all(&source)?;
    fs::create_dir(&disk)?;
    // Cut to 64 bytes, the long name differs from the short one in case only.
    fs::write(source.join(format!("A{}", "b".repeat(70))), "long")?;
    fs::write(source.join(format!("a{}", "b".repeat(63))), "short")?;
    let _disk = FsQuirks::install(
        &disk,
        Quirks {
            name_max: Some(64),
            ..Quirks::ext4()
        },
    );
    let (stage, _) = rename_stage(root.path().join("staging"));
    let (plans, on_merged) = recorded_plans();

    merge_fixing((&source, &disk), copy, stage, &on_merged)?;

    let made = names(&disk)?
        .into_iter()
        .map(|name| Location::local(disk.join(name)))
        .collect::<HashSet<_>>();
    assert_eq!(made.len(), 2, "{row}");
    assert_eq!(created_set(&plans.borrow()[0]), made, "{row}");
    Ok(())
}

#[test]
fn a_merge_records_the_names_its_copy_gives_a_cut_name() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    with_both_copies(check_cut_names)
}
