// SPDX-License-Identifier: MIT

//! What the process does before the interface asks for its first listing. `src/main.rs` does
//! the same for the GTK application.

use std::marker::PhantomData;

use strata_core::{logging, metrics};

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

/// Proof that the calling thread ran the start-up sequence and owns the default GLib context,
/// which `Browser` schedules on. Building the application state takes one.
pub(crate) struct UiThread {
    _thread: PhantomData<*const ()>,
}

impl UiThread {
    pub(crate) fn claim() -> Result<Self, String> {
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
