// SPDX-License-Identifier: MIT

//! Block-device facts read off the UI thread and kept until the devices change.

use std::{
    cell::{OnceCell, RefCell},
    collections::{HashMap, HashSet},
    thread::LocalKey,
};

use gtk::{gio, prelude::*};

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

/// The known fact, or its default while the first probe runs. The sidebars
/// rebuild when a probe changes the answer.
pub(super) fn cached<T: Fact>(cache: Cache<T>, unix_device: &str, probe: fn(&str) -> T) -> T {
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
    gtk::glib::MainContext::default().spawn_local(async move {
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
            super::super::device_release::refresh_sidebars();
        }
    });
}
