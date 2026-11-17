// SPDX-License-Identifier: MIT

//! Preview loading, syntax highlighter wakeups, and header identity formatting.
//!
//! `LocalPreviewProvider` already dispatches `PreviewEvent` callbacks on the GLib main context,
//! so its callback queues directly into the tab's [`Later`](crate::slots::Later). C10a's
//! [`Highlighter`] runs on its own background thread and wakes the UI thread through
//! `glib::MainContext::default().invoke`, which fans out to weak `Later` handles so closed tabs
//! or windows are not retained and a covered Wayland window still runs `poll`
//! (`AppHandle::update_if`).

use std::{
    cell::{Cell, RefCell},
    path::Path,
    rc::{Rc, Weak},
    sync::Arc,
    time::Duration,
};

use rune::Tx;
use strata_components::{
    code_view::highlight::{CodeBlocks, Highlighter},
    preview_drawer::{DocumentView, HeaderInfo, HostedPreview},
    viewers::archive::PromptState,
};
use strata_core::{
    i18n::{file_size, tr},
    model::FileEntry,
    util::modified_date,
};
use strata_desktop::settings::{
    Preferences, configured_hardware_acceleration, configured_video_preview_backend,
};
use strata_fs::{
    adapters::LocalPreviewProvider,
    services::{
        DocumentLayout, INCORRECT_ARCHIVE_PASSWORD, LoadHandle, MediaPreviewSize, Preview,
        PreviewContent, PreviewEvent, PreviewProvider, PreviewRequest, PreviewRequestId,
        SecretString, normalize_preview_text,
    },
};
use strata_kit::{
    icons::IconName,
    look::Look,
    theme::syntax::{Editor, editor, syntect_theme},
};
use strata_preview::sandbox::{DocumentFormat, MediaPreviewBackend, polaris_gpu_available};

use super::{
    archive,
    document::{DocumentSpec, RenderedDocument},
    image::ImageViewer,
    pdf, sheet,
    viewer::{CodeSource, RenderedBody, Viewer, build_code_state},
};
use crate::{
    App,
    listing::known,
    shell,
    slots::{Ids, Later},
};

pub(super) const TEXT_BYTE_LIMIT: usize = 1024 * 1024;
const PREVIEW_SPINNER_DELAY: Duration = Duration::from_millis(120);
const DEFAULT_MEDIA_EDGE: i32 = 520;

thread_local! {
    static HIGHLIGHTER: RefCell<Option<Highlighter>> = const { RefCell::new(None) };
    static HIGHLIGHT_WAITERS: RefCell<Vec<Weak<Later<App>>>> = const { RefCell::new(Vec::new()) };
}

fn shared_highlighter(look: &Look, waiter: &Rc<Later<App>>) -> Highlighter {
    let weak = Rc::downgrade(waiter);
    HIGHLIGHT_WAITERS.with(|slot| {
        let mut waiters = slot.borrow_mut();
        waiters.retain(|existing| existing.strong_count() > 0);
        if !waiters.iter().any(|existing| Weak::ptr_eq(existing, &weak)) {
            waiters.push(weak);
        }
    });
    HIGHLIGHTER.with(|slot| {
        let mut slot = slot.borrow_mut();
        if let Some(existing) = slot.as_ref() {
            return existing.clone();
        }
        let highlighter = Highlighter::new(Arc::new(syntect_theme(look)));
        highlighter.set_waker(Arc::new(|| {
            glib::MainContext::default().invoke(notify_highlight_waiters);
        }));
        *slot = Some(highlighter.clone());
        highlighter
    })
}

fn notify_highlight_waiters() {
    let live: Vec<Rc<Later<App>>> = HIGHLIGHT_WAITERS.with(|slot| {
        let mut waiters = slot.borrow_mut();
        waiters.retain(|weak| weak.strong_count() > 0);
        waiters.iter().filter_map(Weak::upgrade).collect()
    });
    for later in live {
        later.then(|app: &mut App, tx: &mut Tx| {
            let blocks = app.preview.loader.poll_blocks();
            let viewer = app.preview.viewer.poll_async(tx);
            blocks | viewer
        });
    }
}

