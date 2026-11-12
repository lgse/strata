// SPDX-License-Identifier: MIT

use std::{
    fs, io,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

#[cfg(any(test, feature = "rar"))]
use std::process::Command;

use crate::services::{ModelPreviewStage, model_preview::MAX_MODEL_INPUT_BYTES};

#[cfg(feature = "rar")]
pub(crate) mod archive;
pub(crate) mod browser;
pub(crate) mod media;
pub(crate) mod metadata;
mod model;
pub(crate) mod raw_metadata;
pub(crate) mod runtime;

mod command;
mod operation;
mod output;
mod process;

#[cfg(test)]
use command::polaris_gpu_available_at;
#[cfg(any(test, feature = "rar"))]
use command::sandbox_input_path;
use command::{SandboxCommand, runtime_command, sandbox_command};
pub(crate) use command::{gpu_devices, numbered_name, polaris_gpu_available, sandbox_tool};
pub(crate) use operation::{
    CoverFormat, DocumentFormat, MediaPreviewBackend, PDF_PASSWORD_PROTECTED, ParseOperation,
    PdfPreviewError, PdfRenderSize,
};
pub(crate) use output::png_dimensions;
#[cfg(test)]
use output::{NEXT_DIRECTORY, OutputError, prune_orphaned_private_outputs};
use output::{
    PrivateOutput, archive_output_error, read_metadata, read_private_output, valid_output,
};
pub(crate) use process::Renderer;
use process::{
    kill_renderer, monitor_renderer, spawn_renderer, terminate, wait_for_renderer,
    wait_for_renderer_reporting,
};

const WALL_TIME_LIMIT: Duration = Duration::from_secs(12);
const ADDRESS_SPACE_LIMIT_BYTES: u64 = 2 * 1024 * 1024 * 1024;
pub(crate) const FILE_SIZE_LIMIT_BYTES: u64 = 512 * 1024 * 1024;
const TEMPORARY_STORAGE_LIMIT_BYTES: u64 = 512 * 1024 * 1024;
const MAX_RASTER_INPUT_BYTES: u64 = 512 * 1024 * 1024;
pub(crate) const MAX_OUTPUT_BYTES: u64 = 32 * 1024 * 1024;
pub(crate) const MAX_TEXT_LAYER_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Clone, Default)]
pub(crate) struct Cancellation(Arc<AtomicBool>);

impl From<Arc<AtomicBool>> for Cancellation {
    fn from(flag: Arc<AtomicBool>) -> Self {
        Self(flag)
    }
}

impl Cancellation {
    pub(crate) fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

pub(crate) struct ParseOutput {
    pub(crate) data: Vec<u8>,
    pub(crate) page: i32,
    pub(crate) pages: i32,
    pub(crate) text_layer: Option<crate::services::PdfTextLayer>,
    pub(crate) model_mesh: Option<crate::services::ModelMesh>,
}

/// One sandboxed parse: the input, what to produce from it, and how to stop.
pub(crate) struct ParseRequest<'a> {
    pub(crate) input: &'a Path,
    pub(crate) operation: ParseOperation,
    pub(crate) value: i32,
    pub(crate) media_backend: MediaPreviewBackend,
    pub(crate) cancellation: &'a Cancellation,
}

pub(crate) fn parse(
    input: &Path,
    operation: ParseOperation,
    value: i32,
    media_backend: MediaPreviewBackend,
    cancellation: &Cancellation,
) -> Result<ParseOutput, String> {
    let request = ParseRequest {
        input,
        operation,
        value,
        media_backend,
        cancellation,
    };
    parse_with_progress(request, &|_| {})
}

pub(crate) fn parse_with_progress(
    request: ParseRequest<'_>,
    progress: &dyn Fn(ModelPreviewStage),
) -> Result<ParseOutput, String> {
    let archive = matches!(request.operation, ParseOperation::ArchiveList { .. });
    let cancellation = request.cancellation;
    match parse_sandboxed(request, progress) {
        Err(error) if archive && hides_archive_failure(cancellation, &error) => {
            Err(crate::adapters::ARCHIVE_PREVIEW_FAILED_MESSAGE.to_owned())
        }
        result => result,
    }
}

