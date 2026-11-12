// SPDX-License-Identifier: MIT

use super::*;

#[derive(Clone, Copy, Debug)]
enum Swap {
    BeforeSettingAside,
    AsItIsSetAside,
    AsACaseOnlyRenameParksIt,
}

/// The substituted item went aside under a hidden folder, and the failure
/// names where.
fn assert_stranded(
    root: &Path,
    name: &str,
    result: Result<(), RenameFailure>,
    swap: Swap,
) -> Result<(), Box<dyn Error>> {
    let left = names(root)?;
    let [hidden, _] = &left[..] else {
        return Err(format!("{swap:?}: expected two names, found {left:?}").into());
    };
    assert!(
        hidden.starts_with(HiddenName::Originals.prefix()),
        "{swap:?}: {left:?}"
    );
    assert_eq!(names(&root.join(hidden))?, [name], "{swap:?}");
    assert_eq!(
        result,
        Err(RenameFailure::Stranded {
            temporary: Path::new(hidden).join(name).into_os_string(),
            error: PlaceError::Unconfirmed,
        }),
        "{swap:?}"
    );
    let stranger = fs::read(root.join(hidden).join(name))?;
    assert_eq!(stranger, b"stranger", "{swap:?}");
    Ok(())
}

fn check_set_aside(swap: Swap) -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let (name, skip, mut quirks) = match swap {
        Swap::AsACaseOnlyRenameParksIt => ("report", 1, case_insensitive(Quirks::default())),
        _ => ("target", 0, Quirks::default()),
    };
    let item = root.path().join(name);
    fs::write(&item, b"original")?;
    let expected = identity(&item);
    let (moved, stranger) = (root.path().join("moved"), item.clone());
    let mut racer: Racer = Box::new(move || {
        fs::rename(&stranger, &moved).expect("move the original away");
        fs::write(&stranger, b"stranger").expect("substitute");
    });
    match swap {
        Swap::BeforeSettingAside => racer(),
        _ => quirks.before = vec![(Op::Rename, name, skip, racer)],
    }
    let scope = FsQuirks::install(root.path(), quirks);
    let parent = open_local_parent_directory(root.path())?;

    let result = match swap {
        Swap::AsACaseOnlyRenameParksIt => rename_in(root.path(), "report", "Report").map(drop),
        _ => set_aside(fs(), Entry::new(&parent, OsStr::new(name)), expected, None).map(drop),
    };

    assert_eq!(
        fs::read(root.path().join("moved"))?,
        b"original",
        "{swap:?}"
    );
    let Swap::BeforeSettingAside = swap else {
        return assert_stranded(root.path(), name, result, swap);
    };
    assert_eq!(result, Err(RenameFailure::Place(PlaceError::Changed)));
    assert_eq!(scope.count(Op::Rename), 0);
    let substituted = fs::read(&item)?;
    assert_eq!(substituted, b"stranger");
    assert_eq!(names(root.path())?, ["moved", name]);
    Ok(())
}

#[test]
fn set_aside_refuses_a_substituted_item_or_names_where_it_went() -> Result<(), Box<dyn Error>> {
    for swap in [
        Swap::BeforeSettingAside,
        Swap::AsItIsSetAside,
        Swap::AsACaseOnlyRenameParksIt,
    ] {
        check_set_aside(swap)?;
    }
    Ok(())
}

type ProbeCase = (Preset, Probed, Feasibility, Feasibility, InodeNumbers);

const fn probed(
    no_replace: Option<bool>,
    hard_links: Option<bool>,
    rename_over_empty_directory: Option<bool>,
    links_keep_numbers: bool,
) -> Probed {
    Probed {
        no_replace,
        hard_links,
        rename_over_empty_directory,
        links_keep_numbers,
    }
}

fn check_probe(
    (quirks, expected, files, folders, numbers): ProbeCase,
    learned_first: bool,
) -> Result<(), Box<dyn Error>> {
    let row = format!("{:?}, learned first: {learned_first}", quirks().mount_type);
    let quirks = || match quirks() {
        unnamed if unnamed.mount_type.is_none() => unnamed.on_ext4(),
        named => named,
    };
    let root = tempfile::tempdir()?;
    let parent = open_local_parent_directory(root.path())?;
    {
        let _scope = FsQuirks::install(root.path(), quirks());
        assert_eq!(probe(fs(), parent.as_fd(), true, true, true), Ok(expected));
    }
    let _scope = FsQuirks::install(root.path(), quirks());
    if learned_first {
        known_mount(fs(), parent.as_fd());
    }
    let feasible = [false, true].map(|folders| can_set_aside(fs(), parent.as_fd(), folders));
    assert_eq!(feasible, [Ok(files), Ok(folders)], "{row}");
    links_hold(fs(), parent.as_fd());
    assert_eq!(
        known_mount(fs(), parent.as_fd()).1.identity.numbers,
        numbers,
        "{row}"
    );
    assert!(names(root.path())?.is_empty(), "{:?}", names(root.path())?);
    Ok(())
}

