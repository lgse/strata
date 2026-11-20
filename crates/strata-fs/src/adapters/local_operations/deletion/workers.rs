// SPDX-License-Identifier: MIT

use std::{
    collections::HashSet,
    os::fd::AsFd,
    path::{Path, PathBuf},
    thread,
};

use super::{
    super::{MAX_LOCAL_IO_WORKERS, fs_ops, fs_ops::fs, native_fs::open_local_parent_directory},
    queue::PinnedEntry,
    run::DeletionTarget,
};

pub(in crate::adapters::local_operations) fn local_device_is_rotational(
    fd: impl AsFd,
) -> Option<bool> {
    fs_ops::assert_off_main_thread();
    let stat = rustix::fs::fstat(fd).ok()?;
    let device = PathBuf::from("/sys/dev/block").join(format!(
        "{}:{}",
        rustix::fs::major(stat.st_dev),
        rustix::fs::minor(stat.st_dev)
    ));
    let device = std::fs::canonicalize(device).ok()?;
    rotational_sysfs_node(&device, &mut HashSet::new())
}

enum Slaves {
    None,
    Known(bool),
    Unknown,
}

fn rotational_sysfs_node(path: &Path, visited: &mut HashSet<PathBuf>) -> Option<bool> {
    let path = std::fs::canonicalize(path).ok()?;
    if !visited.insert(path.clone()) {
        return None;
    }
    match rotational_slaves(&path, visited) {
        Slaves::None => rotational_queue_flag(&path),
        Slaves::Known(rotational) => Some(rotational),
        Slaves::Unknown => None,
    }
}

fn rotational_slaves(path: &Path, visited: &mut HashSet<PathBuf>) -> Slaves {
    let Ok(slaves) = std::fs::read_dir(path.join("slaves")) else {
        return Slaves::None;
    };
    let mut found = Slaves::None;
    for slave in slaves.flatten() {
        match rotational_sysfs_node(&slave.path(), visited) {
            Some(true) => return Slaves::Known(true),
            Some(false) => {
                if matches!(found, Slaves::None) {
                    found = Slaves::Known(false);
                }
            }
            None => found = Slaves::Unknown,
        }
    }
    found
}

fn rotational_queue_flag(path: &Path) -> Option<bool> {
    let mut current = Some(path);
    while let Some(node) = current {
        if let Ok(value) = std::fs::read_to_string(node.join("queue/rotational")) {
            return match value.trim() {
                "0" => Some(false),
                "1" => Some(true),
                _ => None,
            };
        }
        current = node.parent();
    }
    None
}

pub(in crate::adapters::local_operations) fn bounded_local_worker_count(
    available: usize,
    rotational: bool,
) -> usize {
    let available = available.max(1);
    if rotational {
        1
    } else {
        available.min(MAX_LOCAL_IO_WORKERS)
    }
}

pub(in crate::adapters::local_operations) fn local_delete_worker_count(
    roots: &[PinnedEntry],
) -> usize {
    let available = thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(1);
    let rotational = roots
        .iter()
        .any(|root| fs().rotational(root.parent.as_fd()) == Some(true));
    bounded_local_worker_count(available, rotational)
}

pub(in crate::adapters::local_operations) async fn local_parent_worker_count(
    parents: impl IntoIterator<Item = PathBuf>,
) -> usize {
    let parents = parents.into_iter().collect::<HashSet<_>>();
    gio::spawn_blocking(move || {
        let available = thread::available_parallelism()
            .map(usize::from)
            .unwrap_or(1);
        let rotational = parents.iter().any(|parent| {
            open_local_parent_directory(parent)
                .ok()
                .and_then(|parent| fs().rotational(parent.as_fd()))
                == Some(true)
        });
        bounded_local_worker_count(available, rotational)
    })
    .await
    .unwrap_or(1)
}

pub(super) async fn deletion_worker_count(targets: &[DeletionTarget]) -> usize {
    local_parent_worker_count(targets.iter().filter_map(|target| {
        target
            .location
            .native_path()
            .and_then(Path::parent)
            .map(Path::to_path_buf)
    }))
    .await
}
