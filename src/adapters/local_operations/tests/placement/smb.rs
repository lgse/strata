// SPDX-License-Identifier: MIT

use super::*;

#[derive(Clone, Copy, Debug)]
enum Renaming {
    Rename,
    Move,
    Trash,
}

fn rename_move_or_trash(
    renaming: Renaming,
    item: &Path,
    cancellable: &gio::Cancellable,
) -> Result<(), glib::Error> {
    let context = glib::MainContext::default();
    let parent = item.parent().unwrap_or(item);
    match renaming {
        Renaming::Rename => context.block_on(super::super::super::rename_native(
            item.to_path_buf(),
            parent.join("renamed"),
            cancellable,
        )),
        Renaming::Move => context
            .block_on(move_local(
                gio::File::for_path(item),
                gio::File::for_path(parent.join("elsewhere").join("moved")),
                Taken::Fail,
                cancellable.clone(),
                None,
                NamePolicy::default(),
            ))
            .map(drop),
        Renaming::Trash => context
            .block_on(super::super::super::trash::trash_entry(
                &gio::File::for_path(item),
                cancellable,
                &super::super::super::Deadline::default(),
            ))
            .map(drop),
    }
}

#[derive(Clone, Copy, Debug)]
enum Below {
    ClosesDuringTheWait,
    StaysOpen(Option<&'static str>),
    WaitCancelled,
    FailsOtherwiseWhileCancelled,
    OnAPosixShare,
}

#[test]
fn an_smb_folder_refused_while_a_file_below_closes_is_renamed_moved_or_trashed_once_it_has()
-> Result<(), Box<dyn Error>> {
    use Below::*;
    use Renaming::*;
    let _serial = serial()?;
    let closetimeo = Duration::from_secs(1);
    let budget = Duration::from_millis(1500);
    for (renaming, below) in [
        (Rename, ClosesDuringTheWait),
        (Move, ClosesDuringTheWait),
        (Trash, ClosesDuringTheWait),
        (Rename, StaysOpen(Some("Text Editor"))),
        (Move, StaysOpen(Some("Text Editor"))),
        (Trash, StaysOpen(Some("Text Editor"))),
        (Rename, StaysOpen(None)),
        (Trash, StaysOpen(None)),
        (Move, WaitCancelled),
        (Rename, FailsOtherwiseWhileCancelled),
        (Move, FailsOtherwiseWhileCancelled),
        (Trash, FailsOtherwiseWhileCancelled),
        (Rename, OnAPosixShare),
        (Move, OnAPosixShare),
        (Trash, OnAPosixShare),
    ] {
        let row = format!("{renaming:?}, {below:?}");
        let start = SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000);
        let _clock = ClockGuard::set(start);
        let root = tempfile::tempdir()?;
        let folder = root.path().join("folder");
        fs::create_dir_all(&folder)?;
        fs::create_dir(root.path().join("elsewhere"))?;
        fs::write(folder.join("open.txt"), b"open")?;
        let cancellable = gio::Cancellable::new();
        let held = Arc::new(std::sync::Mutex::new(None));
        let racer: Option<Racer> = match below {
            ClosesDuringTheWait => {
                let held = held.clone();
                Some(Box::new(move || {
                    drop(held.lock().expect("the hold").take())
                }))
            }
            WaitCancelled | FailsOtherwiseWhileCancelled => {
                let cancellable = cancellable.clone();
                Some(Box::new(move || cancellable.cancel()))
            }
            StaysOpen(_) | OnAPosixShare => None,
        };
        let refused_by = match renaming {
            Rename | Move => Op::Rename,
            Trash => Op::Trash,
        };
        let holder = match below {
            StaysOpen(program) => program.map(|program| Holder::Program(program.to_owned())),
            OnAPosixShare => Some(Holder::Program("Text Editor".to_owned())),
            _ => None,
        };
        let (preset, before, after, fail): (Preset, _, _, _) = match below {
            FailsOtherwiseWhileCancelled => (
                Quirks::cifs,
                racer.map(|racer| (refused_by, "folder", 0, racer)),
                None,
                vec![(refused_by, "folder", 0, Errno::IO)],
            ),
            OnAPosixShare => (Quirks::cifs_posix, None, None, Vec::new()),
            _ => (
                Quirks::cifs,
                None,
                racer.map(|racer| (refused_by, "folder", 0, racer)),
                Vec::new(),
            ),
        };
        let _scope = FsQuirks::install(
            root.path(),
            Quirks {
                folder_renames_refused_below: Some(closetimeo),
                holder,
                before: before.into_iter().collect(),
                after: after.into_iter().collect(),
                fail,
                ..preset()
            },
        );
        *held.lock().map_err(|_| "the hold")? = Some(hold(&folder.join("open.txt")));

        let result = rename_move_or_trash(renaming, &folder, &cancellable);

        let waited = super::super::super::copy_engine::clock_for_test().duration_since(start)?;
        match below {
            ClosesDuringTheWait => {
                assert!(result.is_ok(), "{row}: {result:?}");
                assert!(!folder.exists(), "{row}");
                assert!(closetimeo <= waited && waited < budget, "{row}: {waited:?}");
            }
            StaysOpen(program) => {
                let error = result.err().ok_or(row.clone())?;
                assert!(folder.is_dir(), "{row}");
                assert_eq!(waited, budget, "{row}");
                let still_open = program.map(|program| {
                    rust_i18n::t!(
                        "A file in it is still open in %{program}. Close it there and try again.",
                        program = program
                    )
                    .into_owned()
                });
                match (renaming, still_open) {
                    (Move, Some(reason)) => assert_eq!(
                        error.message(),
                        rust_i18n::t!(
                            "Could not move %{name}: %{error}",
                            name = "folder",
                            error = reason
                        ),
                        "{row}"
                    ),
                    (_, Some(reason)) => assert_eq!(error.message(), reason, "{row}"),
                    (Rename, None) => assert_eq!(
                        error.message(),
                        crate::services::io_error_message(&io::Error::from_raw_os_error(
                            Errno::ACCESS.raw_os_error()
                        )),
                        "{row}"
                    ),
                    (_, None) => assert!(
                        error.matches(gio::IOErrorEnum::PermissionDenied),
                        "{row}: {error:?}"
                    ),
                }
            }
            WaitCancelled => {
                let error = result.err().ok_or(row.clone())?;
                assert!(
                    error.matches(gio::IOErrorEnum::Cancelled),
                    "{row}: {error:?}"
                );
                assert!(folder.is_dir(), "{row}");
                assert!(waited < budget, "{row}: {waited:?}");
            }
            FailsOtherwiseWhileCancelled | OnAPosixShare => {
                let error = result.err().ok_or(row.clone())?;
                assert!(folder.is_dir(), "{row}");
                assert_eq!(waited, Duration::ZERO, "{row}");
                let errno = match below {
                    OnAPosixShare => Errno::ACCESS,
                    _ => Errno::IO,
                };
                let detail = crate::services::io_error_detail(&io::Error::from_raw_os_error(
                    errno.raw_os_error(),
                ));
                match renaming {
                    Trash => assert!(
                        error.matches(gio::io_error_from_errno(errno.raw_os_error())),
                        "{row}: {error:?}"
                    ),
                    Rename | Move => assert!(error.message().contains(&detail), "{row}: {error:?}"),
                }
            }
        }
        drop(held.lock().map_err(|_| "the hold")?.take());
    }
    Ok(())
}

