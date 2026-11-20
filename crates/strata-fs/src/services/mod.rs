// SPDX-License-Identifier: MIT

pub(crate) mod actions;
mod dropped_content;
#[cfg(test)]
mod file_activity_tests;
pub mod file_providers;
mod file_source;
mod item_identity;
pub(crate) mod jobs;
pub mod letter_bookmarks;
mod listeners;
mod mime_type;
mod native_fs;
mod navigation_history;
pub mod network_error;
mod operations;
mod path_match;
mod remote_download;
mod search;
mod transfer_action;

pub use strata_core::file_activity;
pub use strata_preview::services::*;

pub use actions::{
    ActionAvailability, ActionCatalog, ActionHandle, ActionLoadFailure, ActionProgram,
    ActionRegistry, ActionScript, ActionWriteRequest, MatchedAction,
};
pub use dropped_content::{
    DroppedFile, MAX_DROPPED_FILE_BYTES, WebLink, has_link_extension, save_dropped_files,
    save_new_file,
};
pub use file_source::{
    DirectoryChange, DirectoryEvent, DirectoryRequest, DirectoryStamp, FileSource, LoadHandle,
    LocationValidationError, MetadataOutcome, MetadataRequest, MetadataUpdate, MountToken,
    RequestId, Responsiveness, StampAnswer, UriCredentials, WatchRequest, Watching,
    backend_unavailable_message, sanitize_uri_credentials, validate_uri_credentials,
};
pub use file_source::{
    INVALID_NAME_REASON, UNEXPECTED_SYSTEM_ERROR, describes_refused_credentials, error_detail,
    gio_backend_error_message, gio_device_error_message, gio_error_detail, gio_error_message,
    io_error_detail, io_error_message, sanitize_failure_message, system_error_text,
};
pub use item_identity::{Attributes, Evidence, IdentityTraits, InodeNumbers, ItemIdentity, Proof};
pub use jobs::{
    ActionEventSink, ActionRunEvent, ActionRunRequest, ActionRunner, CancelHandle,
    InvocationSource, JobId, JobRequest, JobService, JobSnapshot, JobStatus, ScriptProgress,
    expand_command_arguments,
};
pub use listeners::{ListenerGuard, Listeners};
pub use mime_type::{EntryType, FOLDER_TYPE_NAME, OTHER_TYPE_NAME, entry_type};
pub(crate) use native_fs::{
    is_hidden_name, link_target_kind, listed_kind, native_hidden_names, native_kind, symlink_kind,
};
pub use navigation_history::NavigationHistory;
pub use operations::{
    ArchiveFormat, CancelledOperation, CompressRequest, CreateDirectoryRequest, CreateFileRequest,
    DeleteRequest, ExtractRequest, Holder, KeptCause, KeptItem, LeftoverInside, MoveRecord,
    NameMode, OperationEvent, OperationProvider, OperationRequestId, PasswordFailure, PasteItem,
    PasteRequest, PlaceEntryRequest, PlacedSnapshot, RecoverRequest, RenameCause, RenameRecord,
    RenameRequest, RenamedItem, ReplacedOriginal, RestoreRequest, RestoreSource, RestoreTrashItem,
    ReviewedLeftover, ShownName, SkippedItem, SpecialKind, TransferConflict, TransferReport,
    TypedPath, UndoCopyRequest, UndoMergeRequest, UndoMoveItem, UndoMoveRequest, UndoRenameRequest,
    validate_basename,
};
pub use path_match::{PathMatcher, PathQuery};
pub use remote_download::{
    RemoteDownload, download_remote, prune_stale_downloads, remote_file_name, remote_file_url,
};
pub use search::{
    RESULT_LIMIT as SEARCH_RESULT_LIMIT, RenameScope, rebase_search_indexes,
    refresh_search_indexes_for_directory, refresh_search_indexes_for_rename,
};
pub use search::{
    RefusedFolders, SearchCoverage, SearchEvent, SearchExclusions, SearchHandle, SearchItem,
    SearchLimits, filter_name_matches, filter_query_allows_typos, fold_for_search, index_filter,
    index_folder_paths, index_paths, index_trees_with_exclusions,
};
pub use transfer_action::{
    CrossVolumeDropStrategy, DropActionInput, DropCommit, DropOverride, TransferKind,
    VolumeIdentity, VolumeRelation, drop_commit, transferable_drop_sources, volume_relation,
};
