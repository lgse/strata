// SPDX-License-Identifier: MIT

//! What the process does before the interface asks for its first listing. `src/main.rs` does
//! the same for the GTK application.

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

/// What the window reports to the desktop. Not the GTK application's id: both are installed
/// until parity, and a window rule or a desktop entry for one must not match the other.
pub(crate) const APPLICATION_ID: &str = "io.github.lgse.Strata.Rune";

/// Identity, logging and the saved settings: what needs no particular thread, and reads its
/// files before one becomes the UI thread.
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

/// Reads process-wide install facts once under [`Exception::Startup`] before the calling thread
/// becomes the UI thread, mirroring GTK's `ui::read_install_facts` (`src/ui/mod.rs`).
pub(crate) fn read_install_facts() {
    let _startup = ui_thread::allow(Exception::Startup);
    InstallSource::detect();
}

/// Proof that the calling thread ran the start-up sequence and owns the default GLib context,
/// which `Browser` schedules on. Building the application state takes one.
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

/// Makes the shared key catalog answerable in this process: `strata_desktop::keys` asks an
/// installed key library for a key's name, character and case, and this interface's is
/// `Xkb`: Strata's own table for the keys it declares and libxkbcommon for the rest, each
/// answer the one GDK gives the GTK application. Once for the process, however many windows
/// or hosts claim the UI thread. An error means another library was installed first, so keys
/// would be read by rules this interface did not choose.
///
/// Which libxkbcommon the process runs on is logged, at debug: the backend's rules are exact
/// for one version of it, and a report that a key reads differently here and in the GTK
/// application starts with that line.
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
