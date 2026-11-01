// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, OnceCell, RefCell},
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    rc::{Rc, Weak},
};

use gtk::{gio, prelude::*};

use crate::adapters::MountTable;

fn mounted_devices(table: &MountTable) -> Vec<PathBuf> {
    table
        .sources()
        .filter(|(point, source)| {
            source.starts_with("/dev")
                && point.is_absolute()
                && !gio_unix::functions::is_mount_path_system_internal(point)
        })
        .map(|(point, _)| point.to_owned())
        .collect()
}

pub(super) fn block_device_from_mount_table(table: &MountTable, path: &Path) -> Option<PathBuf> {
    table
        .source_for(path)
        .filter(|source| source.starts_with("/dev"))
        .map(Path::to_owned)
}

pub(super) fn mounted_path_from_table(table: &MountTable, device: &Path) -> Option<PathBuf> {
    let device = device
        .canonicalize()
        .unwrap_or_else(|_| device.to_path_buf());
    table.sources().find_map(|(point, source)| {
        if !source.starts_with("/dev") {
            return None;
        }
        let source = source.canonicalize().unwrap_or_else(|_| source.to_owned());
        (source == device).then(|| point.to_owned())
    })
}

pub(super) fn label_identity(uuid: Option<&str>, root_uri: Option<&str>) -> Option<String> {
    [("volume", uuid), ("root", root_uri)]
        .into_iter()
        .find_map(|(namespace, value)| {
            value
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| format!("{namespace}:{value}"))
        })
}

pub(super) fn device_identity(
    unix_device: Option<&str>,
    uuid: Option<&str>,
    drive_identifier: Option<&str>,
) -> Option<String> {
    [unix_device, uuid, drive_identifier]
        .into_iter()
        .find_map(|value| {
            value
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
        })
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::ui) struct RemovableDestination {
    pub id: String,
    pub name: String,
    pub root: PathBuf,
}

#[derive(Clone, Copy)]
struct RemovableMountFacts {
    shadowed: bool,
    native_root: bool,
    removable: bool,
}

fn removable_mount_is_eligible(facts: RemovableMountFacts) -> bool {
    !facts.shadowed && facts.native_root && facts.removable
}

fn send_to_device_identity(
    volume_uuid: Option<&str>,
    drive_uuid: Option<&str>,
    volume_unix_device: Option<&str>,
    drive_unix_device: Option<&str>,
    root_uri: Option<&str>,
) -> Option<String> {
    let unix_device = volume_unix_device
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or_else(|| {
            drive_unix_device
                .map(str::trim)
                .filter(|value| !value.is_empty())
        });
    [
        ("volume", volume_uuid),
        ("drive", drive_uuid),
        ("unix", unix_device),
        ("root", root_uri),
    ]
    .into_iter()
    .find_map(|(namespace, value)| {
        value
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| format!("{namespace}:{value}"))
    })
}

fn send_to_mount_identity(mount: &gio::Mount) -> Option<String> {
    let volume = mount.volume();
    let drive = mount
        .drive()
        .or_else(|| volume.as_ref().and_then(|volume| volume.drive()));
    let root = mount.root();
    let root_uri = root.uri();
    let volume_uuid = volume.as_ref().and_then(|volume| volume.uuid());
    let drive_uuid = drive
        .as_ref()
        .and_then(|drive| drive.identifier(gio::VOLUME_IDENTIFIER_KIND_UUID.as_str()));
    let volume_unix_device = volume
        .as_ref()
        .and_then(|volume| volume.identifier(gio::VOLUME_IDENTIFIER_KIND_UNIX_DEVICE.as_str()));
    let drive_unix_device = drive
        .as_ref()
        .and_then(|drive| drive.identifier(gio::VOLUME_IDENTIFIER_KIND_UNIX_DEVICE.as_str()));
    send_to_device_identity(
        volume_uuid.as_deref(),
        drive_uuid.as_deref(),
        volume_unix_device.as_deref(),
        drive_unix_device.as_deref(),
        Some(root_uri.as_str()),
    )
}

