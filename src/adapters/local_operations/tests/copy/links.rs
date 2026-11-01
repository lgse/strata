// SPDX-License-Identifier: MIT

use crate::services::{KeptCause, KeptItem, SkippedItem, SpecialKind, TransferReport};

use super::*;

fn links_refused_as_not_permitted() -> Quirks {
    Quirks {
        symlinks: Support::Rejected(Errno::PERM),
        ..Quirks::ext4()
    }
}

fn ntfs3_refusing_targets_as_invalid() -> Quirks {
    Quirks {
        symlinks: Support::Rejected(Errno::INVAL),
        ..Quirks::ntfs3()
    }
}

fn ntfs3_refusing_targets_as_too_large() -> Quirks {
    Quirks {
        symlinks: Support::Rejected(Errno::FBIG),
        ..Quirks::ntfs3()
    }
}

fn ntfs_3g_refusing_targets_as_illegal() -> Quirks {
    Quirks {
        symlinks: Support::Rejected(Errno::ILSEQ),
        ..Quirks::ntfs_3g()
    }
}

fn names_a_link(name: &OsStr) -> bool {
    name.as_encoded_bytes().ends_with(b"link")
}

fn ext4_refusing_link_names_as_invalid() -> Quirks {
    Quirks {
        invalid_name: Some(|name| names_a_link(name).then_some(Errno::INVAL)),
        ..Quirks::ext4()
    }
}

fn ext4_refusing_link_names_as_illegal() -> Quirks {
    Quirks {
        invalid_name: Some(|name| names_a_link(name).then_some(Errno::ILSEQ)),
        ..Quirks::ext4()
    }
}

fn item(source: &Path, conflict: TransferConflict) -> PasteItem {
    PasteItem {
        source: Location::local(source),
        conflict,
        target_name: None,
    }
}

fn conflict_for(replacing: bool) -> TransferConflict {
    if replacing {
        TransferConflict::ReplaceExisting
    } else {
        TransferConflict::FailIfExists
    }
}

fn symlinks_asked(scope: &FsQuirks) -> usize {
    scope
        .calls()
        .iter()
        .filter(|call| call.op == Op::Symlink)
        .count()
}

fn exfat_fuse_without_udev() -> Quirks {
    Quirks {
        device_format: None,
        ..Quirks::exfat_fuse()
    }
}

/// As a share with POSIX extensions refuses a target past 1003 UTF-16
/// units, and any disk that is full.
fn cifs_posix_refusing_links_for_space() -> Quirks {
    Quirks {
        symlinks: Support::Rejected(Errno::NOSPC),
        ..Quirks::cifs_posix()
    }
}

struct Links {
    label: &'static str,
    quirks: Preset,
    /// Whether the links are left out, or else fail the copy.
    left_out: Option<bool>,
    /// How many links two copies in turn ask the mount for.
    asked: [usize; 2],
}

#[test]
fn links_inside_a_copied_folder_are_left_out_where_the_destination_cannot_hold_them()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let rows = [
        Links {
            label: "ext4",
            quirks: Quirks::ext4,
            left_out: Some(false),
            asked: [2, 2],
        },
        // The driver has none, so it is never asked.
        Links {
            label: "vfat",
            quirks: Quirks::vfat,
            left_out: Some(true),
            asked: [0, 0],
        },
        Links {
            label: "exfat-fuse",
            quirks: Quirks::exfat_fuse,
            left_out: Some(true),
            asked: [0, 0],
        },
        // One refusal teaches the mount, for later copies too.
        Links {
            label: "cifs",
            quirks: Quirks::cifs,
            left_out: Some(true),
            asked: [1, 0],
        },
        Links {
            label: "exfat-fuse udev cannot name",
            quirks: exfat_fuse_without_udev,
            left_out: Some(true),
            asked: [1, 0],
        },
        // A refusal of one link, or of its target, teaches nothing.
        Links {
            label: "refusing each link (made up)",
            quirks: links_refused_as_not_permitted,
            left_out: Some(true),
            asked: [2, 2],
        },
        Links {
            label: "ntfs3, malformed targets",
            quirks: ntfs3_refusing_targets_as_invalid,
            left_out: Some(true),
            asked: [2, 2],
        },
        Links {
            label: "ntfs3, targets too long",
            quirks: ntfs3_refusing_targets_as_too_large,
            left_out: Some(true),
            asked: [2, 2],
        },
        Links {
            label: "ntfs-3g, malformed targets",
            quirks: ntfs_3g_refusing_targets_as_illegal,
            left_out: Some(true),
            asked: [2, 2],
        },
        Links {
            label: "cifs with POSIX extensions, full",
            quirks: cifs_posix_refusing_links_for_space,
            left_out: None,
            asked: [1, 1],
        },
    ];
    for row in &rows {
        check_links_inside(row)?;
    }
    Ok(())
}

