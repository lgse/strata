// SPDX-License-Identifier: MIT

pub(crate) mod diagnostics;
pub(crate) use strata_fs::services::*;
mod install_source;
pub(crate) mod integration_catalog;
pub(crate) mod package_manager;
mod release_channel;
mod update_check;
mod update_install;

pub use install_source::{InstallSource, ManagedInstall};
pub(crate) use install_source::{ensure_self_managed, installed_executable};
// `best_update`, `rollback_target`, and `ReleaseSummary` are deliberately not
// re-exported here: `rollback_target` is the never-downgrade bypass, and only
// `update_check` (which imports them directly from `release_channel`) has any
// business calling it. Widening this re-export would make that bypass
// reachable from UI code.
pub(crate) use release_channel::{BuildKind, Channel, Version};
pub(crate) use update_check::{
    ReleaseChoice, ReleaseMetadata, ReleaseNotes, UpdateCheck, check_for_updates,
    fetch_release_notes, fetch_releases, resolve_release_commit,
};
pub(crate) use update_install::{
    InstallCancel, InstallRequest, UpdateInstall, UpdateMethod, install_update, rollback_path,
    update_method,
};
