// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn set_aside_refuses_a_substituted_item_or_names_where_it_went() -> Result<(), Box<dyn Error>> {
    #[derive(Clone, Copy, Debug)]
    enum Swap {
        BeforeSettingAside,
        AsItIsSetAside,
        AsACaseOnlyRenameParksIt,
    }
    for swap in [
        Swap::BeforeSettingAside,
        Swap::AsItIsSetAside,
        Swap::AsACaseOnlyRenameParksIt,
    ] {
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
        if let Swap::BeforeSettingAside = swap {
            assert_eq!(result, Err(RenameFailure::Place(PlaceError::Changed)));
            assert_eq!(scope.count(Op::Rename), 0);
            assert_eq!(fs::read(&item)?, b"stranger");
            assert_eq!(names(root.path())?, ["moved", name]);
            continue;
        }
        let left = names(root.path())?;
        let [hidden, _] = &left[..] else {
            return Err(format!("{swap:?}: expected two names, found {left:?}").into());
        };
        assert!(
            hidden.starts_with(HiddenName::Originals.prefix()),
            "{swap:?}: {left:?}"
        );
        assert_eq!(names(&root.path().join(hidden))?, [name], "{swap:?}");
        assert_eq!(
            result,
            Err(RenameFailure::Stranded {
                temporary: Path::new(hidden).join(name).into_os_string(),
                error: PlaceError::Unconfirmed,
            }),
            "{swap:?}"
        );
        assert_eq!(
            fs::read(root.path().join(hidden).join(name))?,
            b"stranger",
            "{swap:?}"
        );
    }
    Ok(())
}

type ProbeCase = (Preset, Probed, Feasibility, Feasibility, InodeNumbers);

#[test]
fn the_probe_learns_each_quirk_and_leaves_nothing_behind() -> Result<(), Box<dyn Error>> {
    let cases: [ProbeCase; 11] = [
        (
            Quirks::default,
            Probed {
                no_replace: Some(true),
                hard_links: Some(true),
                rename_over_empty_directory: Some(true),
                links_keep_numbers: true,
            },
            Feasibility::Yes,
            Feasibility::Yes,
            InodeNumbers::Kept,
        ),
        (
            Quirks::without_rename_flags,
            Probed {
                no_replace: Some(false),
                hard_links: Some(true),
                rename_over_empty_directory: Some(true),
                links_keep_numbers: true,
            },
            Feasibility::Yes,
            Feasibility::Yes,
            InodeNumbers::Kept,
        ),
        (
            Quirks::vboxsf,
            Probed {
                no_replace: Some(false),
                hard_links: Some(false),
                rename_over_empty_directory: Some(false),
                links_keep_numbers: false,
            },
            Feasibility::Yes,
            Feasibility::Yes,
            InodeNumbers::Renewed,
        ),
        (
            || Quirks {
                rename_over_empty_dir: OverEmptyDirectory::Eperm,
                ..Quirks::vboxsf()
            },
            Probed {
                no_replace: Some(false),
                hard_links: Some(false),
                rename_over_empty_directory: None,
                links_keep_numbers: false,
            },
            Feasibility::Yes,
            Feasibility::Yes,
            InodeNumbers::Renewed,
        ),
        (
            || Quirks {
                links: Support::Rejected(Errno::OPNOTSUPP),
                ..Quirks::without_rename_flags()
            },
            Probed {
                no_replace: Some(false),
                hard_links: Some(false),
                rename_over_empty_directory: Some(true),
                links_keep_numbers: false,
            },
            Feasibility::Yes,
            Feasibility::Yes,
            InodeNumbers::Kept,
        ),
        (
            Quirks::without_folder_placement,
            Probed {
                no_replace: Some(false),
                hard_links: Some(true),
                rename_over_empty_directory: Some(false),
                links_keep_numbers: true,
            },
            Feasibility::Yes,
            Feasibility::No,
            InodeNumbers::Kept,
        ),
        (
            Quirks::without_placement,
            Probed {
                no_replace: Some(false),
                hard_links: None,
                rename_over_empty_directory: Some(false),
                links_keep_numbers: false,
            },
            Feasibility::No,
            Feasibility::No,
            InodeNumbers::Kept,
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
            Probed {
                no_replace: Some(true),
                hard_links: Some(true),
                rename_over_empty_directory: None,
                links_keep_numbers: true,
            },
            Feasibility::Yes,
            Feasibility::Yes,
            InodeNumbers::Kept,
        ),
        (
            // cifs with server inode numbers turned off.
            || Quirks {
                link_inodes: true,
                ..Quirks::default()
            },
            Probed {
                no_replace: Some(true),
                hard_links: Some(false),
                rename_over_empty_directory: Some(true),
                links_keep_numbers: false,
            },
            Feasibility::Yes,
            Feasibility::Yes,
            InodeNumbers::Kept,
        ),
        (
            Quirks::sshfs,
            Probed {
                no_replace: Some(false),
                hard_links: Some(false),
                rename_over_empty_directory: Some(true),
                links_keep_numbers: false,
            },
            Feasibility::Yes,
            Feasibility::Yes,
            InodeNumbers::Renewed,
        ),
        (
            Quirks::ntfs_3g,
            Probed {
                no_replace: Some(true),
                hard_links: Some(true),
                rename_over_empty_directory: Some(true),
                links_keep_numbers: true,
            },
            Feasibility::Yes,
            Feasibility::Yes,
            InodeNumbers::Kept,
        ),
    ];
    for ((quirks, expected, files, folders, numbers), learned_first) in cases
        .into_iter()
        .flat_map(|case| [false, true].map(|learned_first| (case, learned_first)))
    {
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
        assert_eq!(
            can_set_aside(fs(), parent.as_fd(), false),
            Ok(files),
            "{row}"
        );
        assert_eq!(
            can_set_aside(fs(), parent.as_fd(), true),
            Ok(folders),
            "{row}"
        );
        links_hold(fs(), parent.as_fd());
        assert_eq!(
            known_mount(fs(), parent.as_fd()).1.identity.numbers,
            numbers,
            "{row}"
        );
        assert!(names(root.path())?.is_empty(), "{:?}", names(root.path())?);
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
