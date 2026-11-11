// SPDX-License-Identifier: MIT

//! Desktop integration without an interface toolkit: portal and D-Bus setup, the updater,
//! bundled integrations, the saved settings, and what this build of the application is.

// strata-core embeds the catalogs; this path holds none, so the macro adds none itself.
rust_i18n::i18n!(
    "src/no-catalogs",
    fallback = "en",
    backend = strata_core::i18n::Catalogs::compiled()
);

use strata_core::{i18n, model, storage, trusted_command, util};
use strata_preview::sandbox;

pub mod build_info;
pub mod portal_setup;
pub mod services;
pub mod settings;
