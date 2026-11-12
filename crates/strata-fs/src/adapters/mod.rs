// SPDX-License-Identifier: MIT

pub mod bookmarks;
pub mod directory_summary;
pub mod directory_validation;
mod file_manager1;
pub mod gio_close;
mod gio_location;
mod local_actions;
mod local_files;
mod local_jobs;
mod local_operations;
mod local_preview;
pub mod thumbnail_cache;
pub mod trash;
pub mod trash_restore;
pub mod trash_support;
mod volume;

pub use file_manager1::{RevealRequest, export_file_manager};
pub use gio_location::{
    CONTENT_TYPE_ATTRIBUTES, content_type_of, gio_file_for_location, location_for_file,
    reveal_target_for_file,
};
pub use local_actions::{LocalActionStore, resolve_executable as resolve_action_executable};
pub use local_files::LocalFileSource;
pub use local_files::{RecentRemovalState, recent_remove_entries};
pub use local_files::{info_is_hidden, query_file_entry, watch};
pub use local_jobs::LocalActionRunner;
pub use local_operations::LocalOperationProvider;
pub use local_operations::{
    ARCHIVE_UNSUPPORTED_MESSAGE, ActiveOperation, TransferTargetCollision, flush_filesystem,
    list_archive_entries_direct, transfer_targets, typed_folder_blocked, typed_name_refusal,
    typed_rename_is_taken,
};
#[cfg(any(test, feature = "test-support"))]
pub use local_operations::{
    ArchiveListing, ArchiveListingStatus, decode_archive_listing, write_compression_fixture,
};
#[cfg(any(test, feature = "test-support"))]
pub use local_operations::{Births, FsOp, FsQuirks, Quirks, evict, fs_quirks};
pub use local_operations::{Watched, probes};
#[cfg(any(test, feature = "test-support"))]
pub use local_operations::{
    clear_sync_probe, install_sync_probe, release_sync_probe, sync_probe_is_blocked,
    sync_probe_len, sync_probe_observations,
};
pub use local_preview::LocalPreviewProvider;
#[cfg(any(test, feature = "test-support"))]
pub use volume::set_mountinfo_for_isolated_test;
pub use volume::{DropVolumeQuery, DropVolumes, lookup_drop_volumes};
pub use volume::{
    MountTable, RemovableRoot, invalidate_mount_snapshot, mount_can_unplug, mount_is_removable,
    volume_can_unplug, with_mount_snapshot,
};
pub use volume::{mount_lanes, mount_watch};