#[cfg(test)]
pub(super) fn live_highlight_waiter_count() -> usize {
    HIGHLIGHT_WAITERS.with(|slot| {
        let mut waiters = slot.borrow_mut();
        waiters.retain(|weak| weak.strong_count() > 0);
        waiters.len()
    })
}

pub(super) struct Loader {
    provider: Rc<dyn PreviewProvider>,
    media_backend: Rc<Cell<MediaPreviewBackend>>,
    pub(super) later: Later<App>,
    pub(super) ids: Ids,
    waiter: Rc<Later<App>>,
    highlighter: Option<Highlighter>,
    blocks: Option<CodeBlocks>,
    last_editor: Option<Editor>,
    next_request: u64,
    pub(super) current_request: Option<PreviewRequestId>,
    active_load: Option<LoadHandle>,
    spinner_timer: Rc<RefCell<Option<glib::SourceId>>>,
    pub(super) defer_on_resume: bool,
    pub(super) focus_archive_on_ready: bool,
    pub(super) focus_archive_request: Option<PreviewRequestId>,
}

impl Loader {
    pub(super) fn new(later: Later<App>, ids: Ids) -> Self {
        let media_backend = Rc::new(Cell::new(MediaPreviewBackend::Automatic));
        let backend_reader = media_backend.clone();
        let waiter = Rc::new(later.clone());
        Self {
            provider: Rc::new(LocalPreviewProvider::new(Rc::new(move || {
                backend_reader.get()
            }))),
            media_backend,
            later,
            ids,
            waiter,
            highlighter: None,
            blocks: None,
            last_editor: None,
            next_request: 1,
            current_request: None,
            active_load: None,
            spinner_timer: Rc::default(),
            defer_on_resume: false,
            focus_archive_on_ready: false,
            focus_archive_request: None,
        }
    }

    pub(super) fn provider(&self) -> Rc<dyn PreviewProvider> {
        self.provider.clone()
    }

    pub(super) fn set_media_backend(&self, backend: MediaPreviewBackend) {
        self.media_backend.set(backend);
    }

    #[cfg(test)]
    pub(super) fn media_backend(&self) -> MediaPreviewBackend {
        self.media_backend.get()
    }

    pub(super) fn is_active(&self) -> bool {
        self.active_load.is_some()
    }

    pub(super) fn cancel(&mut self) {
        self.current_request = None;
        self.active_load = None;
        self.defer_on_resume = false;
        self.focus_archive_request = None;
        if let Some(timer) = self.spinner_timer.borrow_mut().take() {
            timer.remove();
        }
    }

    pub(super) fn take_focus_archive(&mut self, expected: PreviewRequestId) -> bool {
        self.focus_archive_request.take() == Some(expected)
    }

    pub(super) fn sync_theme(&mut self, look: &Look) {
        let current = editor(look);
        if self.last_editor == Some(current) {
            return;
        }
        self.last_editor = Some(current);
        if let Some(highlighter) = &self.highlighter {
            highlighter.set_theme(Arc::new(syntect_theme(look)));
        }
    }

    pub(super) fn poll_blocks(&mut self) -> bool {
        self.blocks.as_mut().is_some_and(CodeBlocks::poll)
    }

    fn highlighter(&mut self, look: &Look) -> Highlighter {
        if let Some(existing) = &self.highlighter {
            return existing.clone();
        }
        let highlighter = shared_highlighter(look, &self.waiter);
        self.last_editor = Some(editor(look));
        self.highlighter = Some(highlighter.clone());
        highlighter
    }

    fn blocks(&mut self, look: &Look) -> CodeBlocks {
        if let Some(existing) = &self.blocks {
            return existing.clone();
        }
        let blocks = self.highlighter(look).blocks();
        self.blocks = Some(blocks.clone());
        blocks
    }
}

/// Effective media decoding backend (`media_preview_backend` in
/// `src/ui/preferences/preview_settings.rs`).
pub(super) fn effective_media_backend(saved: &Preferences) -> MediaPreviewBackend {
    if !configured_hardware_acceleration(saved, polaris_gpu_available()) {
        MediaPreviewBackend::Software
    } else {
        configured_video_preview_backend(saved)
    }
}

