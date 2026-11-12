// SPDX-License-Identifier: MIT

mod install_source;
pub mod integration_catalog;
pub mod package_manager;
mod release_channel;
pub mod update_check;
mod update_install;

pub(crate) use strata_fs::services::*;

pub use install_source::{InstallSource, ManagedInstall};
pub use install_source::{ensure_self_managed, installed_executable};
// `best_update`, `rollback_target`, and `ReleaseSummary` are deliberately not
// re-exported here: `rollback_target` is the never-downgrade bypass, and only
// `update_check` (which imports them directly from `release_channel`) has any
// business calling it. Widening this re-export would make that bypass
// reachable from UI code.
pub use release_channel::{BuildKind, Channel, Version};
pub use update_check::{
    ReleaseChoice, ReleaseMetadata, ReleaseNotes, UpdateCheck, check_for_updates,
    fetch_release_notes, fetch_releases, resolve_release_commit,
};
pub use update_install::{
    InstallCancel, InstallRequest, UpdateInstall, UpdateMethod, install_update, rollback_path,
    update_method,
};
