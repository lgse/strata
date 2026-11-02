// SPDX-License-Identifier: MIT

use super::*;

#[derive(Clone, Copy, Debug)]
enum SourceMeanwhile {
    Closes,
    IsSwapped,
    StaysOpen(Option<&'static str>),
    DeletedBefore { closes: bool },
}

/// A mount, and what happens to the moved folder whose `open.txt` another
/// program holds open.
struct HeldOpen {
    mount: &'static str,
    preset: Preset,
    meanwhile: SourceMeanwhile,
}

impl HeldOpen {
    fn row(&self) -> String {
        format!("{}, {:?}", self.mount, self.meanwhile)
    }

    fn holder(&self) -> Option<Holder> {
        match self.meanwhile {
            SourceMeanwhile::StaysOpen(Some(program)) => Some(Holder::Program(program.to_owned())),
            _ => None,
        }
    }

    /// After the folder's unlink: it is swapped for another, or the hold on
    /// its file is released.
    fn racer<T: Send + 'static>(
        &self,
        held: &Arc<std::sync::Mutex<Option<T>>>,
        live: PathBuf,
        aside: PathBuf,
    ) -> Option<Racer> {
        let release = held.clone();
        match self.meanwhile {
            SourceMeanwhile::IsSwapped => Some(Box::new(move || {
                fs::rename(&live, &aside).expect("move the folder");
                fs::create_dir(&live).expect("make another folder");
            })),
            SourceMeanwhile::Closes | SourceMeanwhile::DeletedBefore { closes: true } => {
                Some(Box::new(move || {
                    drop(release.lock().expect("the hold").take());
                }))
            }
            SourceMeanwhile::StaysOpen(_) | SourceMeanwhile::DeletedBefore { closes: false } => {
                None
            }
        }
    }

    /// What the move left at the source and reported kept.
    fn assert_left(
        &self,
        folder: &Path,
        moved: &Path,
        report: &TransferReport,
    ) -> Result<(), Box<dyn Error>> {
        let row = self.row();
        match self.meanwhile {
            SourceMeanwhile::Closes | SourceMeanwhile::DeletedBefore { closes: true } => {
                assert!(!folder.exists(), "{row}");
                assert!(report.kept.is_empty(), "{row}: {:?}", report.kept);
            }
            SourceMeanwhile::IsSwapped => {
                let changed = [kept(folder.to_path_buf(), KeptCause::Changed)];
                assert_eq!(report.kept, changed, "{row}");
                assert!(names(folder)?.is_empty(), "{row}");
                assert!(moved.is_dir(), "{row}");
            }
            SourceMeanwhile::DeletedBefore { closes: false } => {
                let still_open = [kept(folder.to_path_buf(), KeptCause::StillOpen(None))];
                assert_eq!(report.kept, still_open, "{row}");
                let [leftover] = names(folder)?.try_into().map_err(|_| "one leftover")?;
                assert!(
                    crate::model::hidden_names::foreign_leftover(OsStr::new(&leftover)).is_some(),
                    "{row}: {leftover}"
                );
            }
            SourceMeanwhile::StaysOpen(_) => {
                let cause = match (self.mount, self.holder()) {
                    ("cifs-posix", None) => KeptCause::Undeletable(Errno::ACCESS.raw_os_error()),
                    (_, holder) => KeptCause::StillOpen(holder),
                };
                assert_eq!(report.kept, [kept(folder.to_path_buf(), cause)], "{row}");
                assert!(folder.is_dir(), "{row}");
            }
        }
        Ok(())
    }
}

/// Unlinks `open.txt` of `folder` through the filesystem seam while it is
/// still held open.
fn unlink_held(folder: &Path) -> Result<(), Box<dyn Error>> {
    let dir = open_local_parent_directory(folder)?;
    super::super::super::fs_ops::fs().unlink(
        super::super::super::fs_ops::Entry::new(&dir, OsStr::new("open.txt")),
        rustix::fs::AtFlags::empty(),
    )?;
    Ok(())
}

fn check_held_open(case: &HeldOpen) -> Result<(), Box<dyn Error>> {
    let row = case.row();
    let _clock = ClockGuard::set(SystemTime::now());
    let root = tempfile::tempdir()?;
    let source_dir = root.path().join("source");
    let folder = source_dir.join("folder");
    let moved = source_dir.join("folder.moved");
    fs::create_dir_all(&folder)?;
    fs::write(folder.join("open.txt"), b"open elsewhere")?;
    fs::write(folder.join("other.txt"), b"other")?;
    let held = Arc::new(std::sync::Mutex::new(None));
    let racer = case.racer(&held, folder.clone(), moved.clone());
    let _source = FsQuirks::install(
        &source_dir,
        Quirks {
            after: racer
                .map(|racer| (Op::Unlink, "folder", 0, racer))
                .into_iter()
                .collect(),
            holder: case.holder(),
            ..(case.preset)()
        },
    );
    *held.lock().map_err(|_| "the hold")? = Some(hold(&folder.join("open.txt")));
    let deleted_before = matches!(case.meanwhile, SourceMeanwhile::DeletedBefore { .. });
    if deleted_before {
        unlink_held(&folder)?;
    }
    let destination = root.path().join("destination");
    fs::create_dir_all(&destination)?;
    let _destination = FsQuirks::install(&destination, Quirks::default());

    let (_operation, events) =
        paste_move(1590, &destination, &folder, TransferConflict::FailIfExists);
    pump_until_transfer(&events);

    assert!(
        matches!(
            terminal_transfer(&events.borrow()),
            Some(OperationEvent::Pasted { .. })
        ),
        "{row}"
    );
    let copied = destination.join("folder");
    if deleted_before {
        assert_eq!(names(&copied)?, ["other.txt"], "{row}");
    } else {
        assert_eq!(
            fs::read(copied.join("open.txt"))?,
            b"open elsewhere",
            "{row}"
        );
    }
    case.assert_left(&folder, &moved, &reported(&events))?;
    drop(held.lock().map_err(|_| "the hold")?.take());
    Ok(())
}

#[test]
fn a_moved_folder_goes_once_another_program_closes_its_file_within_the_mount_s_wait()
-> Result<(), Box<dyn Error>> {
    use SourceMeanwhile::*;
    let _serial = serial()?;
    let held = |mount, preset: Preset, meanwhile| HeldOpen {
        mount,
        preset,
        meanwhile,
    };
    let rows = [
        held("nfs42", Quirks::nfs42, Closes),
        held("nfs42", Quirks::nfs42, IsSwapped),
        held("nfs42", Quirks::nfs42, StaysOpen(None)),
        held("nfs42", Quirks::nfs42, StaysOpen(Some("Text Editor"))),
        held("cifs-posix", Quirks::cifs_posix, StaysOpen(None)),
        held(
            "cifs-posix",
            Quirks::cifs_posix,
            StaysOpen(Some("Text Editor")),
        ),
        held("nfs42", Quirks::nfs42, DeletedBefore { closes: false }),
        held("nfs42", Quirks::nfs42, DeletedBefore { closes: true }),
        held("sshfs", Quirks::sshfs, DeletedBefore { closes: false }),
    ];
    for case in &rows {
        check_held_open(case)?;
    }
    Ok(())
}