/// Formats `entry`'s header bar identity (`show_identity` in `src/ui/preview/loading.rs`).
pub(super) fn identity_for(
    look: &Look,
    entry: &FileEntry,
    content_type: Option<&str>,
) -> HeaderInfo {
    let size = known(&entry.size).map_or_else(|| "—".to_owned(), file_size);
    let modified = modified_date(entry);
    let kind = content_type
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| extension_label(entry));
    HeaderInfo::new(entry.display_name.clone())
        .icon(look.icon(icon_for(entry)))
        .details([size, modified, kind])
}

fn extension_label(entry: &FileEntry) -> String {
    entry
        .location
        .native_path()
        .and_then(Path::extension)
        .and_then(|ext| ext.to_str())
        .unwrap_or("file")
        .to_owned()
}

fn icon_for(entry: &FileEntry) -> IconName {
    if entry.is_directory() {
        return IconName::Folder;
    }
    IconName::FileText
}

pub(super) fn start(app: &mut App, entry: FileEntry) {
    let render_document = shell::preferences(app).render_documents_by_default;
    app.preview.document_view = if render_document {
        DocumentView::Rendered
    } else {
        DocumentView::Source
    };
    start_request(app, entry, render_document, None);
}

pub(super) fn start_with_password(app: &mut App, entry: FileEntry, password: SecretString) {
    let render_document = shell::preferences(app).render_documents_by_default;
    start_request(app, entry, render_document, Some(password));
}

pub(super) fn reload_rendered(app: &mut App, entry: FileEntry) {
    app.preview.document_view = DocumentView::Rendered;
    start_request(app, entry, true, None);
}

fn start_request(
    app: &mut App,
    entry: FileEntry,
    render_document: bool,
    archive_password: Option<SecretString>,
) {
    app.preview.header_source = Some((entry.clone(), None));
    app.preview.header = identity_for(&app.look, &entry, None);
    let later = app.preview.loader.later.clone();
    app.preview.raw.load(&entry, later);
    let focus_archive = std::mem::replace(&mut app.preview.loader.focus_archive_on_ready, false);
    app.preview.loader.cancel();
    if app.preview.drawer.is_suspended() {
        app.preview.loader.defer_on_resume = true;
        app.preview.loader.focus_archive_on_ready = focus_archive;
        app.preview.viewer.stop();
        app.preview.viewer = Viewer::Empty;
        return;
    }
    let request_id = PreviewRequestId(app.preview.loader.next_request);
    app.preview.loader.next_request = app.preview.loader.next_request.saturating_add(1);
    app.preview.loader.current_request = Some(request_id);
    if focus_archive {
        app.preview.loader.focus_archive_request = Some(request_id);
    }
    schedule_spinner(app, request_id);

    let later = app.preview.loader.later.clone();
    let emit = Rc::new(move |event: PreviewEvent| {
        later.then(move |app: &mut App, tx: &mut Tx| on_event(app, request_id, event, tx));
    });
    let request = PreviewRequest {
        id: request_id,
        entry,
        text_byte_limit: TEXT_BYTE_LIMIT,
        render_document,
        pdf_page: 0,
        media_size: MediaPreviewSize::for_viewport(DEFAULT_MEDIA_EDGE, DEFAULT_MEDIA_EDGE, 1),
        model_palette: Default::default(),
        model_view: Default::default(),
        archive_password,
    };
    let handle = app.preview.loader.provider.load(request, emit);
    app.preview.loader.active_load = Some(handle);
}

fn schedule_spinner(app: &mut App, expected: PreviewRequestId) {
    let later = app.preview.loader.later.clone();
    let slot = app.preview.loader.spinner_timer.clone();
    let source = glib::timeout_add_local_once(PREVIEW_SPINNER_DELAY, move || {
        slot.borrow_mut().take();
        later.then(move |app: &mut App, _: &mut Tx| {
            let waiting = app.preview.enabled
                && app.preview.loader.current_request == Some(expected)
                && app.preview.viewer.is_empty();
            if waiting {
                app.preview.viewer = Viewer::Loading;
                return true;
            }
            false
        });
    });
    app.preview.loader.spinner_timer.replace(Some(source));
}