/// A source folder whose `link` and `sub/link` point at `a.txt`, and an
/// empty `disk` to copy it onto.
fn linked_folder(root: &Path) -> io::Result<(PathBuf, PathBuf)> {
    let source = root.join("source");
    let disk = root.join("disk");
    fs::create_dir_all(source.join("sub"))?;
    fs::create_dir(&disk)?;
    fs::write(source.join("a.txt"), b"a")?;
    fs::write(source.join("sub/b.txt"), b"b")?;
    std::os::unix::fs::symlink("a.txt", source.join("link"))?;
    std::os::unix::fs::symlink("../a.txt", source.join("sub/link"))?;
    Ok((source, disk))
}

fn check_links_inside(row: &Links) -> Result<(), Box<dyn Error>> {
    let label = row.label;
    let root = tempfile::tempdir()?;
    let (source, disk) = linked_folder(root.path())?;
    let scope = FsQuirks::install(&disk, (row.quirks)());
    for (copy, asked) in ["first", "second"].into_iter().zip(row.asked) {
        let target = disk.join(copy);
        scope.clear_calls();

        let (copied, report) = reporting(
            &source,
            Destination::New(target.clone(), Taken::Fail),
            Options {
                names: fixing(),
                ..Options::default()
            },
        );

        assert_eq!(symlinks_asked(&scope), asked, "{label}: {copy}");
        let Some(left_out) = row.left_out else {
            assert!(copied.is_err(), "{label}: {copy}");
            assert_eq!(names(&disk)?, Vec::<String>::new(), "{label}");
            continue;
        };
        copied.map_err(|error| format!("{label}: {error}"))?;
        assert_eq!(fs::read(target.join("sub/b.txt"))?, b"b", "{label}");
        assert_links_copied_or_left_out(&target, report, left_out, label)?;
    }
    Ok(())
}

fn assert_links_copied_or_left_out(
    target: &Path,
    report: TransferReport,
    left_out: bool,
    label: &str,
) -> io::Result<()> {
    let mut skipped = report
        .skipped
        .into_iter()
        .map(|item| (item.location, item.kind))
        .collect::<Vec<_>>();
    skipped.sort_by(|left, right| left.0.compare(&right.0));
    if !left_out {
        assert_eq!(
            fs::read_link(target.join("sub/link"))?,
            Path::new("../a.txt")
        );
        assert_eq!(skipped, [], "{label}");
        return Ok(());
    }
    assert_eq!(names(target)?, ["a.txt", "sub"], "{label}");
    assert_eq!(names(&target.join("sub"))?, ["b.txt"], "{label}");
    let links =
        ["link", "sub/link"].map(|link| (Location::local(target.join(link)), SpecialKind::Symlink));
    assert_eq!(skipped, links, "{label}");
    Ok(())
}

struct SelectedLinks {
    label: &'static str,
    quirks: Preset,
    moving: bool,
    replacing: bool,
    asked: usize,
}

