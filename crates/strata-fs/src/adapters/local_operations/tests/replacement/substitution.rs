// SPDX-License-Identifier: MIT

//! Items another process substitutes while they are replaced are never taken for the ones
//! the replacement checked.

use std::sync::atomic::Ordering;

use super::fs_quirks::{autodisable_serverino, evict};
use super::*;

#[test]
fn replacement_move_does_not_delete_a_substituted_source() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let sources = root.path().join("sources");
    let destination = root.path().join("destination");
    fs::create_dir(&sources)?;
    fs::create_dir(&destination)?;
    let source = sources.join("source.txt");
    let original_source = sources.join("original-source.txt");
    let target = destination.join("target.txt");
    fs::write(&source, b"replacement")?;
    fs::write(&target, b"original target")?;
    let _sources = FsQuirks::install(&sources, Quirks::default());
    let _destination = FsQuirks::install(
        &destination,
        Quirks {
            before: vec![(Op::Write, "*", 0, substitute(&source, &original_source))],
            ..Quirks::default()
        },
    );

    let (result, kept) = replace_collecting(&source, &target, true, |report| report.kept);

    result?;
    expect_contents(&[
        (target, b"replacement"),
        (source.clone(), b"new arrival"),
        (original_source, b"replacement"),
    ]);
    assert_eq!(
        kept,
        [crate::services::KeptItem {
            location: Location::local(&source),
            cause: crate::services::KeptCause::Changed,
        }]
    );
    Ok(())
}

#[test]
fn replacement_stops_before_exchanging_a_substituted_target() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    for (op, name) in [(Op::Clone, "*"), (Op::OpenAt, "target.txt")] {
        let root = tempfile::tempdir()?;
        let source = root.path().join("source.txt");
        let target = root.path().join("target.txt");
        let original_target = root.path().join("original-target.txt");
        fs::write(&source, b"replacement")?;
        fs::write(&target, b"original target")?;
        let scope = FsQuirks::install(
            root.path(),
            Quirks {
                before: vec![(op, name, 0, substitute(&target, &original_target))],
                ..Quirks::default()
            },
        );

        let result = replace(&source, &target, false);

        let family = mount_capabilities(scope.key()).family;
        if op == Op::OpenAt && matches!(family, Some(Family::Nfs | Family::Nfs4)) {
            eprintln!("skipped on an NFS base, where Strata holds no item it trashes: {op:?}");
            continue;
        }
        let error = result.expect_err("a substituted target must fail identity validation");
        assert!(error.to_string().contains("changed"), "{op:?}: {error}");
        expect_contents(&[
            (target, b"new arrival"),
            (original_target, b"original target"),
        ]);
        let listed = ["original-target.txt", "source.txt", "target.txt"];
        expect_names(root.path(), &listed, &format!("{op:?}"))?;
    }
    Ok(())
}

/// What happens to the target while its replacement is read.
#[derive(Clone, Copy, Debug)]
enum Meanwhile {
    Evicted,
    StageEvicted,
    ReplacedAlike,
    ReplacedLonger,
    EvictedOnceNumbersWent,
    ReplacedAlikeOnceNumbersWent,
    ReplacedAlikeAndEvictedOnceNumbersWent,
}

impl Meanwhile {
    fn theirs(self) -> &'static [u8] {
        match self {
            Meanwhile::ReplacedLonger => b"OLD, longer",
            _ => b"OLD",
        }
    }

    fn told_by_change_time(self) -> bool {
        matches!(
            self,
            Meanwhile::ReplacedAlikeOnceNumbersWent
                | Meanwhile::ReplacedAlikeAndEvictedOnceNumbersWent
        )
    }

    fn stage_evicted(self) -> bool {
        matches!(self, Meanwhile::StageEvicted)
    }

    /// Does this to the target at `share`, recording in `done` whether it could.
    fn racer(self, share: PathBuf, arrival: PathBuf, done: Arc<AtomicBool>) -> Racer {
        Box::new(move || {
            let target = share.join("target");
            let numbers_went = || autodisable_serverino(&share);
            let renamed_over = || fs::rename(&arrival, &target).is_ok();
            let evicted = || evict(&target);
            let happened = match self {
                Meanwhile::Evicted => evicted(),
                Meanwhile::StageEvicted => return,
                Meanwhile::ReplacedAlike | Meanwhile::ReplacedLonger => renamed_over(),
                Meanwhile::EvictedOnceNumbersWent => numbers_went() && evicted(),
                Meanwhile::ReplacedAlikeOnceNumbersWent => numbers_went() && renamed_over(),
                Meanwhile::ReplacedAlikeAndEvictedOnceNumbersWent => {
                    numbers_went() && renamed_over() && evicted()
                }
            };
            done.store(happened, Ordering::Release);
        })
    }
}