pub(super) fn on_event(
    app: &mut App,
    expected: PreviewRequestId,
    event: PreviewEvent,
    tx: &mut Tx,
) -> bool {
    if !app.preview.enabled || app.preview.loader.current_request != Some(expected) {
        return false;
    }
    if let Some(timer) = app.preview.loader.spinner_timer.borrow_mut().take() {
        timer.remove();
    }
    match event {
        PreviewEvent::Progress { .. } => {
            if app.preview.viewer.is_empty() {
                app.preview.viewer = Viewer::Loading;
                return true;
            }
            false
        }
        PreviewEvent::Ready(loaded) => {
            app.preview.loader.active_load = None;
            app.preview.loader.current_request = None;
            render_ready(app, loaded, tx);
            true
        }
        PreviewEvent::NeedsPassword { entry, .. } => {
            app.preview.loader.active_load = None;
            app.preview.loader.current_request = None;
            render_password_prompt(app, (expected, &entry), None, tx);
            true
        }
        PreviewEvent::Failed { entry, message, .. } => {
            app.preview.loader.active_load = None;
            app.preview.loader.current_request = None;
            render_failed(app, (expected, &entry), &message, tx);
            true
        }
    }
}

fn render_password_prompt(
    app: &mut App,
    (expected, entry): (PreviewRequestId, &FileEntry),
    error: Option<String>,
    tx: &mut Tx,
) {
    let focus = app.preview.loader.take_focus_archive(expected);
    app.preview.header_source = Some((entry.clone(), None));
    app.preview.header = identity_for(&app.look, entry, None);
    let mut prompt = match std::mem::replace(&mut app.preview.viewer, Viewer::Empty) {
        Viewer::ArchivePrompt(existing) if error.is_some() => existing,
        mut other => {
            other.stop();
            Box::new(PromptState::default())
        }
    };
    prompt.set_error(error);
    if focus {
        prompt.focus();
        tx.focus_when_shown(archive::PROMPT_FIELD_KEY, true);
    }
    app.preview.viewer = Viewer::ArchivePrompt(prompt);
}

fn render_failed(
    app: &mut App,
    (expected, entry): (PreviewRequestId, &FileEntry),
    message: &str,
    tx: &mut Tx,
) {
    if message == INCORRECT_ARCHIVE_PASSWORD {
        render_password_prompt(app, (expected, entry), Some(tr(message)), tx);
        return;
    }
    let _ = app.preview.loader.take_focus_archive(expected);
    app.preview.viewer.stop();
    let offers_open = message == strata_preview::sandbox::PDF_PASSWORD_PROTECTED;
    app.preview.viewer = Viewer::Message {
        title: tr("Preview unavailable"),
        detail: Some(tr(message)),
        can_open: offers_open,
    };
}

pub(super) fn render_ready(app: &mut App, loaded: Preview, tx: &mut Tx) {
    let focus_archive = app.preview.loader.take_focus_archive(loaded.request_id);
    app.preview.viewer.stop();
    app.preview.header_source = Some((loaded.entry.clone(), Some(loaded.content_type.clone())));
    app.preview.header = identity_for(&app.look, &loaded.entry, Some(&loaded.content_type));
    app.preview.viewer = build_ready_viewer(app, loaded, focus_archive, tx);
    app.preview.viewer.reset_viewport(tx);
    reapply_active_find(app, tx);
}

fn build_ready_viewer(app: &mut App, loaded: Preview, focus_archive: bool, tx: &mut Tx) -> Viewer {
    let wrap = shell::preferences(app).preview_text_wrap;
    let highlighter = app.preview.loader.highlighter(&app.look);
    let file_name = loaded.entry.native_name.to_string_lossy().into_owned();
    match loaded.content {
        PreviewContent::Archive { tree } => build_archive_viewer(app, &tree, focus_archive, tx),
        PreviewContent::Text { content, truncated } => Viewer::code(
            CodeSource {
                content: normalize_preview_text(&content).into_owned(),
                truncated,
                wrap,
                file_name: &file_name,
                fallback_reason: None,
            },
            &highlighter,
        ),
        PreviewContent::Document {
            source,
            document,
            fallback_reason,
            warnings,
            truncated,
        } => build_document_viewer(
            app,
            DocumentLoad {
                entry: &loaded.entry,
                content_type: &loaded.content_type,
                file_name: &file_name,
                source,
                document,
                fallback_reason,
                warnings,
                truncated,
                wrap,
            },
            &highlighter,
        ),
        PreviewContent::Rendered { document, warnings } => {
            let body = build_rendered_body(
                app,
                &loaded.entry,
                &loaded.content_type,
                (document, warnings),
                false,
                wrap,
            );
            Viewer::rendered_only(body)
        }
        PreviewContent::Rasterized { png } => build_rasterized_viewer(&loaded.entry, &png),
        PreviewContent::Pdf {
            png,
            page,
            pages,
            text_layer,
        } => pdf::viewer(loaded.entry, (&png, page, pages), text_layer),
        PreviewContent::Image | PreviewContent::Media => {
            unavailable_viewer(Some("The sandboxed renderer returned no preview"))
        }
        PreviewContent::Unsupported => Viewer::Message {
            title: tr("No visual preview"),
            detail: Some(tr("Metadata is available for this file type.")),
            can_open: false,
        },
        _ => unavailable_viewer(None),
    }
}