#[test]
fn selected_links_the_destination_cannot_hold_are_left_out_and_the_paste_goes_on()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let row = |label, quirks: Preset, moving, replacing, asked| SelectedLinks {
        label,
        quirks,
        moving,
        replacing,
        asked,
    };
    // vfat has no links, cifs refuses the first one it is asked for, ntfs3
    // refuses each link's target, and the last mounts every name for a link.
    let targets = ntfs3_refusing_targets_as_invalid;
    let (invalid, illegal) = (
        ext4_refusing_link_names_as_invalid,
        ext4_refusing_link_names_as_illegal,
    );
    let rows = [
        row("vfat", Quirks::vfat, false, false, 0),
        row("vfat, moved", Quirks::vfat, true, false, 0),
        row("vfat, replacing", Quirks::vfat, false, true, 0),
        row("cifs", Quirks::cifs, false, false, 1),
        row("cifs, moved", Quirks::cifs, true, false, 1),
        row("cifs, replacing", Quirks::cifs, false, true, 1),
        row("ntfs3, malformed targets", targets, false, false, 2),
        row(
            "ntfs3, malformed targets, replacing",
            targets,
            false,
            true,
            2,
        ),
        row("names invalid", invalid, false, false, 2),
        row("names illegal, moved", illegal, true, false, 2),
    ];
    for row in &rows {
        check_selected_links(row)?;
    }
    Ok(())
}

fn check_selected_links(row: &SelectedLinks) -> Result<(), Box<dyn Error>> {
    let label = row.label;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    let disk = root.path().join("disk");
    fs::create_dir_all(&source)?;
    fs::create_dir(&disk)?;
    let links = ["a-link", "b-link"].map(|name| source.join(name));
    let file = source.join("file.txt");
    for link in &links {
        std::os::unix::fs::symlink("file.txt", link)?;
    }
    fs::write(&file, b"file")?;
    if row.replacing {
        fs::write(disk.join("b-link"), b"earlier")?;
    }
    let scope = FsQuirks::install(&disk, (row.quirks)());

    let events = run_paste(PasteRequest {
        id: OperationRequestId(1590),
        destination: Location::local(&disk),
        items: vec![
            item(&links[0], TransferConflict::FailIfExists),
            item(&links[1], conflict_for(row.replacing)),
            item(&file, TransferConflict::FailIfExists),
        ],
        move_sources: row.moving,
    });

    assert_eq!(symlinks_asked(&scope), row.asked, "{label}");
    for link in &links {
        assert_eq!(fs::read_link(link)?, Path::new("file.txt"), "{label}");
    }
    assert_eq!(fs::read(disk.join("file.txt"))?, b"file", "{label}");
    assert_eq!(file.exists(), !row.moving, "{label}");
    assert_links_left_off(&disk, row.replacing, label)?;
    assert_selected_links_reported(&events, row, (&links, &file), &disk);
    assert!(
        matches!(
            terminal_transfer(&events),
            Some(OperationEvent::Pasted { locations, .. })
                if *locations == [Location::local(&file)]
        ),
        "{label}"
    );
    Ok(())
}

/// Neither link reached `disk`, which keeps what it held at `b-link`.
fn assert_links_left_off(disk: &Path, replacing: bool, label: &str) -> io::Result<()> {
    assert!(
        fs::symlink_metadata(disk.join("a-link")).is_err(),
        "{label}"
    );
    if replacing {
        assert_eq!(fs::read(disk.join("b-link"))?, b"earlier", "{label}");
    } else {
        assert!(
            fs::symlink_metadata(disk.join("b-link")).is_err(),
            "{label}"
        );
    }
    Ok(())
}