/// Evicts every stage in `share` as it is closed, recording in `done` whether one was.
fn stage_evictions(share: &Path, done: &Arc<AtomicBool>) -> Vec<(Op, &'static str, usize, Racer)> {
    (0..4)
        .map(|skip| {
            let (at, done) = (share.to_path_buf(), done.clone());
            let racer: Racer = Box::new(move || {
                let stages = names(&at).expect("list the share");
                let evicted = stages
                    .iter()
                    .filter(|name| name.starts_with(".strata-copy-"))
                    .any(|stage| evict(&at.join(stage)));
                done.fetch_or(evicted, Ordering::AcqRel);
            });
            (Op::Close, ".strata-copy-*", skip, racer)
        })
        .collect()
}

/// A share holding `source` and `target`, beside an `arrival` an hour as old as the target.
struct Share {
    _root: tempfile::TempDir,
    share: PathBuf,
    source: PathBuf,
    target: PathBuf,
    arrival: PathBuf,
    hour_before: u64,
    meanwhile: Meanwhile,
    row: String,
}

impl Share {
    fn new(preset: Preset, meanwhile: Meanwhile) -> Result<Self, Box<dyn Error>> {
        let row = format!("{:?}, {meanwhile:?}", preset().mount_type);
        let root = tempfile::tempdir()?;
        let share = root.path().join("share");
        fs::create_dir(&share)?;
        let (source, target) = (share.join("source"), share.join("target"));
        let arrival = root.path().join("arrival");
        fs::write(&source, b"new")?;
        fs::write(&target, b"old")?;
        fs::write(&arrival, meanwhile.theirs())?;
        let hour_before = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)?
            .as_secs()
            - 3600;
        for file in [&target, &arrival] {
            set_times(file, hour_before, 0)?;
        }
        Ok(Self {
            _root: root,
            share,
            source,
            target,
            arrival,
            hour_before,
            meanwhile,
            row,
        })
    }

    /// The base this runs on cannot show what the row needs, if so.
    fn unsupported_base(&self) -> Result<Option<&'static str>, io::Error> {
        let (arrival, target, hour_before) = (&self.arrival, &self.target, self.hour_before);
        if self.meanwhile.told_by_change_time()
            && !a_later_change_time(arrival, target, hour_before)?
        {
            return Ok(Some(
                "a base that reports the modification time as the change time",
            ));
        }
        let root = self.share.parent().expect("the share has a root");
        if self.meanwhile.stage_evicted() && !base_takes_rename_flags(root)? {
            return Ok(Some("a base that refuses or fakes rename flags"));
        }
        Ok(None)
    }

    fn install(&self, preset: Preset, happened: &Arc<AtomicBool>) -> FsQuirks {
        let racer =
            self.meanwhile
                .racer(self.share.clone(), self.arrival.clone(), happened.clone());
        let after = if self.meanwhile.stage_evicted() {
            stage_evictions(&self.share, happened)
        } else {
            Vec::new()
        };
        FsQuirks::install(
            &self.share,
            Quirks {
                trash: Trash::Unsupported,
                before: vec![(Op::OpenSource, "source", 0, racer)],
                after,
                ..preset()
            },
        )
    }

    fn expect_target(
        &self,
        result: Result<(), glib::Error>,
        replaced: bool,
    ) -> Result<(), io::Error> {
        let row = &self.row;
        if replaced {
            assert!(result.is_ok(), "{row}: {result:?}");
            assert_eq!(fs::read(&self.target)?, b"new", "{row}");
        } else {
            let Err(error) = result else {
                panic!("{row}: replaced instead of refusing")
            };
            assert!(error.to_string().contains("changed"), "{row}: {error}");
            assert_eq!(fs::read(&self.target)?, self.meanwhile.theirs(), "{row}");
        }
        expect_names(&self.share, &["source", "target"], row)
    }
}

