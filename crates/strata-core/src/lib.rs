// SPDX-License-Identifier: MIT

//! Strata's model and the services every other crate builds on: localization, storage,
//! logging and file-activity tracking. No interface toolkit.

// build.rs embeds data/locales; this path holds no catalogs, so the macro adds none itself.
rust_i18n::i18n!(
    "src/i18n/no-catalogs",
    fallback = "en",
    backend = crate::i18n::Catalogs::compiled()
);

pub mod error_text;
pub mod file_activity;
pub mod i18n;
pub mod logging;
pub mod metrics;
pub mod model;
pub mod storage;
pub mod trusted_command;
pub mod util;