fn build_archive_viewer(
    app: &mut App,
    tree: &strata_fs::services::ArchivePreviewTree,
    focus_archive: bool,
    tx: &mut Tx,
) -> Viewer {
    let mut state = archive::build_state(tree, &app.preview.loader.ids);
    if focus_archive {
        state.enter();
        tx.focus_when_shown(archive::ARCHIVE_KEY, true);
    }
    Viewer::Archive(Box::new(state))
}

fn reapply_active_find(app: &mut App, tx: &mut Tx) {
    if app.preview.drawer.is_finding() {
        let query = app.preview.drawer.find_query().to_owned();
        app.preview.viewer.find(&query);
        tx.wake_after(Duration::ZERO);
    }
}

fn unavailable_viewer(reason: Option<&str>) -> Viewer {
    Viewer::Message {
        title: tr("Preview unavailable"),
        detail: reason.map(tr),
        can_open: false,
    }
}

fn build_rasterized_viewer(entry: &FileEntry, png: &[u8]) -> Viewer {
    match ImageViewer::from_png(entry, png) {
        Ok(img) => Viewer::Image(Box::new(img)),
        Err(detail) => Viewer::Message {
            title: tr("Preview unavailable"),
            detail: Some(detail),
            can_open: false,
        },
    }
}

struct DocumentLoad<'a> {
    entry: &'a FileEntry,
    content_type: &'a str,
    file_name: &'a str,
    source: String,
    document: Option<DocumentLayout>,
    fallback_reason: Option<String>,
    warnings: Vec<String>,
    truncated: bool,
    wrap: bool,
}

fn build_document_viewer(
    app: &mut App,
    load: DocumentLoad<'_>,
    highlighter: &Highlighter,
) -> Viewer {
    let render_pending = load.document.is_none() && load.fallback_reason.is_none();
    let code = build_code_state(
        CodeSource {
            content: normalize_preview_text(&load.source).into_owned(),
            truncated: load.truncated,
            wrap: load.wrap,
            file_name: load.file_name,
            fallback_reason: load.fallback_reason.as_deref(),
        },
        highlighter,
    );
    let warnings = load.warnings;
    let rendered = load.document.map(|doc_layout| {
        build_rendered_body(
            app,
            load.entry,
            load.content_type,
            (doc_layout, warnings),
            true,
            load.wrap,
        )
    });
    Viewer::document(app.preview.document_view, rendered, code, render_pending)
}

fn build_rendered_body(
    app: &mut App,
    entry: &FileEntry,
    content_type: &str,
    (layout, warnings): (DocumentLayout, Vec<String>),
    source_available: bool,
    wrap: bool,
) -> RenderedBody {
    if sheet::is_table_file(content_type, &entry.native_name)
        && let Some(grid) = sheet::from_layout(&layout, &warnings, source_available)
    {
        return RenderedBody::Sheet(Box::new(grid));
    }
    let word_processor =
        !source_available && DocumentFormat::for_file(content_type, &entry.native_name).is_some();
    let blocks = app.preview.loader.blocks(&app.look);
    let later = app.preview.loader.later.clone();
    let doc = RenderedDocument::new(DocumentSpec {
        layout,
        warnings,
        wrap: (!word_processor).then_some(wrap),
        source_available,
        document_path: entry.location.native_path().map(Path::to_path_buf),
        blocks,
        later,
    });
    RenderedBody::Document(Box::new(doc))
}
