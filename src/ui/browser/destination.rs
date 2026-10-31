// SPDX-License-Identifier: MIT

use std::path::{Path, PathBuf};

use futures_lite::future::BoxedLocal;

use crate::{adapters::probes, services::LocationValidationError};

pub(super) fn canonical_existing_directory(path: &Path) -> Option<PathBuf> {
    crate::util::ui_thread::assert_off();
    let canonical = std::fs::canonicalize(path).ok()?;
    canonical.is_dir().then_some(canonical)
}

pub(super) fn canonical_directory_within(root: &Path, candidate: &Path) -> Option<PathBuf> {
    let root = canonical_existing_directory(root)?;
    let candidate = canonical_existing_directory(candidate)?;
    candidate.strip_prefix(root).ok()?;
    Some(candidate)
}

pub(super) fn existing_folder(path: &Path) -> BoxedLocal<Result<PathBuf, String>> {
    let path = path.to_path_buf();
    Box::pin(async move {
        match probes::destination_folder(&path).await {
            Ok(_) => Ok(path),
            Err(LocationValidationError::Missing | LocationValidationError::NotDirectory) => {
                Err(crate::i18n::tr("Choose an existing folder."))
            }
            Err(error) => Err(error.message()),
        }
    })
}