/// Archive failures other than the documented contract messages are reported generically.
fn hides_archive_failure(cancellation: &Cancellation, message: &str) -> bool {
    !cancellation.is_cancelled() && !is_archive_contract_message(message)
}

fn is_archive_contract_message(message: &str) -> bool {
    message == crate::adapters::INVALID_ARCHIVE
        || message == crate::adapters::ARCHIVE_TOO_LARGE_MESSAGE
        || message == "Preview cancelled"
        || message == crate::services::file_activity::HELD_FOR_OPERATION
}

fn parse_sandboxed(
    request: ParseRequest<'_>,
    progress: &dyn Fn(ModelPreviewStage),
) -> Result<ParseOutput, String> {
    if request.cancellation.is_cancelled() {
        return Err("Preview cancelled".to_owned());
    }
    if request.operation.is_media() {
        return Err("Media previews require a decoded-frame session".into());
    }
    let canonical = request
        .input
        .canonicalize()
        .map_err(|error| crate::services::io_error_message(&error))?;
    let stop = request.cancellation.clone();
    let hold = Arc::new(crate::services::file_activity::hold(
        [request.input, &canonical],
        move || stop.cancel(),
    )?);
    let request = ParseRequest {
        input: &canonical,
        ..request
    };
    hold.settle(parse_canonical(request, progress, Arc::clone(&hold)))
}

fn input_limit_message(operation: &ParseOperation) -> String {
    match operation {
        ParseOperation::InspectImage | ParseOperation::ConvertImage => {
            "The image exceeds the 32 MiB conversion limit. Choose a smaller image.".to_owned()
        }
        ParseOperation::PreviewRoute(_) => {
            "This route file exceeds the 32 MiB preview limit.".to_owned()
        }
        ParseOperation::PreviewModel(_) => {
            const _: () = assert!(
                MAX_MODEL_INPUT_BYTES == 128 * 1024 * 1024,
                "update the model size limit message and its catalog entry"
            );
            "This model file exceeds the 128 MiB preview limit. Try a smaller or lower-detail version.".to_owned()
        }
        _ => "Preview input exceeds the supported size limit".to_owned(),
    }
}

fn check_input(input: &Path, operation: &ParseOperation) -> Result<(), String> {
    let input_metadata =
        fs::metadata(input).map_err(|error| crate::services::io_error_message(&error))?;
    if !input_metadata.is_file() {
        return Err("Preview input is not a regular file".to_owned());
    }
    if operation
        .input_size_limit()
        .is_some_and(|limit| input_metadata.len() > limit)
    {
        return Err(input_limit_message(operation));
    }
    Ok(())
}

fn archive_secret(operation: &ParseOperation) -> Result<Option<rustix::fd::OwnedFd>, String> {
    let ParseOperation::ArchiveList {
        password: Some(password),
        ..
    } = operation
    else {
        return Ok(None);
    };
    stage_secret_anon(password.expose().as_bytes()).map(Some)
}

fn spawn_sandboxed(request: &ParseRequest<'_>, output: &PrivateOutput) -> Result<Renderer, String> {
    let secret = archive_secret(&request.operation)?;
    let current_executable = std::env::current_exe()
        .map_err(|error| format!("Unable to locate the Strata executable: {error}"))?;
    let running_executable = PathBuf::from(format!("/proc/{}/exe", std::process::id()));
    let executable =
        resolve_renderer_executable(&current_executable, &running_executable, output.path())?;
    let bwrap = crate::trusted_command::resolve_unprivileged("bwrap")
        .map_err(|error| sandbox_unavailable(&error))?;
    let devices = Vec::new();
    let mut command = SandboxCommand {
        bwrap: &bwrap,
        executable: &executable,
        input: request.input,
        output: output.path(),
        operation: request.operation.clone(),
        value: request.value,
        media_backend: request.media_backend,
        devices: &devices,
    }
    .build();
    if let Some(secret) = secret {
        // Command duplicates only this child's stdin; concurrent spawns cannot inherit the secret.
        command.stdin(Stdio::from(fs::File::from(secret)));
        command.arg("0");
    }
    command.stderr(Stdio::null());
    command.stdout(Stdio::null());
    let child = spawn_renderer(&mut command).map_err(|error| sandbox_start_failed(&error))?;
    drop(command);
    Ok(child)
}