/// Mounts that rename in place, with and without the flags and links.
fn renaming_mounts() -> [ProbeCase; 6] {
    use Feasibility::{No, Yes};
    use InodeNumbers::{Kept, Renewed};
    let all = probed(Some(true), Some(true), Some(true), true);
    [
        (Quirks::default, all, Yes, Yes, Kept),
        (
            Quirks::without_rename_flags,
            probed(Some(false), Some(true), Some(true), true),
            Yes,
            Yes,
            Kept,
        ),
        (
            Quirks::vboxsf,
            probed(Some(false), Some(false), Some(false), false),
            Yes,
            Yes,
            Renewed,
        ),
        (
            || Quirks {
                rename_over_empty_dir: OverEmptyDirectory::Eperm,
                ..Quirks::vboxsf()
            },
            probed(Some(false), Some(false), None, false),
            Yes,
            Yes,
            Renewed,
        ),
        (
            || Quirks {
                links: Support::Rejected(Errno::OPNOTSUPP),
                ..Quirks::without_rename_flags()
            },
            probed(Some(false), Some(false), Some(true), false),
            Yes,
            Yes,
            Kept,
        ),
        (
            Quirks::without_folder_placement,
            probed(Some(false), Some(true), Some(false), true),
            Yes,
            No,
            Kept,
        ),
    ]
}

/// Mounts that place nothing, refuse probe names, or renumber links.
fn other_mounts() -> [ProbeCase; 5] {
    use Feasibility::{No, Yes};
    use InodeNumbers::{Kept, Renewed};
    let all = probed(Some(true), Some(true), Some(true), true);
    [
        (
            Quirks::without_placement,
            probed(Some(false), None, Some(false), false),
            No,
            No,
            Kept,
        ),
        (
            // SMB with POSIX extensions: EISDIR from an exclusive file create is
            // a taken name, but from mkdir it is not.
            || Quirks {
                fail: vec![
                    (Op::Create, ".strata-probe-*", 0, Errno::ISDIR),
                    (Op::Mkdir, ".strata-probe-*", 0, Errno::ISDIR),
                ],
                ..Quirks::default()
            },
            probed(Some(true), Some(true), None, true),
            Yes,
            Yes,
            Kept,
        ),
        (
            // cifs with server inode numbers turned off.
            || Quirks {
                link_inodes: true,
                ..Quirks::default()
            },
            probed(Some(true), Some(false), Some(true), false),
            Yes,
            Yes,
            Kept,
        ),
        (
            Quirks::sshfs,
            probed(Some(false), Some(false), Some(true), false),
            Yes,
            Yes,
            Renewed,
        ),
        (Quirks::ntfs_3g, all, Yes, Yes, Kept),
    ]
}

#[test]
fn the_probe_learns_each_quirk_and_leaves_nothing_behind() -> Result<(), Box<dyn Error>> {
    for case in renaming_mounts().into_iter().chain(other_mounts()) {
        for learned_first in [false, true] {
            check_probe(case, learned_first)?;
        }
    }
    Ok(())
}

#[test]
fn the_probe_keeps_files_it_cannot_prove_are_its_own() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let parent = open_local_parent_directory(root.path())?;
    let dir = root.path().to_path_buf();
    let racer: Racer = Box::new(move || {
        for name in names(&dir).expect("list") {
            fs::write(dir.join("theirs"), b"theirs").expect("write");
            fs::rename(dir.join("theirs"), dir.join(name)).expect("replace");
        }
    });
    let _scope = FsQuirks::install(
        root.path(),
        Quirks {
            // After the probe saw its link, before it removes it.
            before: vec![(Op::Stat, ".strata-probe-*", 2, racer)],
            ..Quirks::sshfs()
        },
    );

    let probed = probe(fs(), parent.as_fd(), true, true, false)?;

    assert_eq!(probed.hard_links, Some(false));
    let left = names(root.path())?;
    assert_eq!(left.len(), 2, "{left:?}");
    for name in left {
        assert_eq!(fs::read(root.path().join(name))?, b"theirs");
    }
    Ok(())
}
