// SPDX-License-Identifier: MIT

//! Listing, file operations, trash, volumes, search, jobs and the browser's state, on GIO
//! and GLib without an interface toolkit.

// strata-core embeds the catalogs; this path holds none, so the macro adds none itself.
rust_i18n::i18n!(
    "src/no-catalogs",
    fallback = "en",
    backend = strata_core::i18n::Catalogs::compiled()
);

use strata_core::{error_text, i18n, metrics, model, storage, util};
#[cfg(feature = "rar")]
use strata_preview::rar_extraction;
use strata_preview::sandbox;
#[cfg(any(test, feature = "test-support"))]
use strata_preview::sandbox_helper;

pub mod adapters;
pub mod app;
pub mod child_environment;
pub mod services;
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