fn renderer_timeout(operation: &ParseOperation) -> Duration {
    match operation {
        ParseOperation::DocumentImage
        | ParseOperation::DocumentMermaid
        | ParseOperation::DocumentMath { .. } => Duration::from_secs(3),
        _ => WALL_TIME_LIMIT,
    }
}

fn is_model_thumbnail(operation: &ParseOperation) -> bool {
    matches!(operation, ParseOperation::PreviewModel(render)
        if render.purpose == crate::services::ModelRenderPurpose::Thumbnail)
}

/// Forwards each new, plausible model stage the helper reports while it runs.
struct ModelProgress<'a> {
    enabled: bool,
    output: &'a Path,
    previous: Option<ModelPreviewStage>,
    progress: &'a dyn Fn(ModelPreviewStage),
}

impl ModelProgress<'_> {
    fn reported_stage(&self) -> Option<ModelPreviewStage> {
        let bytes = read_private_output(&self.output.join("result.progress"), 128).ok()?;
        let stage = serde_json::from_slice::<ModelPreviewStage>(&bytes).ok()?;
        let implausible = matches!(stage, ModelPreviewStage::Rendering { triangles }
            if triangles > crate::services::model_preview::MAX_MODEL_TRIANGLES);
        (!implausible).then_some(stage)
    }

    fn poll(&mut self) {
        if !self.enabled {
            return;
        }
        let Some(stage) = self.reported_stage() else {
            return;
        };
        if self.previous != Some(stage) {
            self.previous = Some(stage);
            (self.progress)(stage);
        }
    }
}

fn reports_failure(operation: &ParseOperation) -> bool {
    matches!(
        operation,
        ParseOperation::PreviewModel(_)
            | ParseOperation::PreviewRoute(_)
            | ParseOperation::InspectImage
            | ParseOperation::ConvertImage
            | ParseOperation::PreviewPdf(_)
    )
}

fn reported_failure(operation: &ParseOperation, data: Vec<u8>) -> Option<String> {
    if !matches!(operation, ParseOperation::PreviewPdf(_)) {
        return String::from_utf8(data).ok();
    }
    matches!(
        serde_json::from_slice::<PdfPreviewError>(&data),
        Ok(PdfPreviewError::PasswordProtected)
    )
    .then(|| PDF_PASSWORD_PROTECTED.to_owned())
}

fn renderer_failure(operation: &ParseOperation, output: &Path) -> String {
    if !reports_failure(operation) {
        return "The sandboxed preview renderer failed".to_owned();
    }
    read_private_output(&output.join("result.error"), 512)
        .ok()
        .and_then(|data| reported_failure(operation, data))
        .unwrap_or_else(|| "The sandboxed preview renderer failed".to_owned())
}

fn output_limit(operation: &ParseOperation) -> u64 {
    match operation {
        ParseOperation::InspectImage => 32,
        ParseOperation::MediaMetadata | ParseOperation::AudioTags | ParseOperation::RawMetadata => {
            metadata::MAX_METADATA_BYTES
        }
        _ => MAX_OUTPUT_BYTES,
    }
}

fn read_output_data(operation: &ParseOperation, output: &Path) -> Result<Vec<u8>, String> {
    let is_archive = matches!(operation, ParseOperation::ArchiveList { .. });
    let result_path = output.join(operation.output_name());
    let data = read_private_output(&result_path, output_limit(operation)).map_err(|error| {
        if is_archive {
            archive_output_error(&error)
        } else {
            error.message()
        }
    })?;
    if !valid_output(operation.clone(), &data) {
        return Err(if is_archive {
            crate::adapters::INVALID_ARCHIVE.to_owned()
        } else {
            "The preview renderer produced invalid image data".to_owned()
        });
    }
    Ok(data)
}

fn read_text_layer(
    operation: &ParseOperation,
    output: &Path,
) -> Option<crate::services::PdfTextLayer> {
    if !matches!(operation, ParseOperation::PreviewPdf(_)) {
        return None;
    }
    read_private_output(&output.join("result.text"), MAX_TEXT_LAYER_BYTES)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
}

