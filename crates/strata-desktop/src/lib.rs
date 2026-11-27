// SPDX-License-Identifier: MIT

// strata-core embeds the catalogs; this path holds none, so the macro adds none itself.
rust_i18n::i18n!(
    "src/no-catalogs",
    fallback = "en",
    backend = strata_core::i18n::Catalogs::compiled()
);

use strata_core::{i18n, model, storage, trusted_command, util};
use strata_fs::adapters;
use strata_preview::sandbox;

pub mod build_info;
pub mod keys;
pub mod portal_setup;
pub mod services;
pub mod settings;
pub mod tabs_session;
#[cfg(test)]
mod test_support;
pub mod window_controls;
