// SPDX-License-Identifier: GPL-3.0-or-later

mod file_source;
mod operations;
mod preview;
mod search;
mod update_check;

pub use file_source::{
    DirectoryChange, DirectoryEvent, DirectoryRequest, FileSource, LoadHandle,
    LocationValidationError, RequestId,
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
pub(crate) use update_check::{UpdateCheck, check_for_updates};
