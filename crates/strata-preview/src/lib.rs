// SPDX-License-Identifier: MIT

//! The sandbox launcher, the helper process it runs, and the parsers and types of previews.

// strata-core embeds the catalogs; this path holds none, so the macro adds none itself.
rust_i18n::i18n!(
    "src/no-catalogs",
    fallback = "en",
    backend = strata_core::i18n::Catalogs::compiled()
);

use strata_core::{error_text, i18n, model, storage, trusted_command, util};

pub mod media;
#[cfg(feature = "rar")]
pub mod rar_extraction;
pub mod sandbox;
pub mod sandbox_helper;
pub mod services;
