// SPDX-License-Identifier: MIT

//! Preview loading, syntax highlighter wakeups, and header identity formatting.
//!
//! `LocalPreviewProvider` already dispatches `PreviewEvent` callbacks on the GLib main context,
//! so its callback queues directly into the tab's [`Later`](crate::slots::Later). C10a's
//! [`Highlighter`] runs on its own background thread and wakes the UI thread through
//! `glib::MainContext::default().invoke`, which fans out to registered `Later` handles so a
//! covered Wayland window still runs `poll` (`AppHandle::update_if`).

use std::{
    cell::{Cell, RefCell},
    path::Path,
    rc::Rc,
    sync::Arc,
    time::Duration,
};

use rune::Tx;
use strata_components::{
    code_view::highlight::Highlighter,
    preview_drawer::{HeaderInfo, HostedPreview},
};
use strata_core::{
    i18n::{file_size, tr},
    model::FileEntry,
    util::modified_date,
};
use strata_fs::{
    adapters::LocalPreviewProvider,
    services::{
        LoadHandle, MediaPreviewSize, Preview, PreviewContent, PreviewEvent, PreviewProvider,
        PreviewRequest, PreviewRequestId, normalize_preview_text,
    },
};
use strata_kit::{
    icons::IconName,
    look::Look,
    theme::syntax::{Editor, editor, syntect_theme},
};
use strata_preview::sandbox::MediaPreviewBackend;

use super::viewer::{CodeSource, Viewer};
use crate::{App, listing::known, shell, slots::Later};

const TEXT_BYTE_LIMIT: usize = 1024 * 1024;
const PREVIEW_SPINNER_DELAY: Duration = Duration::from_millis(120);
const DEFAULT_MEDIA_EDGE: i32 = 520;

thread_local! {
    static HIGHLIGHTER: RefCell<Option<Highlighter>> = const { RefCell::new(None) };
    static HIGHLIGHT_WAITERS: RefCell<Vec<Later<App>>> = const { RefCell::new(Vec::new()) };
}

fn shared_highlighter(look: &Look, later: &Later<App>) -> Highlighter {
    HIGHLIGHT_WAITERS.with(|waiters| {
        waiters.borrow_mut().push(later.clone());
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
    let waiters = HIGHLIGHT_WAITERS.with(|slot| slot.borrow().clone());
    for later in waiters {
        later.then(|app: &mut App, tx: &mut Tx| app.preview.viewer.poll(tx));
    }
}

pub(super) struct Loader {
    provider: Rc<dyn PreviewProvider>,
    media_backend: Rc<Cell<MediaPreviewBackend>>,
    pub(super) later: Later<App>,
    highlighter: Option<Highlighter>,
    last_editor: Option<Editor>,
    next_request: u64,
    current_request: Option<PreviewRequestId>,
    active_load: Option<LoadHandle>,
    spinner_timer: Rc<RefCell<Option<glib::SourceId>>>,
    pub(super) defer_on_resume: bool,
}

impl Loader {
    pub(super) fn new(later: Later<App>) -> Self {
        let media_backend = Rc::new(Cell::new(MediaPreviewBackend::Automatic));
        let backend_reader = media_backend.clone();
        Self {
            provider: Rc::new(LocalPreviewProvider::new(Rc::new(move || {
                backend_reader.get()
            }))),
            media_backend,
            later,
            highlighter: None,
            last_editor: None,
            next_request: 1,
            current_request: None,
            active_load: None,
            spinner_timer: Rc::default(),
            defer_on_resume: false,
        }
    }

    pub(super) fn set_media_backend(&self, backend: &str) {
        let parsed =
            MediaPreviewBackend::from_argument(backend).unwrap_or(MediaPreviewBackend::Automatic);
        self.media_backend.set(parsed);
    }

    pub(super) fn is_active(&self) -> bool {
        self.current_request.is_some()
    }

    pub(super) fn cancel(&mut self) {
        self.current_request = None;
        self.active_load = None;
        self.defer_on_resume = false;
        if let Some(timer) = self.spinner_timer.borrow_mut().take() {
            timer.remove();
        }
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

    fn highlighter(&mut self, look: &Look) -> Highlighter {
        if let Some(existing) = &self.highlighter {
            return existing.clone();
        }
        let highlighter = shared_highlighter(look, &self.later);
        self.last_editor = Some(editor(look));
        self.highlighter = Some(highlighter.clone());
        highlighter
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
    Path::new(&entry.native_name)
        .extension()
        .and_then(|ext| ext.to_str())
        .filter(|ext| !ext.is_empty())
        .map_or_else(|| tr("File"), |ext| ext.to_ascii_uppercase())
}

fn icon_for(entry: &FileEntry) -> IconName {
    if entry.is_directory() {
        return IconName::Folder;
    }
    IconName::FileText
}

pub(super) fn start(app: &mut App, entry: FileEntry) {
    app.preview.header = identity_for(&app.look, &entry, None);
    app.preview.loader.cancel();
    if app.preview.drawer.is_suspended() {
        app.preview.loader.defer_on_resume = true;
        app.preview.viewer = Viewer::Empty;
        return;
    }
    let request_id = PreviewRequestId(app.preview.loader.next_request);
    app.preview.loader.next_request = app.preview.loader.next_request.saturating_add(1);
    app.preview.loader.current_request = Some(request_id);
    schedule_spinner(app, request_id);

    let render_document = shell::preferences(app).render_documents_by_default;
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
        archive_password: None,
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

fn on_event(app: &mut App, expected: PreviewRequestId, event: PreviewEvent, tx: &mut Tx) -> bool {
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
            render_ready(app, loaded, tx);
            true
        }
        PreviewEvent::NeedsPassword { .. } => {
            app.preview.viewer = Viewer::Message {
                title: tr("Password required"),
                detail: None,
                can_open: true,
            };
            true
        }
        PreviewEvent::Failed { message, .. } => {
            let offers_open = message == strata_preview::sandbox::PDF_PASSWORD_PROTECTED;
            app.preview.viewer = Viewer::Message {
                title: tr("Preview unavailable"),
                detail: Some(tr(&message)),
                can_open: offers_open,
            };
            true
        }
    }
}

fn render_ready(app: &mut App, loaded: Preview, tx: &mut Tx) {
    app.preview.header = identity_for(&app.look, &loaded.entry, Some(&loaded.content_type));
    let wrap = shell::preferences(app).preview_text_wrap;
    let highlighter = app.preview.loader.highlighter(&app.look);
    let file_name = loaded.entry.native_name.to_string_lossy();
    app.preview.viewer = match loaded.content {
        PreviewContent::Text { content, truncated } => Viewer::code(
            CodeSource {
                content: normalize_preview_text(&content).into_owned(),
                truncated,
                wrap,
                file_name: &file_name,
            },
            &highlighter,
        ),
        PreviewContent::Document {
            source, truncated, ..
        } => Viewer::code(
            CodeSource {
                content: normalize_preview_text(&source).into_owned(),
                truncated,
                wrap,
                file_name: &file_name,
            },
            &highlighter,
        ),
        PreviewContent::Unsupported => Viewer::Message {
            title: tr("No visual preview"),
            detail: Some(tr("Metadata is available for this file type.")),
            can_open: false,
        },
        _ => Viewer::Message {
            title: tr("Preview unavailable"),
            detail: None,
            can_open: false,
        },
    };
    if app.preview.drawer.is_finding() {
        let query = app.preview.drawer.find_query().to_owned();
        app.preview.viewer.find(&query);
        app.preview.viewer.poll(tx);
    }
}