fn candidate_for_mount(mount: &gio::Mount) -> Option<RemovableDestination> {
    let root = mount.root().path();
    let facts = RemovableMountFacts {
        shadowed: mount.is_shadowed(),
        native_root: root.is_some(),
        removable: crate::adapters::mount_is_removable(mount),
    };
    if !removable_mount_is_eligible(facts) {
        return None;
    }
    Some(RemovableDestination {
        id: send_to_mount_identity(mount)?,
        name: mount.name().to_string(),
        root: root?,
    })
}

fn without_ambiguous_destinations(
    destinations: Vec<RemovableDestination>,
) -> Vec<RemovableDestination> {
    let mut counts = HashMap::new();
    for destination in &destinations {
        *counts.entry(destination.id.clone()).or_insert(0usize) += 1;
    }
    destinations
        .into_iter()
        .filter(|destination| counts.get(&destination.id) == Some(&1))
        .collect()
}

fn sort_removable_destinations(destinations: &mut [RemovableDestination]) {
    destinations.sort_by(|left, right| {
        left.name
            .to_lowercase()
            .cmp(&right.name.to_lowercase())
            .then_with(|| left.id.cmp(&right.id))
    });
}

type Watcher = (u64, Weak<dyn Fn()>);

#[derive(Default)]
struct RemovableCache {
    candidates: RefCell<Vec<RemovableDestination>>,
    writable: RefCell<HashMap<PathBuf, (u64, bool)>>,
    unanswered: RefCell<HashSet<PathBuf>>,
    destinations: RefCell<Vec<RemovableDestination>>,
    recent_folders: RefCell<HashMap<String, (u64, HashSet<PathBuf>)>>,
    generation: Cell<u64>,
    refresh_queued: Cell<bool>,
    watchers: RefCell<Vec<Watcher>>,
    next_watcher: Cell<u64>,
    monitor: OnceCell<gio::VolumeMonitor>,
    #[cfg(test)]
    offered_for_test: RefCell<Option<Vec<RemovableDestination>>>,
}

thread_local! {
    static REMOVABLE: RemovableCache = RemovableCache::default();
}

pub(in crate::ui) struct RemovableWatch {
    id: u64,
    _changed: Rc<dyn Fn()>,
}

impl Drop for RemovableWatch {
    fn drop(&mut self) {
        REMOVABLE.with(|cache| {
            cache.watchers.borrow_mut().retain(|(id, _)| *id != self.id);
        });
    }
}

pub(in crate::ui) fn watch_removable_destinations(changed: impl Fn() + 'static) -> RemovableWatch {
    start_removable_destinations();
    let changed: Rc<dyn Fn()> = Rc::new(changed);
    let id = REMOVABLE.with(|cache| {
        let id = cache.next_watcher.get() + 1;
        cache.next_watcher.set(id);
        cache
            .watchers
            .borrow_mut()
            .push((id, Rc::downgrade(&changed)));
        id
    });
    RemovableWatch {
        id,
        _changed: changed,
    }
}

pub(in crate::ui) fn removable_destinations() -> Vec<RemovableDestination> {
    start_removable_destinations();
    probe_unanswered_roots();
    REMOVABLE.with(|cache| cache.destinations.borrow().clone())
}

pub(in crate::ui) fn send_to_recent_folders(destination: &RemovableDestination) -> Vec<PathBuf> {
    let remembered = crate::ui::preferences::PreferenceManager::shared()
        .send_to_recent_destinations(&destination.id);
    if remembered.is_empty() {
        return remembered;
    }
    let previous = REMOVABLE.with(|cache| {
        cache
            .recent_folders
            .borrow()
            .get(&destination.id)
            .map(|(_, found)| found.clone())
            .unwrap_or_default()
    });
    let mut unlisted = previous.clone();
    let listed = remembered
        .iter()
        .filter(|relative| unlisted.remove(*relative))
        .cloned()
        .collect();
    check_recent_folders(destination.clone(), remembered, previous);
    listed
}

