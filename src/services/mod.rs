// SPDX-License-Identifier: GPL-3.0-or-later

mod document;
mod file_source;
mod operations;
mod preview;
mod release_channel;
mod search;
mod update_check;
mod update_install;

pub(crate) use document::{
    DocumentBlock, DocumentLayout, DocumentListChildKind, DocumentSpan, DocumentSpanStyle,
    DocumentTableCellLayout, DocumentUnit, DocumentUnitKind, document_kind, has_web_scheme,
    layout_document, parse_document, parse_markdown,
};
pub use file_source::{
    DirectoryChange, DirectoryEvent, DirectoryRequest, FileSource, LoadHandle,
    LocationValidationError, RequestId, UriCredentials, backend_unavailable_message,
    sanitize_uri_credentials, validate_uri_credentials,
};
pub use operations::{
    ArchiveFormat, CancelledOperation, CompressRequest, CreateDirectoryRequest, CreateFileRequest,
    DeleteRequest, ExtractRequest, OperationEvent, OperationProvider, OperationRequestId,
    PasteItem, PasteRequest, RenameRequest, RestoreRequest, TransferConflict, validate_basename,
};
pub use preview::{
    Preview, PreviewContent, PreviewEvent, PreviewProvider, PreviewRequest, PreviewRequestId,
};
pub(crate) use preview::{content_family, has_plain_text_extension, normalize_preview_text};
// `best_update`, `rollback_target`, and `ReleaseSummary` are deliberately not
// re-exported here: `rollback_target` is the never-downgrade bypass, and only
// `update_check` (which imports them directly from `release_channel`) has any
// business calling it. Widening this re-export would make that bypass
// reachable from UI code.
pub(crate) use release_channel::{BuildKind, Channel, Version};
pub(crate) use search::{SearchEvent, SearchHandle, SearchItem, index_tree};
pub(crate) use update_check::{
    ReleaseMetadata, ReleaseNotes, UpdateCheck, check_for_updates, fetch_release_notes,
};
pub(crate) use update_install::{InstallRequest, UpdateInstall, install_update};