#[test]
fn an_smb_item_refused_as_busy_is_reported_as_still_open_elsewhere() -> Result<(), Box<dyn Error>> {
    use Renaming::*;
    let _serial = serial()?;
    let busy: Preset = Quirks::cifs;
    let local: Preset = Quirks::ext4;
    for (renaming, preset, smb) in [
        (Rename, busy, true),
        (Move, busy, true),
        (Trash, busy, true),
        (Rename, local, false),
    ] {
        let row = format!("{renaming:?}, SMB: {smb}");
        let root = tempfile::tempdir()?;
        let item = root.path().join("notes.txt");
        fs::write(&item, b"notes")?;
        fs::create_dir(root.path().join("elsewhere"))?;
        let refused_by = match renaming {
            Rename | Move => Op::Rename,
            Trash => Op::Trash,
        };
        let _scope = FsQuirks::install(
            root.path(),
            Quirks {
                fail: vec![(refused_by, "notes.txt", 0, Errno::BUSY)],
                ..preset()
            },
        );

        let error = rename_move_or_trash(renaming, &item, &gio::Cancellable::new())
            .err()
            .ok_or(row.clone())?;

        assert_eq!(fs::read(&item)?, b"notes", "{row}");
        let still_open =
            crate::i18n::tr("It is still open in a program on this or another computer.");
        let reason = match (renaming, smb) {
            (Move, true) => rust_i18n::t!(
                "Could not move %{name}: %{error}",
                name = "notes.txt",
                error = still_open
            )
            .into_owned(),
            (_, true) => still_open,
            (_, false) => crate::services::io_error_message(&io::Error::from_raw_os_error(
                Errno::BUSY.raw_os_error(),
            )),
        };
        assert_eq!(error.message(), reason, "{row}");
    }
    Ok(())
}