#[test]
fn a_target_is_replaced_only_while_its_mount_still_knows_it() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    if !base_offers(&FILE_HANDLES)? {
        return Ok(());
    }
    let samba = || Quirks {
        births: Births::MadeUp,
        ..Quirks::cifs()
    };
    let path_keyed = || Quirks {
        reused_inodes: Some("target"),
        ..Quirks::sshfs()
    };
    let reusing_nfs3 = || Quirks {
        reused_inodes: Some("target"),
        ..Quirks::nfs3()
    };
    // The mount, what happens meanwhile, whether the target is replaced, and whether what
    // happens is possible on this base.
    let rows: [(Preset, Meanwhile, bool, bool); 9] = [
        (Quirks::vfat, Meanwhile::Evicted, true, true),
        (Quirks::vfat, Meanwhile::StageEvicted, true, true),
        (Quirks::sshfs, Meanwhile::Evicted, true, false),
        (Quirks::sshfs, Meanwhile::ReplacedAlike, false, true),
        (path_keyed, Meanwhile::ReplacedLonger, false, true),
        (reusing_nfs3, Meanwhile::ReplacedAlike, false, true),
        (samba, Meanwhile::EvictedOnceNumbersWent, true, true),
        (samba, Meanwhile::ReplacedAlikeOnceNumbersWent, false, true),
        (
            samba,
            Meanwhile::ReplacedAlikeAndEvictedOnceNumbersWent,
            false,
            true,
        ),
    ];
    for (preset, meanwhile, replaced, possible) in rows {
        let share = Share::new(preset, meanwhile)?;
        if let Some(base) = share.unsupported_base()? {
            eprintln!("skipped on {base}: {}", share.row);
            continue;
        }
        let happened = Arc::new(AtomicBool::new(!possible));
        let scope = share.install(preset, &happened);

        let (result, _) = replace_reporting(&share.source, &share.target, false);

        assert_eq!(happened.load(Ordering::Acquire), possible, "{}", share.row);
        if meanwhile.stage_evicted() {
            assert_eq!(scope.count(Op::Exchange), 1, "{}", share.row);
        }
        share.expect_target(result, replaced)?;
    }
    Ok(())
}

/// Saves the racer's item at `target`, over whatever is there if `saves_over`.
fn taking(target: &Path, saved: &Path, saves_over: bool) -> Racer {
    let (target, saved) = (target.to_path_buf(), saved.to_path_buf());
    Box::new(move || {
        if saves_over {
            let _ = fs::write(&saved, b"racer");
            let _ = fs::rename(&saved, &target);
        } else {
            let _ = fs::write(&target, b"racer");
        }
    })
}

#[test]
fn a_taken_name_keeps_the_original_aside_and_names_it() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let cases: [(Preset, Op, usize, bool, bool); 4] = [
        (Quirks::default, Op::Rename, 1, false, false),
        (Quirks::vboxsf, Op::Create, 0, false, false),
        (Quirks::default, Op::Rename, 0, true, false),
        (Quirks::default, Op::Rename, 0, true, true),
    ];
    for (quirks, op, skip, saves_over, move_source) in cases {
        let root = tempfile::tempdir()?;
        let source = root.path().join("source");
        let target = root.path().join("target");
        fs::write(&source, b"new")?;
        fs::write(&target, b"old")?;
        let racer = taking(&target, &root.path().join("saved"), saves_over);
        let _quirks = FsQuirks::install(
            root.path(),
            without_trash(
                NFS_SUPER_MAGIC,
                Quirks {
                    before: vec![(op, "target", skip, racer)],
                    ..quirks()
                },
            ),
        );

        let (result, _) = replace_reporting(&source, &target, move_source);

        let row = format!("{op:?}, saves over: {saves_over}, moving: {move_source}");
        let error = result.expect_err("the racer keeps its item");
        let kept = originals_folder(root.path())?.ok_or("the original was not kept")?;
        assert!(
            error.message().contains(&format!("{kept}/target")),
            "{row}: {error}"
        );
        let original = root.path().join(&kept).join("target");
        if saves_over {
            assert_eq!(fs::read(original)?, b"racer", "{row}");
            expect_names(root.path(), &[kept.as_str(), "source"], &row)?;
        } else {
            assert_eq!(fs::read(original)?, b"old", "{row}");
            assert_eq!(fs::read(&target)?, b"racer", "{row}");
            expect_names(root.path(), &[kept.as_str(), "source", "target"], &row)?;
        }
        assert_eq!(fs::read(&source)?, b"new", "{row}");
    }
    Ok(())
}