fn check_recent_folders(
    destination: RemovableDestination,
    remembered: Vec<PathBuf>,
    previous: HashSet<PathBuf>,
) {
    let generation = REMOVABLE.with(next_generation);
    gtk::glib::MainContext::default().spawn_local(async move {
        let Some(found) = folders_inside(&destination.root, &remembered, &previous).await else {
            return;
        };
        let changed = REMOVABLE.with(|cache| {
            let mut known = cache.recent_folders.borrow_mut();
            if known
                .get(&destination.id)
                .is_some_and(|(checked, _)| *checked > generation)
            {
                return false;
            }
            known
                .insert(destination.id, (generation, found.clone()))
                .map_or(!found.is_empty(), |(_, previous)| previous != found)
        });
        if changed {
            notify_removable_watchers();
        }
    });
}

async fn folders_inside(
    root: &Path,
    remembered: &[PathBuf],
    previous: &HashSet<PathBuf>,
) -> Option<HashSet<PathBuf>> {
    use crate::{adapters::probes, services::LocationValidationError};
    let unanswered =
        |error: &LocationValidationError| matches!(error, LocationValidationError::Unavailable(_));
    let root = match probes::destination_folder(root).await {
        Ok(root) => root,
        Err(error) if unanswered(&error) => return None,
        Err(_) => return Some(HashSet::new()),
    };
    let mut folders = HashSet::new();
    let mut found = HashSet::new();
    for relative in remembered
        .iter()
        .filter(|relative| crate::ui::preferences::is_valid_send_to_relative_path(relative))
    {
        match probes::destination_folder(&root.join(relative)).await {
            Ok(folder) if folder != root && folder.starts_with(&root) => {
                if folders.insert(folder) {
                    found.insert(relative.clone());
                }
            }
            Err(error) if unanswered(&error) && previous.contains(relative) => {
                found.insert(relative.clone());
            }
            _ => {}
        }
        if found.len() == crate::ui::preferences::SEND_TO_RECENT_DESTINATIONS_LIMIT {
            break;
        }
    }
    Some(found)
}

pub(in crate::ui) fn removable_destination_root(id: &str) -> Option<PathBuf> {
    removable_destinations()
        .into_iter()
        .find(|destination| destination.id == id)
        .map(|destination| destination.root)
}

fn start_removable_destinations() {
    let started = REMOVABLE.with(|cache| {
        if cache.monitor.get().is_some() {
            return false;
        }
        let monitor = gio::VolumeMonitor::get();
        monitor.connect_mount_added(|_, _| queue_removable_refresh());
        monitor.connect_mount_removed(|_, _| queue_removable_refresh());
        monitor.connect_mount_changed(|_, _| queue_removable_refresh());
        monitor.connect_volume_added(|_, _| queue_removable_refresh());
        monitor.connect_volume_removed(|_, _| queue_removable_refresh());
        monitor.connect_volume_changed(|_, _| queue_removable_refresh());
        monitor.connect_drive_connected(|_, _| queue_removable_refresh());
        monitor.connect_drive_disconnected(|_, _| queue_removable_refresh());
        monitor.connect_drive_changed(|_, _| queue_removable_refresh());
        cache.monitor.set(monitor).is_ok()
    });
    if started {
        queue_removable_refresh();
    }
}

fn queue_removable_refresh() {
    if REMOVABLE.with(|cache| cache.refresh_queued.replace(true)) {
        return;
    }
    gtk::glib::idle_add_local_once(|| {
        REMOVABLE.with(|cache| cache.refresh_queued.set(false));
        refresh_removable_destinations();
    });
}

fn removable_candidates() -> Vec<RemovableDestination> {
    #[cfg(test)]
    if let Some(offered) = REMOVABLE.with(|cache| cache.offered_for_test.borrow().clone()) {
        return offered;
    }
    gio::VolumeMonitor::get()
        .mounts()
        .iter()
        .filter_map(candidate_for_mount)
        .collect()
}

fn next_generation(cache: &RemovableCache) -> u64 {
    let generation = cache.generation.get() + 1;
    cache.generation.set(generation);
    generation
}

fn refresh_removable_destinations() {
    let candidates = removable_candidates();
    let roots: HashSet<PathBuf> = candidates
        .iter()
        .map(|candidate| candidate.root.clone())
        .collect();
    let generation = REMOVABLE.with(|cache| {
        cache
            .writable
            .borrow_mut()
            .retain(|root, _| roots.contains(root));
        cache.unanswered.borrow_mut().clear();
        cache.candidates.replace(candidates);
        next_generation(cache)
    });
    publish_removable_destinations();
    for root in roots {
        probe_removable_root(root, generation);
    }
}

