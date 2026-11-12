// SPDX-License-Identifier: MIT

use super::*;
use crate::{
    adapters::{Births, FsOp, FsQuirks, Quirks, evict},
    services::{InodeNumbers, ItemIdentity},
};

fn load_stamp(events: &[DirectoryEvent]) -> Option<DirectoryStamp> {
    events.iter().find_map(|event| match event {
        DirectoryEvent::Stamp { request_id, stamp } if *request_id == RequestId(1) => {
            Some((**stamp).clone())
        }
        _ => None,
    })
}

fn names_digest(events: &[DirectoryEvent]) -> Option<u64> {
    events.iter().find_map(|event| match event {
        DirectoryEvent::ListingDigest {
            request_id,
            names_digest,
        } if *request_id == RequestId(1) => Some(*names_digest),
        _ => None,
    })
}

fn listed_stamp(path: &Path) -> Result<DirectoryStamp, &'static str> {
    load_stamp(&run_enumerate(listing_request(path, 100))).ok_or("no stamp")
}

fn probed_stamp(path: &Path) -> StampAnswer {
    let _serial = serial();
    let answer = Rc::new(RefCell::new(None));
    let slot = answer.clone();
    let _probe = LocalFileSource.probe_stamp(
        Location::local(path),
        Rc::new(move |stamp| {
            slot.replace(Some(stamp));
        }),
    );
    drain_until(|| answer.borrow().is_some());
    answer.take().expect("answered")
}

fn stamp_of(answer: StampAnswer) -> DirectoryStamp {
    match answer {
        StampAnswer::Stamp(stamp) => *stamp,
        other => panic!("no stamp: {other:?}"),
    }
}

/// Touches `probe` until the filesystem's change time moves past `than`'s.
fn wait_past(probe: &Path, than: &DirectoryStamp) {
    let than = than.changed.expect("a native change time");
    loop {
        fs::write(probe, b"probe").expect("write the probe");
        let stamped = fs::metadata(probe).expect("stat the probe");
        fs::remove_file(probe).expect("remove the probe");
        if (stamped.ctime(), stamped.ctime_nsec() as u32) > than {
            break;
        }
    }
}

/// Touches `probe` until the filesystem's modification time moves past `than`'s.
fn wait_modified_past(probe: &Path, than: &Path) -> std::io::Result<()> {
    loop {
        fs::write(probe, b"probe")?;
        let stamped = fs::metadata(probe)?;
        fs::remove_file(probe)?;
        if stamped.modified()? > fs::metadata(than)?.modified()? {
            return Ok(());
        }
    }
}

fn found_by_identity(departed: &Path, identity: ItemIdentity) -> Option<Location> {
    let _serial = serial();
    let found = Rc::new(RefCell::new(None));
    let slot = found.clone();
    let _find = LocalFileSource.find_by_identity(
        Location::local(departed),
        identity,
        Rc::new(move |location| {
            slot.replace(Some(location));
        }),
    );
    drain_until(|| found.borrow().is_some());
    found.take().flatten()
}

fn assert_stamp_first_and_digest_last(events: &[DirectoryEvent]) {
    assert!(matches!(events.first(), Some(DirectoryEvent::Stamp { .. })));
    assert!(matches!(
        &events[events.len() - 2..],
        [
            DirectoryEvent::ListingDigest { .. },
            DirectoryEvent::Finished { .. }
        ]
    ));
}

/// A home folder, a folder linked from it onto a network share, and an SSH mount.
struct MountedTree {
    root: tempfile::TempDir,
    _scopes: [FsQuirks; 3],
}

impl MountedTree {
    fn new(no_mount_ids: bool) -> Result<Self, Box<dyn Error>> {
        let root = tempfile::tempdir()?;
        let (home, nas, ssh) = (
            root.path().join("home"),
            root.path().join("nas"),
            root.path().join("ssh"),
        );
        fs::create_dir_all(home.join("documents"))?;
        fs::create_dir_all(nas.join("music"))?;
        fs::create_dir_all(&ssh)?;
        std::os::unix::fs::symlink(nas.join("music"), home.join("music"))?;
        let mount = |path: &Path, quirks: Quirks| {
            FsQuirks::install(
                path,
                Quirks {
                    no_mount_ids,
                    ..quirks
                },
            )
        };
        let _scopes = [
            mount(&home, Quirks::ext4()),
            mount(&nas, Quirks::nfs42()),
            mount(&ssh, Quirks::sshfs()),
        ];
        Ok(Self { root, _scopes })
    }

