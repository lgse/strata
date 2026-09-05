// SPDX-License-Identifier: GPL-3.0-or-later

#[cfg(test)]
mod tests;

use std::{
    cell::RefCell,
    collections::{HashMap, VecDeque},
    os::unix::fs::MetadataExt,
    rc::Rc,
};

use gtk::{gio, glib, prelude::*};

use crate::{model::Location, services::VolumeIdentity};

const LRU_LIMIT: usize = 64;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct CacheKey {
    location: Location,
    follow_symlinks: bool,
}

struct RestatusHook {
    run: Rc<dyn Fn() -> bool>,
}

struct VolumeCache {
    epoch: u64,
    entries: HashMap<CacheKey, VolumeIdentity>,
    order: VecDeque<CacheKey>,
    inflight: HashMap<CacheKey, u64>,
    restatus: Vec<RestatusHook>,
}

impl VolumeCache {
    fn new() -> Self {
        Self {
            epoch: 0,
            entries: HashMap::new(),
            order: VecDeque::new(),
            inflight: HashMap::new(),
            restatus: Vec::new(),
        }
    }

    fn insert(&mut self, key: CacheKey, identity: VolumeIdentity) {
        if self.entries.contains_key(&key) {
            self.order.retain(|existing| existing != &key);
        }
        self.order.push_back(key.clone());
        self.entries.insert(key, identity);
        while self.entries.len() > LRU_LIMIT {
            if let Some(oldest) = self.order.pop_front() {
                self.entries.remove(&oldest);
            }
        }
    }
}

thread_local! {
    static VOLUME: RefCell<VolumeCache> = RefCell::new(VolumeCache::new());
}

pub(crate) fn query_volume_identity(
    location: &Location,
    follow_symlinks: bool,
) -> Option<VolumeIdentity> {
    let identity = query_volume_identity_uncached(location, follow_symlinks)?;
    cache_success(location, follow_symlinks, identity.clone());
    Some(identity)
}

pub(crate) fn cached_volume_identity(
    location: &Location,
    follow_symlinks: bool,
) -> Option<VolumeIdentity> {
    let key = CacheKey {
        location: location.clone(),
        follow_symlinks,
    };
    VOLUME.with(|cache| cache.borrow().entries.get(&key).cloned())
}

pub(crate) fn prefetch_volume_identity(location: Location, follow_symlinks: bool) {
    if location.native_path().is_some() {
        let _ = query_volume_identity(&location, follow_symlinks);
        return;
    }
    let key = CacheKey {
        location: location.clone(),
        follow_symlinks,
    };
    let Some(epoch) = begin_inflight(&key) else {
        return;
    };
    let file = gio_file(&location);
    let flags = query_flags(follow_symlinks);
    glib::MainContext::default().spawn_local(async move {
        let info = file
            .query_info_future(
                gio::FILE_ATTRIBUTE_ID_FILESYSTEM,
                flags,
                glib::Priority::DEFAULT,
            )
            .await;
        let remote = file
            .query_filesystem_info_future(
                gio::FILE_ATTRIBUTE_FILESYSTEM_REMOTE,
                glib::Priority::DEFAULT,
            )
            .await
            .ok();
        let identity = info
            .ok()
            .and_then(|info| identity_from_gio(&location, &info, remote.as_ref()));
        complete_inflight(key, epoch, identity);
    });
}

pub(crate) fn flush_volume_identity_cache() {
    let hooks = VOLUME.with(|cache| {
        let mut cache = cache.borrow_mut();
        cache.epoch = cache.epoch.wrapping_add(1);
        cache.entries.clear();
        cache.order.clear();
        cache.inflight.clear();
        std::mem::take(&mut cache.restatus)
    });
    let live = hooks
        .into_iter()
        .filter(|hook| (hook.run)())
        .collect::<Vec<_>>();
    VOLUME.with(|cache| cache.borrow_mut().restatus.extend(live));
}

pub(crate) fn register_volume_restatus(run: Rc<dyn Fn() -> bool>) {
    VOLUME.with(|cache| cache.borrow_mut().restatus.push(RestatusHook { run }));
}

pub(crate) fn location_is_remote(location: &Location) -> bool {
    match location.native_path() {
        Some(_) => false,
        None => {
            let scheme = location.backend_name();
            scheme != "file" && scheme != "trash"
        }
    }
}

