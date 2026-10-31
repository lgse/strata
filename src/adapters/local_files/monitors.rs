// SPDX-License-Identifier: MIT

use std::{
    path::PathBuf,
    sync::{Mutex, OnceLock, mpsc},
};

use futures_channel::oneshot;
use gtk::{gio, glib, prelude::*};

use crate::{
    adapters::{
        gio_file_for_location,
        local_operations::{self, Watched},
        mount_lanes::{self, Class},
    },
    model::Location,
    services::LoadHandle,
};

pub(super) struct BareMonitor {
    monitor: Option<gio::FileMonitor>,
    file: gio::File,
}

#[expect(
    unsafe_code,
    reason = "a monitor without handlers moves between the UI thread and the monitors worker"
)]
// SAFETY: a bare monitor crosses threads only while no signal handler is connected (callers
// disconnect before dropping it), so no `Rc`-capturing closure is freed off the UI thread. GObject
// reference counts and `g_file_monitor_cancel` are thread-safe, and the monitor's source stays
// attached to the default main context.
unsafe impl Send for BareMonitor {}

impl BareMonitor {
    pub(super) fn monitor(&self) -> &gio::FileMonitor {
        self.monitor.as_ref().expect("held until dropped")
    }
}

impl Drop for BareMonitor {
    fn drop(&mut self) {
        if let Some(monitor) = self.monitor.take() {
            let release = BareMonitor {
                monitor: Some(monitor),
                file: self.file.clone(),
            };
            if let Err(mpsc::SendError(Job::Release(mut unsent))) = send(Job::Release(release)) {
                std::mem::forget(unsent.monitor.take());
            }
        }
    }
}

enum Job {
    Start {
        file: gio::File,
        watched: Watched,
        lane: Lane,
        flags: gio::FileMonitorFlags,
        reply: oneshot::Sender<BareMonitor>,
    },
    Release(BareMonitor),
}

enum Lane {
    Native(PathBuf),
    Remote(String),
}

fn send(job: Job) -> Result<(), mpsc::SendError<Job>> {
    static JOBS: OnceLock<Mutex<mpsc::Sender<Job>>> = OnceLock::new();
    let jobs = JOBS.get_or_init(|| {
        let (jobs, received) = mpsc::channel();
        std::thread::Builder::new()
            .name("strata-monitors".to_owned())
            .spawn(move || received.into_iter().for_each(run))
            .expect("spawn the monitors worker");
        Mutex::new(jobs)
    });
    jobs.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .send(job)
}

fn run(job: Job) {
    match job {
        Job::Start {
            file,
            watched,
            lane,
            flags,
            reply,
        } => {
            if reply.is_canceled() {
                return;
            }
            let admitted = match &lane {
                Lane::Native(path) => mount_lanes::admit(path, Class::Interactive),
                Lane::Remote(authority) => {
                    mount_lanes::admit_remote(authority.clone(), Class::Interactive)
                }
            };
            let Ok(admitted) = admitted else {
                return;
            };
            let created = local_operations::watch(&file, watched, flags);
            admitted.progress().answered(&created);
            drop(admitted);
            match created {
                Ok(monitor) => {
                    let _ = reply.send(BareMonitor {
                        monitor: Some(monitor),
                        file,
                    });
                }
                Err(error) => tracing::warn!(
                    error_domain = ?error.domain(),
                    error_code = error.code(),
                    "directory monitoring unavailable"
                ),
            }
        }
        Job::Release(mut bare) => {
            if let Some(monitor) = bare.monitor.take() {
                local_operations::unwatch(&bare.file, monitor);
            }
        }
    }
}

pub(super) fn start(
    location: &Location,
    watched: Watched,
    flags: gio::FileMonitorFlags,
) -> impl std::future::Future<Output = Option<BareMonitor>> + use<> {
    let file = gio_file_for_location(location);
    let lane = match location.native_path() {
        Some(path) => Lane::Native(path.to_path_buf()),
        None => Lane::Remote(mount_lanes::authority(
            location.uri_value().unwrap_or_default(),
        )),
    };
    let (reply, started) = oneshot::channel();
    let sent = send(Job::Start {
        file,
        watched,
        lane,
        flags,
        reply,
    })
    .is_ok();
    async move {
        if !sent {
            return None;
        }
        started.await.ok()
    }
}

pub(crate) fn watch(
    location: &Location,
    watched: Watched,
    changed: impl Fn(&gio::File, Option<&gio::File>) + 'static,
    live: impl FnOnce() + 'static,
) -> LoadHandle {
    let started = start(location, watched, gio::FileMonitorFlags::NONE);
    let held = std::rc::Rc::new(std::cell::RefCell::new(
        None::<(BareMonitor, glib::SignalHandlerId)>,
    ));
    let slot = held.clone();
    let task = glib::MainContext::default().spawn_local(async move {
        if let Some(bare) = started.await {
            let handler = bare
                .monitor()
                .connect_changed(move |_, file, other_file, _| changed(file, other_file));
            slot.replace(Some((bare, handler)));
            live();
        }
    });
    LoadHandle::new(move || {
        task.abort();
        if let Some((bare, handler)) = held.take() {
            bare.monitor().disconnect(handler);
        }
    })
}
