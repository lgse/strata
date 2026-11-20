// SPDX-License-Identifier: MIT

pub mod camera_preview;
mod document;
pub mod document_media;
pub mod docx;
pub mod image_conversion;
pub mod model_preview;
pub(crate) mod odt;
mod preview;
pub mod route_preview;
pub(crate) mod rtf;
pub mod table;

pub use strata_core::file_activity;

pub use document::{
    DocumentBlock, DocumentLayout, DocumentListChildKind, DocumentMedia, DocumentSpan,
    DocumentSpanStyle, DocumentTableCellLayout, DocumentUnit, DocumentUnitKind, document_kind,
    has_web_scheme, layout_document, parse_document, parse_markdown,
};
pub use model_preview::{ModelEncoding, ModelFormat, ModelMesh, ModelPalette, ModelView};
pub use model_preview::{ModelRender, ModelRenderPurpose};
pub use preview::{
    ArchiveDirectory, ArchiveFileEntry, ArchiveNode, ArchivePreviewTree, MediaPreviewSize,
    ModelPreviewStage, PdfTextLayer, Preview, PreviewContent, PreviewEvent, PreviewProvider,
    PreviewRequest, PreviewRequestId, SandboxedMedia, SecretString, archive_preview_tree,
};
pub use preview::{
    INCORRECT_ARCHIVE_PASSWORD, archive_preview_format, bytes_support_named_family, content_family,
    guess_content_type, has_plain_text_extension, is_extensionless_dotfile, is_image_path,
    is_media_path, is_model, is_non_executable_extensionless_dotfile, is_raw_video_stream,
    looks_like_text, model_format, normalize_preview_text, read_head_sample,
    read_supports_named_family, reconcile_with_sample, refine_reported_content_type,
    split_archive_name, supports_remote_video,
};
