// SPDX-License-Identifier: GPL-3.0-or-later

mod file_source;
mod operations;
mod preview;
mod search;
mod update_check;
mod update_install;

pub use file_source::{
    DirectoryChange, DirectoryEvent, DirectoryRequest, FileSource, LoadHandle,
    LocationValidationError, RequestId, backend_unavailable_message, uri_has_embedded_password,
};
pub use operations::{
    CreateDirectoryRequest, CreateFileRequest, DeleteRequest, OperationEvent, OperationProvider,
    OperationRequestId, PasteItem, PasteRequest, RenameRequest, RestoreRequest, TransferConflict,
    validate_basename,
};
pub use preview::{
    Preview, PreviewContent, PreviewEvent, PreviewProvider, PreviewRequest, PreviewRequestId,
};
pub(crate) use preview::{content_family, has_plain_text_extension};
pub(crate) use search::{SearchEvent, SearchHandle, SearchItem, index_tree};
pub(crate) use update_check::{
    ReleaseMetadata, ReleaseNoteBlock, ReleaseNotes, UpdateCheck, check_for_updates,
    fetch_release_notes,
};
pub(crate) use update_install::{UpdateInstall, install_update};
