// SPDX-License-Identifier: GPL-3.0-or-later

mod file_source;
mod install_source;
mod operations;
mod preview;
mod release_channel;
mod search;
mod update_check;
mod update_install;

pub use file_source::{
    DirectoryChange, DirectoryEvent, DirectoryRequest, FileSource, LoadHandle,
    LocationValidationError, MetadataOutcome, MetadataRequest, MetadataUpdate, RequestId,
    UriCredentials, backend_unavailable_message, sanitize_uri_credentials,
    validate_uri_credentials,
};
pub(crate) use install_source::ensure_self_managed;
pub use install_source::{InstallSource, ManagedInstall};
pub use operations::{
    ArchiveFormat, CancelledOperation, CompressRequest, CreateDirectoryRequest, CreateFileRequest,
    DeleteRequest, ExtractRequest, OperationEvent, OperationProvider, OperationRequestId,
    PasteItem, PasteRequest, RenameRequest, RestoreRequest, RestoreSource, TransferConflict,
    validate_basename,
};
pub use preview::{
    Preview, PreviewContent, PreviewEvent, PreviewProvider, PreviewRequest, PreviewRequestId,
};
pub(crate) use preview::{
    content_family, has_plain_text_extension, is_extensionless_dotfile,
    is_non_executable_extensionless_dotfile,
};
// `best_update`, `rollback_target`, and `ReleaseSummary` are deliberately not
// re-exported here: `rollback_target` is the never-downgrade bypass, and only
// `update_check` (which imports them directly from `release_channel`) has any
// business calling it. Widening this re-export would make that bypass
// reachable from UI code.
pub(crate) use release_channel::{BuildKind, Channel, Version};
pub(crate) use search::{SearchEvent, SearchHandle, SearchItem, index_tree};
pub(crate) use update_check::{
    ReleaseMetadata, ReleaseNoteBlock, ReleaseNotes, UpdateCheck, check_for_updates,
    fetch_release_notes,
};
pub(crate) use update_install::{
    InstallRequest, UpdateInstall, UpdateMethod, install_update, update_method,
};
