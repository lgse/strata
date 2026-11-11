// SPDX-License-Identifier: MIT

//! Strata's interface on Rune: the application state, its view, and the hosts that run it
//! beside the GLib context Strata's `Browser` schedules on.
//!
//! `docs/rune/slice/V5-shell-and-loop.md` describes the loop and what each host does.

// strata-core embeds the catalogs; this path holds none, so the macro adds none itself.
rust_i18n::i18n!(
    "src/no-catalogs",
    fallback = "en",
    backend = strata_core::i18n::Catalogs::compiled()
);

mod events;
mod host;
mod launch;
mod shell;
mod startup;

pub use launch::run;
