// SPDX-License-Identifier: MIT

use std::{marker::PhantomData, sync::OnceLock};

use strata_core::{
    logging, metrics,
    util::ui_thread::{self, Exception},
};
use strata_desktop::{
    keys::{self, Xkb, conflicts},
    services::InstallSource,
};

use crate::settings::Saved;

pub(crate) const APPLICATION_ID: &str = "io.github.lgse.Strata";

pub(crate) fn initialize_process() -> Saved {
    glib::set_prgname(Some(APPLICATION_ID));
    glib::set_application_name("Strata");
    metrics::initialize();
    logging::initialize();
    load_settings()
}

/// The saved settings, with their language made the process's.
pub(crate) fn load_settings() -> Saved {
    Saved::load()
}

pub(crate) fn read_install_facts() {
    let _startup = ui_thread::allow(Exception::Startup);
    InstallSource::detect();
}

#[derive(Clone, Copy)]
pub(crate) struct UiThread {
    _thread: PhantomData<*const ()>,
}

impl UiThread {
    pub(crate) fn claim() -> Result<Self, String> {
        install_key_library()?;
        read_install_facts();
        strata_fs::startup::claim_watched_ui_thread();
        let context = glib::MainContext::default();
        let owned = context
            .acquire()
            .map_err(|_| "another thread owns the default GLib context".to_owned())?;
        // The interface thread keeps the context until the process ends.
        std::mem::forget(owned);
        Ok(Self {
            _thread: PhantomData,
        })
    }
}

pub(crate) fn install_key_library() -> Result<(), String> {
    static OURS: OnceLock<bool> = OnceLock::new();
    let install = || {
        let installed = keys::install(&Xkb, conflicts::never_fires);
        tracing::debug!(
            library = ?keys::library_in_use(),
            installed,
            "the key library is Strata's table over libxkbcommon"
        );
        installed
    };
    if *OURS.get_or_init(install) {
        Ok(())
    } else {
        Err("another key library was installed before the interface's own".to_owned())
    }
}

#[cfg(test)]
mod tests;