fn read_parse_output(operation: ParseOperation, output: &Path) -> Result<ParseOutput, String> {
    let data = read_output_data(&operation, output)?;
    let (page, pages) = read_metadata(&output.join("result.meta"));
    let model_mesh = if let ParseOperation::PreviewModel(render) = operation {
        model::read_mesh(render, &output.join("result.model"))?
    } else {
        None
    };
    // A page without an extractable text layer (scanned images, malformed layout)
    // still previews; selection just stays unavailable there.
    let text_layer = read_text_layer(&operation, output);
    Ok(ParseOutput {
        data,
        page,
        pages,
        text_layer,
        model_mesh,
    })
}

fn browser_output(data: Vec<u8>) -> ParseOutput {
    ParseOutput {
        data,
        page: 0,
        pages: 0,
        text_layer: None,
        model_mesh: None,
    }
}

fn parse_canonical(
    request: ParseRequest<'_>,
    progress: &dyn Fn(ModelPreviewStage),
    hold: Arc<crate::services::file_activity::Hold>,
) -> Result<ParseOutput, String> {
    let ParseRequest {
        input,
        ref operation,
        cancellation,
        ..
    } = request;
    check_input(input, operation)?;
    if let Some(result) = browser::preview(input, operation, cancellation) {
        return result.map(browser_output);
    }
    let output = Arc::new(
        PrivateOutput::create().map_err(|error| crate::services::io_error_message(&error))?,
    );
    let mut child = spawn_sandboxed(&request, &output)?;
    let timeout = renderer_timeout(operation);
    let mut model_progress = ModelProgress {
        enabled: matches!(operation, ParseOperation::PreviewModel(_)),
        output: output.path(),
        previous: None,
        progress,
    };
    let mut poll_progress = || model_progress.poll();
    let status = if is_model_thumbnail(operation) {
        model::wait_for_thumbnail(
            child,
            cancellation,
            timeout,
            &mut poll_progress,
            Arc::clone(&output),
            hold,
        )?
    } else {
        wait_for_renderer_reporting(&mut child, cancellation, timeout, &mut poll_progress)?
    };
    if !status.success() {
        return Err(renderer_failure(operation, output.path()));
    }
    read_parse_output(request.operation, output.path())
}

// A memfd avoids named-file residue and dependence on TMPDIR's O_TMPFILE support.
// CLOEXEC prevents unrelated children from inheriting it before the renderer spawns.
pub(crate) fn stage_secret_anon(secret: &[u8]) -> Result<rustix::fd::OwnedFd, String> {
    use rustix::fs::{MemfdFlags, Mode, fchmod, memfd_create};
    use std::io::{Seek, Write};

    let fd = memfd_create(c"strata-preview-password", MemfdFlags::CLOEXEC)
        .map_err(|error| format!("Unable to stage the preview secret: {error}"))?;
    fchmod(&fd, Mode::from_bits_truncate(0o600))
        .map_err(|error| format!("Unable to stage the preview secret: {error}"))?;
    let mut file = std::fs::File::from(fd);
    file.write_all(secret)
        .map_err(|error| format!("Unable to stage the preview secret: {error}"))?;
    file.seek(std::io::SeekFrom::Start(0))
        .map_err(|error| format!("Unable to stage the preview secret: {error}"))?;
    Ok(file.into())
}

fn resolve_renderer_executable(
    current: &Path,
    running: &Path,
    private_output: &Path,
) -> Result<PathBuf, String> {
    if current.is_file() {
        return Ok(current.to_path_buf());
    }

    let snapshot = private_output.join("strata-preview-helper");
    fs::copy(running, &snapshot)
        .map_err(|error| format!("Unable to preserve the running Strata executable: {error}"))?;
    Ok(snapshot)
}

/// Fixed text for a missing or untrusted bubblewrap; the detail names system directories.
pub(crate) fn sandbox_unavailable(detail: &str) -> String {
    tracing::warn!(detail, "the bubblewrap sandbox is unavailable");
    crate::i18n::tr("The bubblewrap sandbox is not available")
}

pub(crate) fn sandbox_start_failed(error: &io::Error) -> String {
    tracing::warn!(%error, "unable to start the sandbox");
    rust_i18n::t!(
        "Unable to start the sandbox: %{error}",
        error = crate::services::io_error_detail(error)
    )
    .into_owned()
}

#[cfg(test)]
mod tests;
