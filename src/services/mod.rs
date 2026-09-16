// SPDX-License-Identifier: MIT

pub(crate) mod camera_preview;
mod document;
mod file_source;
mod install_source;
mod mime_type;
mod native_fs;
mod operations;
mod preview;
mod release_channel;
mod search;
mod transfer_action;
mod update_check;
mod update_install;

pub(crate) use document::{
    DocumentBlock, DocumentLayout, DocumentListChildKind, DocumentSpan, DocumentSpanStyle,
    DocumentTableCellLayout, DocumentUnit, DocumentUnitKind, document_kind, has_web_scheme,
    layout_document, parse_document, parse_markdown,
};
pub use file_source::{
    DirectoryChange, DirectoryEvent, DirectoryRequest, FileSource, LoadHandle,
    LocationValidationError, MetadataOutcome, MetadataRequest, MetadataUpdate, RequestId,
    UriCredentials, backend_unavailable_message, sanitize_uri_credentials,
    validate_uri_credentials,
};
pub(crate) use install_source::ensure_self_managed;
pub use install_source::{InstallSource, ManagedInstall};
pub use mime_type::{
    BROKEN_LINK_TYPE_NAME, EntryType, FOLDER_TYPE_NAME, OTHER_TYPE_NAME, entry_type,
    entry_type_description, mime_description_for_name,
};
pub(crate) use native_fs::{is_hidden_name, native_hidden_names, native_kind};
pub use operations::{
    ArchiveFormat, CancelledOperation, CompressRequest, CreateDirectoryRequest, CreateFileRequest,
    DeleteRequest, ExtractRequest, MoveRecord, OperationEvent, OperationProvider,
    OperationRequestId, PasteItem, PasteRequest, RenameRecord, RenameRequest, RestoreRequest,
    RestoreSource, RestoreTrashItem, TransferConflict, UndoCopyRequest, UndoMoveItem,
    UndoMoveRequest, UndoRenameRequest, validate_basename,
};
pub use preview::{
    MediaPreviewSize, Preview, PreviewContent, PreviewEvent, PreviewProvider, PreviewRequest,
    PreviewRequestId, SandboxedMedia,
};
pub(crate) use preview::{
    content_family, has_plain_text_extension, is_extensionless_dotfile, is_image_path,
    is_media_path, is_non_executable_extensionless_dotfile, normalize_preview_text,
    supports_remote_video,
};
pub(crate) use transfer_action::{
    CrossVolumeDropStrategy, DropActionInput, DropCommit, DropOverride, TransferKind,
    VolumeIdentity, VolumeRelation, drop_commit, transferable_drop_sources, volume_relation,
};
// `best_update`, `rollback_target`, and `ReleaseSummary` are deliberately not
// re-exported here: `rollback_target` is the never-downgrade bypass, and only
// `update_check` (which imports them directly from `release_channel`) has any
// business calling it. Widening this re-export would make that bypass
// reachable from UI code.
pub(crate) use release_channel::{BuildKind, Channel, Version};
pub(crate) use search::{
    SearchCoverage, SearchEvent, SearchHandle, SearchItem, filter_name_matches, fold_for_search,
    index_filter, index_tree, index_trees,
};
pub(crate) use update_check::{
    ReleaseMetadata, ReleaseNotes, UpdateCheck, check_for_updates, fetch_release_notes,
};
pub(crate) use update_install::{
    InstallRequest, UpdateInstall, UpdateMethod, install_update, update_method,
};