    fn join(&self, path: &str) -> PathBuf {
        self.root.path().join(path)
    }
}

#[test]
fn a_load_reports_its_stamp_first_and_a_digest_of_its_names_last() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let folder = root.path().join("folder");
    fs::create_dir(&folder)?;
    for name in ["a", "b"] {
        fs::write(folder.join(name), b"content")?;
    }
    let gate = Gate::default();
    let _scope = FsQuirks::install(
        &folder,
        Quirks {
            before: vec![(FsOp::ListChunks, "*", 0, gate.racer())],
            ..Quirks::ext4()
        },
    );

    let events = Events::default();
    {
        let _serial = serial();
        let _load = load_into(&events, &folder, 1);
        drain_until(|| gate.reached() && load_stamp(&events.borrow()).is_some());
        assert_eq!(outcome_of(&events, 1), None);
        gate.release();
        drain_until(|| outcome_of(&events, 1).is_some());
    }
    let first = events.take();
    let again = run_enumerate(listing_request(&folder, 100));
    fs::rename(folder.join("b"), folder.join("c"))?;
    let renamed = run_enumerate(listing_request(&folder, 100));
    let truncated = run_enumerate(listing_request(&folder, 1));

    assert_stamp_first_and_digest_last(&first);
    let stamp = load_stamp(&first).ok_or("no stamp")?;
    assert_eq!(
        stamp.identity.map(|identity| identity.evidence.kind),
        Some(rustix::fs::FileType::Directory)
    );
    let digest = names_digest(&first);
    assert_eq!(
        (names_digest(&again), names_digest(&truncated)),
        (digest, None)
    );
    assert_ne!(names_digest(&renamed), digest);
    assert!(load_stamp(&truncated).is_some());
    Ok(())
}

#[test]
fn a_check_sees_a_change_the_attribute_cache_hides() -> Result<(), Box<dyn Error>> {
    for checked_by_a_load in [false, true] {
        let root = tempfile::tempdir()?;
        let share = root.path().join("share");
        fs::create_dir(&share)?;
        let _scope = FsQuirks::install(&share, Quirks::nfs42());
        let check = || match checked_by_a_load {
            true => listed_stamp(&share),
            false => Ok(stamp_of(probed_stamp(&share))),
        };

        let baseline = check()?;
        wait_past(&root.path().join("probe"), &baseline);
        fs::write(share.join("added elsewhere"), b"content")?;
        let cached = crate::adapters::local_operations::folder_stamp(&share, false)
            .map_err(|error| error.to_string())?;
        let checked = check()?;

        assert_eq!(
            (cached.modified, Some(cached.changed)),
            (baseline.modified, baseline.changed),
            "a plain stat answers from the cache"
        );
        assert!(
            checked.moved_since(&baseline),
            "checked by a load: {checked_by_a_load}"
        );
    }
    Ok(())
}

#[test]
fn a_stamp_classifies_the_folder_by_its_mount_not_its_path() -> Result<(), Box<dyn Error>> {
    for no_mount_ids in [false, true] {
        let tree = MountedTree::new(no_mount_ids)?;
        let documents = listed_stamp(&tree.join("home/documents"))?;
        let linked = listed_stamp(&tree.join("home/music"))?;
        let music = listed_stamp(&tree.join("nas/music"))?;
        let ssh = listed_stamp(&tree.join("ssh"))?;

        assert_eq!(
            (
                documents.outside_writers,
                linked.outside_writers,
                music.outside_writers,
                linked.identity.map(|identity| identity.traits.numbers),
            ),
            (false, true, true, Some(InodeNumbers::Kept)),
            "no mount ids: {no_mount_ids}"
        );
        assert_eq!(
            (documents.daemon_cache, music.daemon_cache, ssh.daemon_cache),
            (None, None, Some(Duration::from_secs(21))),
            "no mount ids: {no_mount_ids}"
        );
        assert_eq!(
            (
                music.mount.is_some(),
                linked.mount == music.mount,
                linked.mount == documents.mount
            ),
            (true, true, false),
            "no mount ids: {no_mount_ids}"
        );
    }
    Ok(())
}

