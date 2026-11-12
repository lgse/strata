// SPDX-License-Identifier: MIT

use super::*;

// FAT, exFAT, sshfs and HFS+ keep coarse times, so a rewrite of the same
// size within one tick shows neither in the modification time nor in the
// change time, which they lack or keep as coarse.

fn vfat() -> Quirks {
    Quirks {
        ctime_is_mtime: true,
        ..Quirks::vfat()
    }
}

fn exfat_fuse() -> Quirks {
    Quirks {
        ctime_is_mtime: true,
        ..Quirks::exfat_fuse()
    }
}

/// exfat-fuse on a device udev has no record of.
fn fuseblk() -> Quirks {
    Quirks {
        device_format: None,
        ..exfat_fuse()
    }
}

/// exfat-fuse on a device udev cannot read.
fn udev_failed() -> Quirks {
    Quirks {
        fail: vec![(Op::DeviceFormat, "*", 0, Errno::ACCESS)],
        ..exfat_fuse()
    }
}

fn sshfs() -> Quirks {
    Quirks {
        ctime_is_mtime: true,
        ..Quirks::sshfs()
    }
}

fn hfsplus() -> Quirks {
    Quirks {
        mount_type: Some(("hfsplus", "rw")),
        ctime_is_mtime: true,
        ..Quirks::default()
    }
}

/// An SMB share of a FAT disk, whose times only their lack of a fraction
/// of a second tells coarse.
fn fat_share() -> Quirks {
    Quirks {
        ctime_is_mtime: true,
        ..Quirks::cifs()
    }
}