/// A move keeps the links at the source and moves only the file; a copy
/// reports the links it left out of the destination.
fn assert_selected_links_reported(
    events: &[OperationEvent],
    row: &SelectedLinks,
    (links, file): (&[PathBuf; 2], &Path),
    disk: &Path,
) {
    let label = row.label;
    let report = followed_report(events);
    if !row.moving {
        let skipped = ["a-link", "b-link"].map(|name| SkippedItem {
            location: Location::local(disk.join(name)),
            kind: SpecialKind::Symlink,
        });
        assert_eq!(
            (report.skipped, report.kept),
            (skipped.to_vec(), Vec::new()),
            "{label}"
        );
        return;
    }
    assert_eq!(
        item_moves(events),
        [(
            Location::local(file),
            Location::local(disk.join("file.txt"))
        )],
        "{label}"
    );
    let kept = links.clone().map(|link| KeptItem {
        location: Location::local(link),
        cause: KeptCause::Skipped(SpecialKind::Symlink),
    });
    assert_eq!(
        (report.kept, report.skipped),
        (kept.to_vec(), Vec::new()),
        "{label}"
    );
}

#[test]
fn malformed_link_targets_are_left_out_where_names_must_be_utf8() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let posix = Quirks::cifs_posix;
    let rows: [Malformed; 4] = [
        ("cifs-posix", posix, false, false),
        ("cifs-posix, moved", posix, true, false),
        ("cifs-posix, replacing", posix, false, true),
        ("ext4", Quirks::ext4, false, false),
    ];
    for row in rows {
        check_malformed_targets(row)?;
    }
    Ok(())
}

fn malformed() -> &'static OsStr {
    <OsStr as std::os::unix::ffi::OsStrExt>::from_bytes(b"x\xffy")
}

/// A mount, and whether the paste moves or replaces a `bad` link already
/// at the destination.
type Malformed = (&'static str, Preset, bool, bool);

fn check_malformed_targets(
    (label, preset, moving, replacing): Malformed,
) -> Result<(), Box<dyn Error>> {
    let held = label == "ext4";
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    let disk = root.path().join("disk");
    let folder = source.join("folder");
    fs::create_dir_all(&folder)?;
    fs::create_dir(&disk)?;
    let selected = source.join("bad");
    std::os::unix::fs::symlink(malformed(), &selected)?;
    std::os::unix::fs::symlink(malformed(), folder.join("bad"))?;
    std::os::unix::fs::symlink("good", folder.join("good"))?;
    if replacing {
        fs::write(disk.join("bad"), b"earlier")?;
    }
    let scope = FsQuirks::install(&disk, preset());

    let events = run_paste(PasteRequest {
        id: OperationRequestId(1590),
        destination: Location::local(&disk),
        items: vec![
            item(&folder, TransferConflict::FailIfExists),
            item(&selected, conflict_for(replacing)),
        ],
        move_sources: moving,
    });

    assert_eq!(
        scope.count(Op::Symlink),
        if held { 3 } else { 1 },
        "{label}"
    );
    assert_eq!(
        fs::read_link(disk.join("folder/good"))?,
        Path::new("good"),
        "{label}"
    );
    let report = followed_report(&events);
    if held {
        let copied = [disk.join("folder/bad"), disk.join("bad")].map(fs::read_link);
        let [in_folder, alone] = copied;
        assert_eq!(
            (in_folder?, alone?),
            (PathBuf::from(malformed()), PathBuf::from(malformed())),
            "{label}"
        );
        assert_eq!(report.skipped, [], "{label}");
        return Ok(());
    }
    assert_eq!(names(&disk.join("folder"))?, ["good"], "{label}");
    if replacing {
        assert_eq!(fs::read(disk.join("bad"))?, b"earlier", "{label}");
    } else {
        assert!(fs::symlink_metadata(disk.join("bad")).is_err(), "{label}");
    }
    if moving {
        assert_malformed_kept(&report, &selected, &folder, label)?;
    } else {
        assert_malformed_skipped(&report, &disk, label);
    }
    Ok(())
}

fn assert_malformed_kept(
    report: &TransferReport,
    selected: &Path,
    folder: &Path,
    label: &str,
) -> io::Result<()> {
    assert_eq!(names(folder)?, ["bad"], "{label}");
    assert_eq!(fs::read_link(selected)?, malformed(), "{label}");
    let kept = [selected.to_path_buf(), folder.join("bad")].map(|path| KeptItem {
        location: Location::local(path),
        cause: KeptCause::Skipped(SpecialKind::Symlink),
    });
    let mut reported = report.kept.clone();
    reported.sort_by(|left, right| left.location.compare(&right.location));
    assert_eq!(reported, kept, "{label}");
    Ok(())
}

fn assert_malformed_skipped(report: &TransferReport, disk: &Path, label: &str) {
    let skipped = [disk.join("bad"), disk.join("folder/bad")].map(|path| SkippedItem {
        location: Location::local(path),
        kind: SpecialKind::Symlink,
    });
    let mut reported = report.skipped.clone();
    reported.sort_by(|left, right| left.location.compare(&right.location));
    assert_eq!(reported, skipped, "{label}");
}

#[test]
fn a_selected_link_whose_name_sshfs_answers_as_not_permitted_is_numbered_not_left_out()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    let disk = root.path().join("disk");
    fs::create_dir_all(&source)?;
    fs::create_dir(&disk)?;
    let link = source.join("link");
    std::os::unix::fs::symlink("file.txt", &link)?;
    for taken in ["link", "link (1)"] {
        fs::write(disk.join(taken), b"earlier")?;
    }
    let _disk = FsQuirks::install(&disk, Quirks::sshfs());

    let events = run_paste(PasteRequest {
        id: OperationRequestId(1590),
        destination: Location::local(&disk),
        items: vec![PasteItem {
            source: Location::local(&link),
            conflict: TransferConflict::KeepBoth,
            target_name: None,
        }],
        move_sources: false,
    });

    assert_eq!(fs::read(disk.join("link (1)"))?, b"earlier");
    assert_eq!(fs::read_link(disk.join("link (2)"))?, Path::new("file.txt"));
    assert!(followed_report(&events).is_empty());
    Ok(())
}