fn probe_unanswered_roots() {
    let roots = REMOVABLE.with(|cache| cache.unanswered.take());
    if roots.is_empty() {
        return;
    }
    let generation = REMOVABLE.with(next_generation);
    for root in roots {
        probe_removable_root(root, generation);
    }
}

fn probe_removable_root(root: PathBuf, generation: u64) {
    gtk::glib::MainContext::default().spawn_local(async move {
        let answer = crate::adapters::probes::writable_folder(&root).await;
        let learned = REMOVABLE.with(|cache| {
            let offered = cache
                .candidates
                .borrow()
                .iter()
                .any(|candidate| candidate.root == root);
            if !offered {
                return false;
            }
            let Ok(folder) = answer else {
                cache.unanswered.borrow_mut().insert(root);
                return false;
            };
            let mut known = cache.writable.borrow_mut();
            if known
                .get(&root)
                .is_some_and(|(checked, _)| *checked > generation)
            {
                return false;
            }
            cache.unanswered.borrow_mut().remove(&root);
            known.insert(root, (generation, folder.is_some()));
            true
        });
        if learned {
            publish_removable_destinations();
        }
    });
}

fn publish_removable_destinations() {
    let changed = REMOVABLE.with(|cache| {
        let writable = cache.writable.borrow();
        let mut destinations = without_ambiguous_destinations(
            cache
                .candidates
                .borrow()
                .iter()
                .filter(|candidate| {
                    writable
                        .get(&candidate.root)
                        .is_some_and(|(_, writable)| *writable)
                })
                .cloned()
                .collect(),
        );
        sort_removable_destinations(&mut destinations);
        if *cache.destinations.borrow() == destinations {
            return false;
        }
        cache.destinations.replace(destinations);
        true
    });
    if changed {
        notify_removable_watchers();
    }
}

fn notify_removable_watchers() {
    let watchers: Vec<_> = REMOVABLE.with(|cache| {
        cache
            .watchers
            .borrow()
            .iter()
            .filter_map(|(_, changed)| changed.upgrade())
            .collect()
    });
    for changed in watchers {
        changed();
    }
}

