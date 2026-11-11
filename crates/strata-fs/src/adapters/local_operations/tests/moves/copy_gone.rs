// SPDX-License-Identifier: MIT

use super::*;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Replaced {
    No,
    Yes,
    /// Same size, written a tick after the source: how a file reusing the
    /// copy's inode within its birth tick looks on ntfs3.
    InPlace,
    /// So within the tick of a coarse clock that stamped the source too.
    InPlaceWithinTheTick,
    /// So within a tick of ntfs3's clock, which stays coarse after Linux
    /// 6.13, from a source with multigrain times.
    InPlaceWithinTheTickOnNtfs3,
}

impl Replaced {
    fn on_ntfs3(self) -> bool {
        self == Replaced::InPlaceWithinTheTickOnNtfs3
    }

    fn within_the_tick(self) -> bool {
        self.on_ntfs3() || self == Replaced::InPlaceWithinTheTick
    }

    /// Removes the copy, puts another file in its place, or both.
    fn racer(self, copy: PathBuf) -> Racer {
        Box::new(move || {
            if matches!(self, Replaced::No | Replaced::Yes) {
                fs::remove_file(&copy).expect("remove the copy");
            }
            if self != Replaced::No {
                fs::write(&copy, b"other").expect("put another file there");
            }
        })
    }
}

/// The destination's quirks, with `racer` after the call that publishes
/// the copy, or before the barrier.
fn publishing(
    replaced: Replaced,
    published: Option<Op>,
    coarse_clock: Option<SystemTime>,
    racer: Racer,
) -> Quirks {
    let mut quirks = Quirks {
        trash: super::super::fs_quirks::Trash::Unsupported,
        coarse_clock,
        ..if replaced.on_ntfs3() {
            Quirks::ntfs3()
        } else {
            Quirks::ext4()
        }
    };
    let Some(op) = published else {
        quirks.before.push((Op::SyncFs, "*", 0, racer));
        return quirks;
    };
    quirks.after.push((op, "notes.txt", 0, racer));
    if op == Op::Rename {
        quirks.tmpfile = Support::Rejected(Errno::OPNOTSUPP);
    }
    quirks
}

fn check_copy_gone(
    conflict: TransferConflict,
    published: Option<Op>,
    replaced: Replaced,
) -> Result<(), Box<dyn Error>> {
    let row = format!("{conflict:?}, published: {published:?}, replaced: {replaced:?}");
    let within_the_tick = replaced.within_the_tick();
    // Before the kernel's coarse clock, which may run a tick behind.
    let tick = SystemTime::now() - Duration::from_secs(1);
    let _clock = within_the_tick.then(|| ClockGuard::set(tick));
    let kernel = if replaced.on_ntfs3() {
        (6, 13, 0)
    } else {
        (6, 12, 0)
    };
    let _kernel = within_the_tick.then(|| KernelGuard::set(Some(kernel)));
    let coarse_clock = within_the_tick.then_some(tick);
    let root = tempfile::tempdir()?;
    let sources = root.path().join("sources");
    let destination = root.path().join("destination");
    fs::create_dir(&sources)?;
    fs::create_dir(&destination)?;
    let source = sources.join("notes.txt");
    fs::write(&source, b"notes")?;
    if replaced == Replaced::InPlace {
        set_times(&source, an_hour_ago()?, 0)?;
    }
    if conflict == TransferConflict::ReplaceExisting {
        fs::write(destination.join("notes.txt"), b"old")?;
    }
    let racer = replaced.racer(destination.join("notes.txt"));
    let _sources = FsQuirks::install(
        &sources,
        Quirks {
            coarse_clock,
            ..Quirks::ext4()
        },
    );
    let quirks = publishing(replaced, published, coarse_clock, racer);
    let _destination = FsQuirks::install(&destination, quirks);

    let (_operation, events) = paste_move(98, &destination, &source, conflict);
    pump_until_transfer(&events);

    assert_pasted(&events, &row);
    assert_eq!(fs::read(&source).ok(), Some(b"notes".to_vec()), "{row}");
    assert_eq!(
        reported(&events).kept,
        [kept(source.clone(), KeptCause::CopyGone)],
        "{row}"
    );
    Ok(())
}

#[test]
fn a_moved_source_stays_when_its_copy_is_gone_before_the_deletion() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let rows = [
        (TransferConflict::FailIfExists, None),
        (TransferConflict::ReplaceExisting, None),
        (TransferConflict::FailIfExists, Some(Op::LinkFd)),
        (TransferConflict::FailIfExists, Some(Op::Rename)),
        (TransferConflict::ReplaceExisting, Some(Op::Exchange)),
    ]
    .into_iter()
    .flat_map(|(conflict, published)| {
        [Replaced::No, Replaced::Yes].map(|replaced| (conflict, published, replaced))
    })
    .chain(
        [
            Replaced::InPlace,
            Replaced::InPlaceWithinTheTick,
            Replaced::InPlaceWithinTheTickOnNtfs3,
        ]
        .map(|replaced| (TransferConflict::FailIfExists, None, replaced)),
    );
    for (conflict, published, replaced) in rows {
        if published == Some(Op::LinkFd) && !base_offers(&UNNAMED_FILES)? {
            continue;
        }
        check_copy_gone(conflict, published, replaced)?;
    }
    Ok(())
}