#[test]
fn a_paste_through_a_symbolic_link_takes_the_rules_of_the_mount_behind_it()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    let stick = root.path().join("stick");
    let link = root.path().join("link");
    fs::create_dir_all(source.join("photos?"))?;
    fs::create_dir(&stick)?;
    std::os::unix::fs::symlink(&stick, &link)?;
    fs::write(source.join("a:b"), b"colon")?;
    fs::write(source.join("photos?/x*y.jpg"), b"star")?;
    fs::File::create(source.join("disk.img"))?.set_len(FAT32_MAX_FILE_SIZE + 1)?;
    let _stick = FsQuirks::install(&stick, Quirks::vfat());
    let paste = |names: &[&str]| PasteRequest {
        id: OperationRequestId(1530),
        destination: Location::local(&link),
        items: names
            .iter()
            .map(|name| PasteItem {
                source: Location::local(source.join(name)),
                conflict: TransferConflict::FailIfExists,
                target_name: None,
            })
            .collect(),
        move_sources: false,
    };

    run_paste(paste(&["a:b", "photos?"]));

    assert_eq!(names(&stick)?, ["a_b", "photos_"]);
    assert_eq!(fs::read(stick.join("a_b"))?, b"colon");
    assert_eq!(fs::read(stick.join("photos_/x_y.jpg"))?, b"star");
    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let _operation = LocalOperationProvider.paste(
        paste(&["disk.img"]),
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    );
    drive_until_transfer_settles(&events);

    let too_large = rust_i18n::t!(
        "%{name} is too large for a FAT32 drive (maximum file size: 4 GiB). Use an exFAT or another large-file-capable drive instead.",
        name = "disk.img"
    );
    assert!(
        matches!(
            events.borrow().last(),
            Some(OperationEvent::TransferFailed { message, .. }) if *message == too_large
        ),
        "{:?}",
        events.borrow()
    );
    assert_eq!(names(&stick)?, ["a_b", "photos_"]);
    Ok(())
}