#[cfg(test)]
pub(in crate::ui) fn offer_removable_destinations_for_test(offered: Vec<RemovableDestination>) {
    REMOVABLE.with(|cache| cache.offered_for_test.replace(Some(offered)));
    start_removable_destinations();
    queue_removable_refresh();
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct BlockCryptoHint {
    pub dm_uuid: Option<String>,
    pub id_fs_usage: Option<String>,
    pub id_fs_type: Option<String>,
    pub crypto_uuid: Option<String>,
    /// The device is, or is held by, an open dm-crypt mapping.
    pub cleartext_open: bool,
}

impl BlockCryptoHint {
    pub(super) fn as_ref(&self) -> BlockCryptoRef<'_> {
        BlockCryptoRef {
            dm_uuid: self.dm_uuid.as_deref(),
            id_fs_usage: self.id_fs_usage.as_deref(),
            id_fs_type: self.id_fs_type.as_deref(),
            crypto_uuid: self.crypto_uuid.as_deref(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct BlockCryptoRef<'a> {
    pub dm_uuid: Option<&'a str>,
    pub id_fs_usage: Option<&'a str>,
    pub id_fs_type: Option<&'a str>,
    pub crypto_uuid: Option<&'a str>,
}

// GVfs marks crypto volumes with changes-prevent/changes-allow emblems, not encrypted icon names.
pub(super) fn is_encrypted_device(
    icon_names: &[&str],
    start_stop: Option<gio::DriveStartStopType>,
    block: BlockCryptoRef<'_>,
) -> bool {
    icon_names
        .iter()
        .copied()
        .any(icon_name_indicates_encrypted)
        || start_stop == Some(gio::DriveStartStopType::Password)
        || block_indicates_crypto(block)
}

/// GVfs' padlock emblem wins; without it, an open mapping whose filesystem is
/// unmounted (for example an fstab entry) still counts as unlocked.
pub(super) fn encrypted_device_is_locked(
    icon_names: &[&str],
    has_mount: bool,
    cleartext_open: bool,
) -> bool {
    let mut padlock_locked = false;
    let mut padlock_unlocked = false;
    for name in icon_names {
        match icon_name_stem(name).as_str() {
            "changes-prevent" => padlock_locked = true,
            "changes-allow" => padlock_unlocked = true,
            _ => {}
        }
    }
    if padlock_locked {
        true
    } else if padlock_unlocked {
        false
    } else {
        !has_mount && !cleartext_open
    }
}

#[derive(Default)]
struct CryptoHints {
    known: RefCell<HashMap<String, BlockCryptoHint>>,
    pending: RefCell<HashSet<String>>,
    monitor: OnceCell<gio::VolumeMonitor>,
}

thread_local! {
    static CRYPTO_HINTS: CryptoHints = CryptoHints::default();
}

pub(super) fn block_crypto_hint(unix_device: &str) -> BlockCryptoHint {
    block_crypto_hint_with(unix_device, probe_block_crypto)
}

fn block_crypto_hint_with(
    unix_device: &str,
    probe: fn(&str) -> BlockCryptoHint,
) -> BlockCryptoHint {
    CRYPTO_HINTS.with(|hints| {
        hints.monitor.get_or_init(|| {
            let monitor = gio::VolumeMonitor::get();
            let recheck = move || recheck_crypto_hints(probe);
            monitor.connect_volume_added(move |_, _| recheck());
            monitor.connect_volume_removed(move |_, _| recheck());
            monitor.connect_volume_changed(move |_, _| recheck());
            monitor.connect_drive_connected(move |_, _| recheck());
            monitor.connect_drive_disconnected(move |_, _| recheck());
            monitor.connect_drive_changed(move |_, _| recheck());
            monitor
        });
    });
    let known = CRYPTO_HINTS.with(|hints| hints.known.borrow().get(unix_device).cloned());
    known.unwrap_or_else(|| {
        probe_crypto_hint(unix_device.to_owned(), probe);
        BlockCryptoHint::default()
    })
}

fn recheck_crypto_hints(probe: fn(&str) -> BlockCryptoHint) {
    let devices: Vec<String> =
        CRYPTO_HINTS.with(|hints| hints.known.borrow().keys().cloned().collect());
    for device in devices {
        probe_crypto_hint(device, probe);
    }
}

fn probe_crypto_hint(unix_device: String, probe: fn(&str) -> BlockCryptoHint) {
    if !CRYPTO_HINTS.with(|hints| hints.pending.borrow_mut().insert(unix_device.clone())) {
        return;
    }
    gtk::glib::MainContext::default().spawn_local(async move {
        let device = unix_device.clone();
        let hint = gio::spawn_blocking(move || probe(&device))
            .await
            .unwrap_or_default();
        let changed = CRYPTO_HINTS.with(|hints| {
            hints.pending.borrow_mut().remove(&unix_device);
            let seen = hints
                .known
                .borrow_mut()
                .insert(unix_device, hint.clone())
                .unwrap_or_default();
            seen != hint
        });
        if changed {
            super::device_release::refresh_sidebars();
        }
    });
}

fn probe_block_crypto(unix_device: &str) -> BlockCryptoHint {
    let unix_device = unix_device.trim();
    if unix_device.is_empty() {
        return BlockCryptoHint::default();
    }
    let Some(sysfs) = sysfs_block_path(unix_device) else {
        return BlockCryptoHint::default();
    };
    let udev = read_trimmed(sysfs.join("dev"))
        .and_then(|dev| std::fs::read_to_string(format!("/run/udev/data/b{dev}")).ok());
    let dm_uuid = read_trimmed(sysfs.join("dm/uuid")).or_else(|| {
        udev.as_deref()
            .and_then(|data| udev_property(data, "DM_UUID"))
    });
    let (id_fs_usage, id_fs_type) = udev
        .as_deref()
        .map(udev_crypto_properties)
        .unwrap_or((None, None));
    let crypto_uuid = dm_uuid
        .as_deref()
        .and_then(luks_uuid_from_dm_uuid)
        .or_else(|| luks_uuid_from_unix_device(unix_device))
        .or_else(|| udev.as_deref().and_then(udev_crypto_uuid))
        .or_else(|| slave_crypto_uuid(&sysfs));
    let cleartext_open =
        dm_uuid.as_deref().is_some_and(dm_uuid_is_crypt) || holder_is_crypt_mapping(&sysfs);
    BlockCryptoHint {
        dm_uuid,
        id_fs_usage,
        id_fs_type,
        crypto_uuid,
        cleartext_open,
    }
}

fn holder_is_crypt_mapping(sysfs: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(sysfs.join("holders")) else {
        return false;
    };
    entries.flatten().any(|entry| {
        read_trimmed(
            Path::new("/sys/class/block")
                .join(entry.file_name())
                .join("dm/uuid"),
        )
        .is_some_and(|uuid| dm_uuid_is_crypt(&uuid))
    })
}

// An unlocked volume exposes the filesystem UUID, not the saved passphrase's LUKS UUID.
pub(super) fn crypto_password_uuid(
    volume_uuid: Option<&str>,
    unix_device: Option<&str>,
    block: BlockCryptoRef<'_>,
    locked: bool,
) -> Option<String> {
    if let Some(uuid) = block.crypto_uuid.and_then(normalize_luks_uuid) {
        return Some(uuid);
    }
    if let Some(uuid) = block.dm_uuid.and_then(luks_uuid_from_dm_uuid) {
        return Some(uuid);
    }
    if let Some(uuid) = unix_device.and_then(luks_uuid_from_unix_device) {
        return Some(uuid);
    }
    if locked {
        volume_uuid.and_then(normalize_luks_uuid)
    } else {
        None
    }
}

pub(super) fn luks_uuid_from_dm_uuid(dm_uuid: &str) -> Option<String> {
    let rest = strip_prefix_ignore_ascii_case(dm_uuid.trim(), "CRYPT-")?;
    let (kind, remainder) = rest.split_once('-')?;
    if !kind.eq_ignore_ascii_case("LUKS1") && !kind.eq_ignore_ascii_case("LUKS2") {
        return None;
    }
    take_uuid_hex(remainder)
}

pub(super) fn luks_uuid_from_unix_device(unix_device: &str) -> Option<String> {
    let name = Path::new(unix_device.trim()).file_name()?.to_str()?;
    let rest = strip_prefix_ignore_ascii_case(name, "luks-")?;
    normalize_luks_uuid(rest)
}

pub(super) fn normalize_luks_uuid(uuid: &str) -> Option<String> {
    let trimmed = uuid.trim();
    if trimmed
        .chars()
        .any(|ch| ch != '-' && !ch.is_ascii_hexdigit())
    {
        return None;
    }
    take_uuid_hex(trimmed).filter(|_| trimmed.chars().filter(|ch| *ch != '-').count() == 32)
}

fn take_uuid_hex(input: &str) -> Option<String> {
    let mut hex = String::new();
    for ch in input.chars() {
        if ch == '-' {
            continue;
        }
        if !ch.is_ascii_hexdigit() {
            break;
        }
        hex.push(ch.to_ascii_lowercase());
        if hex.len() == 32 {
            return Some(hyphenate_uuid_hex(&hex));
        }
    }
    None
}

fn hyphenate_uuid_hex(hex32: &str) -> String {
    format!(
        "{}-{}-{}-{}-{}",
        &hex32[..8],
        &hex32[8..12],
        &hex32[12..16],
        &hex32[16..20],
        &hex32[20..32]
    )
}

fn strip_prefix_ignore_ascii_case<'a>(value: &'a str, prefix: &str) -> Option<&'a str> {
    let (head, tail) = value.split_at_checked(prefix.len())?;
    head.eq_ignore_ascii_case(prefix).then_some(tail)
}

fn icon_name_indicates_encrypted(name: &str) -> bool {
    let stem = icon_name_stem(name);
    stem.contains("encrypted") || stem == "changes-prevent" || stem == "changes-allow"
}

fn icon_name_stem(name: &str) -> String {
    let lower = name.to_ascii_lowercase();
    lower
        .strip_suffix("-symbolic")
        .unwrap_or(&lower)
        .to_string()
}

fn block_indicates_crypto(block: BlockCryptoRef<'_>) -> bool {
    block.dm_uuid.is_some_and(dm_uuid_is_crypt)
        || block
            .id_fs_usage
            .is_some_and(|usage| usage.eq_ignore_ascii_case("crypto"))
        || block
            .id_fs_type
            .is_some_and(|kind| kind.to_ascii_lowercase().starts_with("crypto"))
}

fn dm_uuid_is_crypt(uuid: &str) -> bool {
    uuid.trim()
        .get(..6)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("CRYPT-"))
}