/// Reads of four bytes, so the copy and the comparison read in chunks.
fn chunked() -> Quirks {
    Quirks {
        short_io: Some(4),
        ctime_is_mtime: true,
        ..Quirks::vfat()
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Rewrite {
    Never,
    Deleting,
    Comparing,
    Compared,
    Grown,
    RenamedOver,
    Reached,
    ReachedCompared,
    ReachedAtStat,
    ChunkCompared,
    ClockSetBack,
}

/// The source `a.txt` of a moved folder, stamped at whole two-second steps
/// with the fraction its mount keeps.
struct Stamped {
    folder: PathBuf,
    file: PathBuf,
    seconds: u64,
    fraction: u32,
}

/// The kernel's coarse clock, a tick behind the engine's.
fn kernel_clock() -> Duration {
    (super::super::super::copy_engine::clock_for_test() - Duration::from_millis(10))
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("a time after 1970")
}

impl Stamped {
    /// A rewrite keeps the file's times, or, `clocked`, takes the kernel's
    /// coarse clock in FAT's two-second steps.
    fn rewriter(&self, contents: &'static [u8], clocked: bool) -> Racer {
        let (path, seconds, fraction) = (self.file.clone(), self.seconds, self.fraction);
        Box::new(move || {
            fs::write(&path, contents).expect("rewrite the source");
            let (step, fraction) = if clocked {
                (kernel_clock().as_secs() / 2 * 2, 0)
            } else {
                (seconds, fraction)
            };
            set_times(&path, step, fraction).expect("set the times");
        })
    }

    /// Moves the engine's clock past the file's tick, then runs `then`.
    fn reached(&self, mut then: Racer) -> Racer {
        let seconds = self.seconds;
        Box::new(move || {
            let after =
                SystemTime::UNIX_EPOCH + Duration::from_secs(seconds) + Duration::from_millis(500);
            super::super::super::copy_engine::set_clock_for_test(Some(after));
            then();
        })
    }

    fn rename_over(&self) -> Racer {
        let (path, saved) = (self.file.clone(), self.folder.join(".a.txt.save"));
        let (seconds, fraction) = (self.seconds, self.fraction);
        Box::new(move || {
            fs::write(&saved, b"ORIGINAL").expect("save the new version aside");
            set_times(&saved, seconds, fraction).expect("set the times");
            fs::rename(&saved, &path).expect("rename it over the source");
        })
    }

    fn rewrite_chunk(&self) -> Racer {
        let path = self.file.clone();
        Box::new(move || {
            let source = fs::OpenOptions::new().write(true).open(&path);
            std::os::unix::fs::FileExt::write_at(&source.expect("open"), b"ORIG", 0)
                .expect("rewrite the first chunk");
            set_times(&path, kernel_clock().as_secs() / 2 * 2, 0).expect("set the times");
        })
    }

    fn set_back(&self) -> Racer {
        let seconds = self.seconds;
        Box::new(move || {
            let hour_before = SystemTime::UNIX_EPOCH + Duration::from_secs(seconds - 3600);
            super::super::super::copy_engine::set_clock_for_test(Some(hour_before));
        })
    }

    /// Whether the move selects the file itself, and the racers before and
    /// after calls that make `rewrite` happen.
    fn racers(&self, rewrite: Rewrite) -> (bool, Vec<Before>, Vec<Before>) {
        let deleting = |racer| vec![(Op::OpenDir, "folder", 1, racer)];
        match rewrite {
            Rewrite::Never => (false, Vec::new(), Vec::new()),
            Rewrite::Deleting => (
                false,
                deleting(self.rewriter(b"ORIGINAL", false)),
                Vec::new(),
            ),
            Rewrite::Comparing => {
                let racer = self.rewriter(b"ORIGINAL", false);
                (true, vec![(Op::OpenSource, "a.txt", 1, racer)], Vec::new())
            }
            Rewrite::Compared => {
                let racer = self.rewriter(b"ORIGINAL", true);
                (false, vec![(Op::Stat, "a.txt", 0, racer)], Vec::new())
            }
            Rewrite::Grown => {
                let before = vec![
                    (
                        Op::Read,
                        "a.txt",
                        0,
                        self.rewriter(b"original, grown", false),
                    ),
                    (
                        Op::OpenDir,
                        "folder",
                        1,
                        self.rewriter(b"ORIGINAL, GROWN", false),
                    ),
                ];
                (false, before, Vec::new())
            }
            Rewrite::RenamedOver => {
                let after = vec![(Op::OpenSource, "a.txt", 1, self.rename_over())];
                (false, Vec::new(), after)
            }
            Rewrite::Reached => {
                let racer = self.reached(self.rewriter(b"ORIGINAL", false));
                (false, deleting(racer), Vec::new())
            }
            Rewrite::ReachedCompared => {
                let before = vec![
                    (Op::OpenDir, "folder", 1, self.reached(Box::new(|| {}))),
                    (Op::Stat, "a.txt", 0, self.rewriter(b"ORIGINAL", true)),
                ];
                (false, before, Vec::new())
            }
            Rewrite::ReachedAtStat => {
                let racer = self.reached(self.rewriter(b"ORIGINAL", false));
                (false, vec![(Op::Stat, "a.txt", 0, racer)], Vec::new())
            }
            // The copy reads two chunks and the end, then the comparison its
            // first chunk.
            Rewrite::ChunkCompared => {
                let after = vec![(Op::Read, "a.txt", 3, self.rewrite_chunk())];
                (false, Vec::new(), after)
            }
            Rewrite::ClockSetBack => (false, deleting(self.set_back()), Vec::new()),
        }
    }
}

impl Stamped {
    /// What the deletion found at the source: gone, or kept as changed with
    /// the contents `rewrite` left.
    fn assert_left(
        &self,
        rewrite: Rewrite,
        item: &Path,
        report: &TransferReport,
        row: &str,
    ) -> io::Result<()> {
        if rewrite == Rewrite::Never {
            assert!(!item.exists(), "{row}");
            assert_eq!(report.kept, [], "{row}");
            return Ok(());
        }
        let left: &[u8] = match rewrite {
            Rewrite::ClockSetBack => b"original",
            Rewrite::ChunkCompared => b"ORIGinal",
            Rewrite::Grown => b"ORIGINAL, GROWN",
            _ => b"ORIGINAL",
        };
        assert_eq!(fs::read(&self.file)?, left, "{row}");
        assert_eq!(
            report.kept,
            [kept(self.file.clone(), KeptCause::Changed)],
            "{row}"
        );
        Ok(())
    }
}

/// Whether the deletion reopened the source and read it again; a file
/// renamed over reads as deleted through the descriptor.
fn read_again(calls: &[super::super::fs_quirks::Call]) -> bool {
    let opened: Vec<usize> = calls
        .iter()
        .enumerate()
        .filter(|(_, call)| call.op == Op::OpenSource && call.names == ["a.txt"])
        .map(|(index, _)| index)
        .collect();
    let reads_source = |call: &super::super::fs_quirks::Call| {
        call.op == Op::Read && (call.names == ["a.txt"] || call.names == ["a.txt (deleted)"])
    };
    opened
        .get(1)
        .is_some_and(|&reopened| calls[reopened..].iter().any(reads_source))
}

/// A row, the mount, how many milliseconds after its modification time the
/// copy reads the file (negative: dated ahead of the clock), how it is
/// rewritten, and whether the deletion reads it again.
type CoarseRow = (&'static str, Preset, i64, Rewrite, bool);

fn check_coarse_tick(
    (row, preset, read_after, rewrite, expected_read_again): CoarseRow,
) -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let folder = root.path().join("source/folder");
    fs::create_dir_all(&folder)?;
    let file = folder.join("a.txt");
    fs::write(&file, b"original")?;
    let epoch = SystemTime::UNIX_EPOCH;
    let seconds = SystemTime::now().duration_since(epoch)?.as_secs() / 2 * 2;
    // ext4 keeps nanoseconds and exFAT hundredths of a second; FAT,
    // sshfs and HFS+ keep whole seconds.
    let fraction = match preset().mount_type {
        Some(("ext4", _)) => 250_000_000,
        Some(("fuseblk", _)) => 500_000_000,
        _ => 0,
    };
    set_times(&file, seconds, fraction)?;
    let read_at = i64::try_from(seconds)? * 1000 + read_after;
    let _clock = ClockGuard::set(epoch + Duration::from_millis(u64::try_from(read_at)?));
    let stamped = Stamped {
        folder,
        file,
        seconds,
        fraction,
    };
    let (selects_file, before, after) = stamped.racers(rewrite);
    let item = if selects_file {
        &stamped.file
    } else {
        &stamped.folder
    };
    let quirks = Quirks {
        before,
        after,
        ..preset()
    };

    let (moved, report, _, calls) = move_across_volumes_logged(item, quirks)?;

    let copied = if selects_file {
        moved
    } else {
        moved.join("a.txt")
    };
    let original: &[u8] = if rewrite == Rewrite::Grown {
        b"original, grown"
    } else {
        b"original"
    };
    assert_eq!(fs::read(copied)?, original, "{row}");
    stamped.assert_left(rewrite, item, &report, row)?;
    assert_eq!(read_again(&calls), expected_read_again, "{row}");
    Ok(())
}

#[test]
fn files_read_within_a_coarse_time_tick_are_compared_before_their_move_deletes_them()
-> Result<(), Box<dyn Error>> {
    use Rewrite::{
        ChunkCompared, ClockSetBack, Compared, Comparing, Deleting, Grown, Never, Reached,
        ReachedAtStat, ReachedCompared, RenamedOver,
    };
    let _serial = serial()?;
    const RECENT: i64 = 500;
    const PAST: i64 = 2050;
    const BEHIND: i64 = 2500;
    const LONG_AGO: i64 = 3_600_000;
    const AHEAD: i64 = -3_600_000;
    let rows: [CoarseRow; 24] = [
        ("vfat", vfat, RECENT, Deleting, true),
        ("vfat, past the tick", vfat, PAST, Deleting, true),
        ("exfat-fuse", exfat_fuse, RECENT, Deleting, true),
        ("fuseblk, no record", fuseblk, RECENT, Deleting, true),
        ("fuseblk, udev failed", udev_failed, RECENT, Deleting, true),
        (
            "exfat-fuse, renamed over",
            exfat_fuse,
            RECENT,
            RenamedOver,
            true,
        ),
        ("sshfs", sshfs, RECENT, Deleting, true),
        ("sshfs, behind", sshfs, BEHIND, Deleting, true),
        ("hfsplus", hfsplus, RECENT, Deleting, true),
        ("FAT share", fat_share, RECENT, Deleting, true),
        ("vfat, file", vfat, RECENT, Comparing, true),
        ("vfat, compared", vfat, RECENT, Compared, true),
        ("vfat, grown", vfat, RECENT, Grown, true),
        ("vfat, unchanged", vfat, RECENT, Never, true),
        ("vfat, in chunks", chunked, RECENT, Never, true),
        ("vfat, chunk compared", chunked, RECENT, ChunkCompared, true),
        ("vfat, clock set back", vfat, RECENT, ClockSetBack, false),
        ("vfat, long before", vfat, LONG_AGO, Never, false),
        ("vfat, ahead", vfat, AHEAD, Never, false),
        ("vfat, ahead, reached", vfat, AHEAD, Reached, true),
        ("vfat, ahead, compared", vfat, AHEAD, ReachedCompared, true),
        (
            "vfat, ahead, at the stat",
            vfat,
            AHEAD,
            ReachedAtStat,
            false,
        ),
        ("sshfs, ahead", sshfs, AHEAD, Never, true),
        ("ext4", Quirks::ext4, RECENT, Never, false),
    ];
    for row in rows {
        check_coarse_tick(row)?;
    }
    Ok(())
}
