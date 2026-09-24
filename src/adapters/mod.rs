// SPDX-License-Identifier: MIT

pub(crate) mod directory_summary;
mod file_manager1;
mod gio_location;
mod local_actions;
mod local_files;
mod local_jobs;
mod local_operations;
mod local_preview;
pub(crate) mod trash;
pub(crate) mod trash_restore;
mod volume;

pub(crate) use file_manager1::{RevealRequest, export_file_manager};
pub(crate) use gio_location::{gio_file_for_location, location_for_file};
pub(crate) use local_actions::{LocalActionStore, resolve_executable as resolve_action_executable};
pub use local_files::LocalFileSource;
pub(crate) use local_files::query_file_entry;
pub(crate) use local_jobs::LocalActionRunner;
pub use local_operations::LocalOperationProvider;
pub(crate) use local_operations::{
    ARCHIVE_PREVIEW_FAILED_MESSAGE, ARCHIVE_TOO_LARGE_MESSAGE, ARCHIVE_UNSUPPORTED_MESSAGE,
    INVALID_ARCHIVE, MAX_ARCHIVE_PASSWORD_BYTES, archive_payload_valid, encode_archive_result,
    flush_filesystem, list_archive_entries_direct,
};
#[cfg(test)]
pub(crate) use local_operations::{
    ArchiveListing, ArchiveListingStatus, decode_archive_listing, write_compression_fixture,
};
#[cfg(test)]
pub(crate) use local_operations::{
    clear_sync_probe, install_sync_probe, release_sync_probe, sync_probe_is_blocked,
    sync_probe_len, sync_probe_observations,
};
pub use local_preview::LocalPreviewProvider;
pub(crate) use volume::MountTable;
pub(crate) use volume::{DropVolumeQuery, DropVolumes, lookup_drop_volumes};
