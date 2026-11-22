// SPDX-License-Identifier: MIT

//! Block-device facts read off the UI thread and kept until the volume monitor reports changes
//! (`src/ui/window/devices/probe_cache.rs` and `held.rs`).

use std::{
    cell::{OnceCell, RefCell},
    collections::{HashMap, HashSet},
    thread::LocalKey,
};

use gio::prelude::*;
use strata_fs::adapters::block_device_is_exclusively_held;

use super::release::refresh_all_sidebars;

pub(super) struct ProbeCache<T> {
    known: RefCell<HashMap<String, T>>,
    pending: RefCell<HashSet<String>>,
    monitor: OnceCell<gio::VolumeMonitor>,
}

impl<T> Default for ProbeCache<T> {
    fn default() -> Self {
        Self {
            known: RefCell::default(),
            pending: RefCell::default(),
            monitor: OnceCell::new(),
        }
    }
}

pub(super) trait Fact: Clone + Default + PartialEq + Send + 'static {}

impl<T: Clone + Default + PartialEq + Send + 'static> Fact for T {}

type Cache<T> = &'static LocalKey<ProbeCache<T>>;

thread_local! {
    static HELD: ProbeCache<bool> = ProbeCache::default();
}

/// Returns whether a block device is currently held exclusively (`O_EXCL` -> `EBUSY`), probed
/// off the UI thread and cached until GIO reports device changes.
pub(super) fn is_exclusively_held(unix_device: &str) -> bool {
    if cfg!(test) {
        return false;
    }
    cached(&HELD, unix_device, block_device_is_exclusively_held)
}

/// Returns the cached fact for `unix_device`, or its default while the first background probe
/// runs. When the probe changes the cached value, every open sidebar refreshes.
pub(super) fn cached<T: Fact>(cache: Cache<T>, unix_device: &str, probe: fn(&str) -> T) -> T {
    if cfg!(test) {
        return T::default();
    }
    cache.with(|state| {
        state.monitor.get_or_init(|| {
            let monitor = gio::VolumeMonitor::get();
            let recheck = move || recheck(cache, probe);
            monitor.connect_volume_added(move |_, _| recheck());
            monitor.connect_volume_removed(move |_, _| recheck());
            monitor.connect_volume_changed(move |_, _| recheck());
            monitor.connect_drive_connected(move |_, _| recheck());
            monitor.connect_drive_disconnected(move |_, _| recheck());
            monitor.connect_drive_changed(move |_, _| recheck());
            monitor
        });
    });
    let known = cache.with(|state| state.known.borrow().get(unix_device).cloned());
    known.unwrap_or_else(|| {
        start(cache, unix_device.to_owned(), probe);
        T::default()
    })
}

fn recheck<T: Fact>(cache: Cache<T>, probe: fn(&str) -> T) {
    let devices: Vec<String> = cache.with(|state| state.known.borrow().keys().cloned().collect());
    for device in devices {
        start(cache, device, probe);
    }
}

fn start<T: Fact>(cache: Cache<T>, unix_device: String, probe: fn(&str) -> T) {
    if !cache.with(|state| state.pending.borrow_mut().insert(unix_device.clone())) {
        return;
    }
    glib::MainContext::default().spawn_local(async move {
        let device = unix_device.clone();
        let fact = gio::spawn_blocking(move || probe(&device))
            .await
            .unwrap_or_default();
        let changed = cache.with(|state| {
            state.pending.borrow_mut().remove(&unix_device);
            let seen = state
                .known
                .borrow_mut()
                .insert(unix_device, fact.clone())
                .unwrap_or_default();
            seen != fact
        });
        if changed {
            refresh_all_sidebars();
        }
    });
}