#[test]
fn a_folder_put_back_to_its_old_time_still_reads_as_changed() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let folder = root.path().join("folder");
    fs::create_dir(&folder)?;

    let baseline = stamp_of(probed_stamp(&folder));
    wait_past(&root.path().join("probe"), &baseline);
    fs::write(folder.join("added"), b"content")?;
    let (seconds, nanoseconds) = baseline.modified;
    fs::File::open(&folder)?.set_modified(
        SystemTime::UNIX_EPOCH + Duration::new(u64::try_from(seconds)?, nanoseconds),
    )?;
    let after = stamp_of(probed_stamp(&folder));

    assert_eq!(after.modified, baseline.modified);
    assert!(after.moved_since(&baseline));
    Ok(())
}

#[test]
fn a_renamed_folder_is_followed_where_its_mount_proves_it_is_the_same() -> Result<(), Box<dyn Error>>
{
    type Row = (&'static str, fn() -> Quirks, bool, bool);
    let rows: [Row; 4] = [
        ("numbers kept", Quirks::ext4, false, true),
        (
            "renumbered, birth kept",
            || Quirks {
                births: Births::MadeUp,
                ..Quirks::vfat()
            },
            true,
            true,
        ),
        ("renumbered, no birth", Quirks::sshfs, true, false),
        (
            "renumbered, folder births unreliable",
            Quirks::exfat,
            true,
            false,
        ),
    ];
    for (row, quirks, evicted, followed) in rows {
        let root = tempfile::tempdir()?;
        let parent = root.path().join("parent");
        let (old, new) = (parent.join("old"), parent.join("new"));
        fs::create_dir_all(&old)?;
        let _scope = FsQuirks::install(&parent, quirks());
        let identity = folder_identity(&old)?;
        wait_modified_past(&root.path().join("probe"), &old)?;
        fs::create_dir(parent.join("sibling"))?;
        fs::rename(&old, &new)?;
        if evicted {
            assert!(evict(&new), "{row}");
        }

        assert_eq!(
            found_by_identity(&old, identity),
            followed.then(|| Location::local(&new)),
            "{row}"
        );
    }
    Ok(())
}

#[test]
fn a_folder_is_found_where_a_renamed_ancestor_took_it() -> Result<(), Box<dyn Error>> {
    type Change = fn(&Path) -> std::io::Result<()>;
    // Each row changes a tree holding the open folder `a/b/c` and `a/elsewhere`.
    let rows: [(&str, Change, Option<&str>); 4] = [
        (
            "parent renamed",
            |root| fs::rename(root.join("a/b"), root.join("a/b2")),
            Some("a/b2/c"),
        ),
        (
            "grandparent renamed",
            |root| fs::rename(root.join("a"), root.join("a2")),
            Some("a2/b/c"),
        ),
        (
            "parent moved into another folder",
            |root| fs::rename(root.join("a/b"), root.join("a/elsewhere/b")),
            None,
        ),
        (
            "parent deleted",
            |root| fs::remove_dir_all(root.join("a/b")),
            None,
        ),
    ];
    for (row, change, found) in rows {
        let root = tempfile::tempdir()?;
        let open = root.path().join("a/b/c");
        fs::create_dir_all(&open)?;
        fs::create_dir_all(root.path().join("a/elsewhere"))?;
        let identity = folder_identity(&open)?;
        change(root.path())?;

        assert_eq!(
            found_by_identity(&open, identity),
            found.map(|found| Location::local(root.path().join(found))),
            "{row}"
        );
    }
    Ok(())
}

#[test]
fn a_renamed_folder_is_never_searched_for_on_another_mount() -> Result<(), Box<dyn Error>> {
    // The open folder `a/b/c/d` after `a/b` became `a/b2`, with a mount at each row's path.
    let rows = [
        ("no mount", None, true),
        ("the renamed folder is a mount point", Some("a/b2"), false),
        (
            "a folder below the renamed one is a mount point",
            Some("a/b2/c"),
            false,
        ),
        ("the searched folder is a mount point", Some("a"), true),
    ];
    for (row, mounted, found) in rows {
        let root = tempfile::tempdir()?;
        let root = fs::canonicalize(root.path())?;
        let open = root.join("a/b/c/d");
        fs::create_dir_all(&open)?;
        let identity = folder_identity(&open)?;
        fs::rename(root.join("a/b"), root.join("a/b2"))?;
        let mounts = MountTable::parse(mounted.map_or_else(String::new, |mounted| {
            format!(
                "36 35 98:0 / {} rw - ext4 /dev/sdb1 rw\n",
                root.join(mounted).display()
            )
        }));

        assert_eq!(
            native_relocated_folder(&open, identity, &mounts, &Progress::unlaned()),
            found.then(|| root.join("a/b2/c/d")),
            "{row}"
        );
    }
    Ok(())
}