fn query_volume_identity_uncached(
    location: &Location,
    follow_symlinks: bool,
) -> Option<VolumeIdentity> {
    if let Some(path) = location.native_path() {
        let metadata = if follow_symlinks {
            std::fs::metadata(path)
        } else {
            std::fs::symlink_metadata(path)
        };
        let metadata = metadata.ok()?;
        return Some(VolumeIdentity {
            filesystem_id: format!("dev:{}", metadata.dev()),
            is_remote: false,
        });
    }
    let file = gio_file(location);
    let info = file
        .query_info(
            gio::FILE_ATTRIBUTE_ID_FILESYSTEM,
            query_flags(follow_symlinks),
            None::<&gio::Cancellable>,
        )
        .ok()?;
    let remote = file
        .query_filesystem_info(
            gio::FILE_ATTRIBUTE_FILESYSTEM_REMOTE,
            None::<&gio::Cancellable>,
        )
        .ok();
    identity_from_gio(location, &info, remote.as_ref())
}

fn identity_from_gio(
    location: &Location,
    info: &gio::FileInfo,
    remote_info: Option<&gio::FileInfo>,
) -> Option<VolumeIdentity> {
    let filesystem_id = info.attribute_string(gio::FILE_ATTRIBUTE_ID_FILESYSTEM)?;
    if filesystem_id.is_empty() {
        return None;
    }
    let gio_remote = remote_info.is_some_and(|info| {
        info.has_attribute(gio::FILE_ATTRIBUTE_FILESYSTEM_REMOTE)
            && info.boolean(gio::FILE_ATTRIBUTE_FILESYSTEM_REMOTE)
    });
    Some(VolumeIdentity {
        filesystem_id: filesystem_id.to_string(),
        is_remote: location_is_remote(location) || gio_remote,
    })
}

fn begin_inflight(key: &CacheKey) -> Option<u64> {
    VOLUME.with(|cache| {
        let mut cache = cache.borrow_mut();
        if cache.entries.contains_key(key) || cache.inflight.contains_key(key) {
            return None;
        }
        let epoch = cache.epoch;
        cache.inflight.insert(key.clone(), epoch);
        Some(epoch)
    })
}

fn cache_success(location: &Location, follow_symlinks: bool, identity: VolumeIdentity) {
    let key = CacheKey {
        location: location.clone(),
        follow_symlinks,
    };
    VOLUME.with(|cache| cache.borrow_mut().insert(key, identity));
}

fn complete_inflight(key: CacheKey, epoch: u64, identity: Option<VolumeIdentity>) {
    let stale = VOLUME.with(|cache| {
        let mut cache = cache.borrow_mut();
        let started = cache.inflight.remove(&key);
        started != Some(epoch)
    });
    if stale {
        return;
    }
    if let Some(identity) = identity {
        VOLUME.with(|cache| cache.borrow_mut().insert(key, identity));
    }
    run_restatus_hooks();
}

fn run_restatus_hooks() {
    let hooks = VOLUME.with(|cache| std::mem::take(&mut cache.borrow_mut().restatus));
    let live = hooks
        .into_iter()
        .filter(|hook| (hook.run)())
        .collect::<Vec<_>>();
    VOLUME.with(|cache| cache.borrow_mut().restatus.extend(live));
}

fn query_flags(follow_symlinks: bool) -> gio::FileQueryInfoFlags {
    if follow_symlinks {
        gio::FileQueryInfoFlags::NONE
    } else {
        gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS
    }
}

fn gio_file(location: &Location) -> gio::File {
    location
        .native_path()
        .map(gio::File::for_path)
        .unwrap_or_else(|| gio::File::for_uri(location.uri_value().unwrap_or_default()))
}

#[cfg(test)]
pub(crate) fn volume_inflight_len() -> usize {
    VOLUME.with(|cache| cache.borrow().inflight.len())
}

#[cfg(test)]
pub(crate) fn volume_cache_len() -> usize {
    VOLUME.with(|cache| cache.borrow().entries.len())
}

#[cfg(test)]
pub(crate) fn volume_epoch() -> u64 {
    VOLUME.with(|cache| cache.borrow().epoch)
}