fn sysfs_block_path(unix_device: &str) -> Option<PathBuf> {
    let given = Path::new(unix_device);
    let resolved = std::fs::canonicalize(given).unwrap_or_else(|_| given.to_owned());
    let name = resolved.file_name()?;
    let path = Path::new("/sys/class/block").join(name);
    path.is_dir().then_some(path)
}

fn read_trimmed(path: PathBuf) -> Option<String> {
    let value = std::fs::read_to_string(path).ok()?;
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

fn udev_crypto_properties(data: &str) -> (Option<String>, Option<String>) {
    (
        udev_property(data, "ID_FS_USAGE"),
        udev_property(data, "ID_FS_TYPE"),
    )
}

fn udev_crypto_uuid(data: &str) -> Option<String> {
    let usage = udev_property(data, "ID_FS_USAGE")?;
    if !usage.eq_ignore_ascii_case("crypto") {
        return None;
    }
    udev_property(data, "ID_FS_UUID").and_then(|uuid| normalize_luks_uuid(&uuid))
}

fn slave_crypto_uuid(sysfs: &Path) -> Option<String> {
    let entries = std::fs::read_dir(sysfs.join("slaves")).ok()?;
    for entry in entries.flatten() {
        let slave = Path::new("/sys/class/block").join(entry.file_name());
        let Some(dev) = read_trimmed(slave.join("dev")) else {
            continue;
        };
        let Ok(data) = std::fs::read_to_string(format!("/run/udev/data/b{dev}")) else {
            continue;
        };
        if let Some(uuid) = udev_crypto_uuid(&data) {
            return Some(uuid);
        }
    }
    None
}

fn udev_property(data: &str, key: &str) -> Option<String> {
    let mut prefix = String::from("E:");
    prefix.push_str(key);
    prefix.push('=');
    data.lines()
        .find_map(|line| line.strip_prefix(prefix.as_str()))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

pub(super) fn should_list_orphaned_password_drive(
    start_stop: gio::DriveStartStopType,
    volume_covers_identity: bool,
) -> bool {
    start_stop == gio::DriveStartStopType::Password && !volume_covers_identity
}

pub(super) fn password_drive_is_orphaned(
    start_stop: gio::DriveStartStopType,
    drive_identity: Option<&str>,
    volume_identities: &[String],
    volume_claims_drive: bool,
) -> bool {
    let covered = volume_claims_drive
        || drive_identity.is_some_and(|identity| {
            volume_identities
                .iter()
                .any(|volume_identity| volume_identity == identity)
        });
    should_list_orphaned_password_drive(start_stop, covered)
}

pub(super) fn global_search_roots() -> Vec<PathBuf> {
    let monitor = gio::VolumeMonitor::get();
    let roots = monitor
        .mounts()
        .into_iter()
        .filter(|mount| !mount.is_shadowed())
        .filter_map(|mount| mount.root().path())
        .chain(mounted_devices(&crate::adapters::mount_watch::latest()));
    search_roots(&super::home_directory(), roots)
}

fn search_roots(home: &Path, mounts: impl IntoIterator<Item = PathBuf>) -> Vec<PathBuf> {
    let mut mounts: Vec<_> = mounts
        .into_iter()
        .filter(|root| {
            root.is_absolute()
                && root != home
                && !gio_unix::functions::is_mount_path_system_internal(root)
                && !root.strip_prefix("/run/user").is_ok_and(|relative| {
                    relative
                        .components()
                        .nth(1)
                        .is_some_and(|part| part.as_os_str() == "gvfs")
                })
        })
        .collect();
    mounts.sort();
    mounts.dedup();
    std::iter::once(home.to_owned()).chain(mounts).collect()
}

#[cfg(test)]
mod tests;
