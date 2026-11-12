// SPDX-License-Identifier: MIT
//! Directory change stamps for native and remote folders.

use super::*;

pub(super) fn directory_stamp(
    path: &Path,
    stamp: &FolderStamp,
    read_at: SystemTime,
) -> DirectoryStamp {
    let mount = crate::adapters::local_operations::stamp_mount(path, stamp);
    DirectoryStamp {
        modified: stamp.modified,
        changed: Some(stamp.changed),
        identity: Some(ItemIdentity {
            evidence: Evidence {
                device: stamp.device,
                inode: stamp.inode,
                kind: stamp.kind,
                birth: stamp.birth,
                handle: None,
                attributes: None,
            },
            traits: mount
                .as_ref()
                .map_or_else(IdentityTraits::default, |mount| mount.identity),
        }),
        mount: stamp
            .mount_id
            .or(mount.as_ref().map(|mount| mount.id))
            .map(MountToken::Native),
        outside_writers: mount.as_ref().is_some_and(|mount| mount.outside_writers),
        daemon_cache: mount.as_ref().and_then(|mount| mount.daemon_cache),
        coarse_window: mount
            .and_then(|mount| mount.coarse_window)
            .or_else(|| CoarseTimes::whole_second(stamp.modified.1).map(|coarse| coarse.window)),
        read_at,
    }
}

pub(super) fn remote_stamp(uri: &str, info: &gio::FileInfo, read_at: SystemTime) -> DirectoryStamp {
    let time = |seconds: &str, microseconds: &str| {
        info.has_attribute(seconds).then(|| {
            (
                i64::try_from(info.attribute_uint64(seconds)).unwrap_or(i64::MAX),
                info.attribute_uint32(microseconds).saturating_mul(1000),
            )
        })
    };
    let modified = time("time::modified", "time::modified-usec").unwrap_or_default();
    let scheme = glib::Uri::parse_scheme(uri).unwrap_or_default();
    DirectoryStamp {
        modified,
        changed: time("time::changed", "time::changed-usec"),
        identity: None,
        mount: Some(MountToken::Remote(mount_lanes::authority(uri))),
        outside_writers: !matches!(scheme.as_str(), "admin" | "mtp" | "gphoto2"),
        coarse_window: CoarseTimes::whole_second(modified.1).map(|coarse| coarse.window),
        daemon_cache: None,
        read_at,
    }
}

pub(super) fn stamped_remotely(location: &Location) -> bool {
    location.uri_value().is_some_and(|uri| {
        !location.is_recent_location()
            && !location.is_camera_photo_root()
            && !location.is_computer_root()
            && glib::Uri::parse_scheme(uri).is_some_and(|scheme| scheme != "trash")
    })
}

pub(super) fn stamp_failure(error: rustix::io::Errno) -> StampAnswer {
    use rustix::io::Errno;
    match error {
        Errno::NOENT | Errno::NOTDIR | Errno::STALE => StampAnswer::Missing,
        Errno::ACCESS | Errno::PERM => StampAnswer::Inaccessible,
        _ => StampAnswer::Unreachable,
    }
}

pub(super) fn remote_stamp_failure(error: &glib::Error) -> StampAnswer {
    if error.matches(gio::IOErrorEnum::NotFound) || error.matches(gio::IOErrorEnum::NotDirectory) {
        StampAnswer::Missing
    } else if error.matches(gio::IOErrorEnum::PermissionDenied) {
        StampAnswer::Inaccessible
    } else {
        StampAnswer::Unreachable
    }
}

pub(super) fn read_stamp(
    path: &Path,
    progress: &Progress,
) -> Result<DirectoryStamp, rustix::io::Errno> {
    let read_at = SystemTime::now();
    let stamp = crate::adapters::local_operations::folder_stamp(path, true);
    progress.answered(&stamp);
    stamp.map(|stamp| {
        if let Some(id) = stamp.mount_id {
            progress.learn(id);
        }
        directory_stamp(path, &stamp, read_at)
    })
}

pub(super) fn stamp_answer(stamp: Result<DirectoryStamp, rustix::io::Errno>) -> StampAnswer {
    match stamp {
        Ok(stamp) => StampAnswer::Stamp(Box::new(stamp)),
        Err(error) => stamp_failure(error),
    }
}

pub(super) fn unanswered_stamp(refusal: Unanswered) -> StampAnswer {
    match refusal {
        Unanswered::Failed => StampAnswer::Unreachable,
        Unanswered::TimedOut | Unanswered::Skipped => StampAnswer::Unresponsive,
    }
}

pub(super) fn check_native_stamps(
    locations: Vec<Location>,
    emit: Rc<dyn Fn(Location, StampAnswer)>,
) -> glib::JoinHandle<()> {
    let paths: Vec<PathBuf> = locations
        .iter()
        .filter_map(|location| location.native_path().map(Path::to_path_buf))
        .collect();
    let job = mount_lanes::run(&paths[0].clone(), Class::Background, move |progress| {
        paths
            .iter()
            .map(|path| stamp_answer(read_stamp(path, progress)))
            .collect::<Vec<_>>()
    });
    glib::MainContext::default().spawn_local(async move {
        let answers = match job.await {
            Ok(answers) => answers,
            Err(refusal) => vec![unanswered_stamp(refusal); locations.len()],
        };
        for (location, answer) in locations.into_iter().zip(answers) {
            emit(location, answer);
        }
    })
}
