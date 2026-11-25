// SPDX-License-Identifier: MIT

use std::{cell::Cell, rc::Rc, sync::Arc};

use rune::{Cx, El, Tx};
use strata_components::pdf_view::{
    PageReply, PdfDocument, PdfIntent, PdfLabels, PdfState, PdfView, RenderCaps, RenderKind,
    RenderRequest, TextLayer, Ticket,
};
use strata_core::{i18n::tr, model::FileEntry};
use strata_desktop::keys::{Modifiers, bindings::PDF_ZOOM_RESET};
use strata_fs::services::{
    LoadHandle, MediaPreviewSize, PdfTextIndex, PdfTextLayer, Preview, PreviewContent,
    PreviewEvent, PreviewRequest, PreviewRequestId,
};

use super::{
    load::TEXT_BYTE_LIMIT,
    viewer::{self, Viewer},
};
use crate::{
    App,
    commands::{Action, Call, Keys, Outcome, Situation, Stage},
};

/// Key of the PDF viewer inside the preview drawer.
pub(crate) const PDF_KEY: &str = "preview-pdf";

const CAPS: RenderCaps = RenderCaps {
    max_width: MediaPreviewSize::MAX_EDGE as u32,
    max_height: 1_800,
    max_pixels: 2_500_000,
};

const TEXT_ONLY_SIZE: MediaPreviewSize = MediaPreviewSize {
    width: 16,
    height: 16,
};

const TEXT_ONLY_FIRST_PAGE_SIZE: MediaPreviewSize = MediaPreviewSize {
    width: 256,
    height: 256,
};

thread_local! {
    static NEXT_VIEWER: Cell<u64> = const { Cell::new(1) };
}

/// The render in flight: dropping it cancels the load.
struct Busy {
    ticket: Ticket,
    _load: Option<LoadHandle>,
}

/// Lifecycle of the single-pass document-wide PDF text extraction for find.
enum TextIndexState {
    Idle,
    Loading(#[expect(dead_code, reason = "drop cancels the sandboxed text extraction")] LoadHandle),
    Loaded,
    Failed,
    Unsupported,
}

pub(super) struct PdfViewer {
    pub(super) state: PdfState,
    id: u64,
    entry: FileEntry,
    busy: Option<Busy>,
    text_index: TextIndexState,
    next_request: u64,
}

impl PdfViewer {
    /// The viewer for `entry`, opened on `page` of `pages` from its rendered `png`.
    fn open(
        entry: FileEntry,
        (png, page, pages): (&[u8], i32, i32),
        text: Option<Arc<PdfTextLayer>>,
    ) -> Result<Self, String> {
        let reply = decode(png, text.as_deref())?;
        let PageReply::Rendered { width, height, .. } = &reply else {
            return Err(tr("Preview unavailable"));
        };
        let size = [*width as f32, *height as f32];
        let pages = usize::try_from(pages.max(1)).unwrap_or(1);
        let mut state = PdfState::new(PDF_KEY);
        state.set_caps(CAPS);
        state.set_searchable(true);
        let first = usize::try_from(page.max(0)).unwrap_or(0);
        state.open_with(PdfDocument::new(pages, size), first, reply);
        Ok(Self {
            state,
            id: NEXT_VIEWER.with(|next| next.replace(next.get().wrapping_add(1))),
            entry,
            busy: None,
            text_index: TextIndexState::Idle,
            next_request: 1,
        })
    }

    pub(super) fn id(&self) -> u64 {
        self.id
    }

    pub(super) fn stop(&mut self) {
        self.busy = None;
        self.text_index = TextIndexState::Idle;
    }

    pub(super) fn find_end(&mut self) {
        if matches!(self.text_index, TextIndexState::Loading(_)) {
            self.text_index = TextIndexState::Idle;
        }
        strata_components::preview_drawer::HostedPreview::find_end(&mut self.state);
    }
}

/// The viewer for a PDF preview's first page, or why it can't show.
pub(super) fn viewer(
    entry: FileEntry,
    first: (&[u8], i32, i32),
    text: Option<Arc<PdfTextLayer>>,
) -> Viewer {
    match PdfViewer::open(entry, first, text) {
        Ok(pdf) => Viewer::Pdf(Box::new(pdf)),
        Err(detail) => Viewer::Message {
            title: tr("Preview unavailable"),
            detail: Some(detail),
            can_open: false,
        },
    }
}

fn text_layer(text: Option<&PdfTextLayer>) -> Option<TextLayer> {
    text.map(|layer| {
        TextLayer::new(
            layer.width,
            layer.height,
            layer.text.clone(),
            layer.glyphs.clone(),
        )
    })
}

/// Decodes a page's PNG into the view's RGBA, with its text layer.
fn decode(png: &[u8], text: Option<&PdfTextLayer>) -> Result<PageReply, String> {
    let rgba = image::load_from_memory(png)
        .map_err(|err| err.to_string())?
        .to_rgba8();
    let (width, height) = rgba.dimensions();
    Ok(PageReply::Rendered {
        width,
        height,
        rgba: rgba.into_raw(),
        text: text_layer(text),
    })
}

pub(super) fn labels() -> PdfLabels {
    let mut labels = PdfLabels::default();
    labels.document = tr("PDF document");
    labels.page = rust_i18n::t!(
        "Page %{page} of %{pages}",
        page = "{page}",
        pages = "{pages}"
    )
    .into_owned();
    labels.position = rust_i18n::t!(
        "Page %{page} of %{pages}, %{zoom}%",
        page = "{page}",
        pages = "{pages}",
        zoom = "{zoom}"
    )
    .into_owned();
    labels.loading = tr("Loading page");
    labels.failed_page = tr("Unable to render this PDF page");
    labels.unavailable = tr("Preview unavailable");
    labels.locked = tr("This PDF is password protected.");
    labels.open = tr("Open");
    labels.page_text = tr("PDF page text");
    labels.page_navigation = tr("Page navigation");
    labels.page_indicator = rust_i18n::t!(
        "Page %{input} / %{pages}",
        input = "{input}",
        pages = "{pages}"
    )
    .into_owned();
    labels.go_to_page = tr("Go to page");
    labels.previous_page = tr("Previous page");
    labels.next_page = tr("Next page");
    labels
}

pub(super) fn build_view(app: &App, cx: &mut Cx<App>) -> El {
    PdfView::new(pdf_ref, pdf_mut)
        .labels(labels())
        // The key map's reset is [`RESET`]'s, at the window's stage.
        .reset_key(|_| false)
        .on_intent(on_intent)
        .build(app, &app.look, cx)
        .no_focus_ring()
}

pub(super) const RESET: Action<App> = Action::new("preview-pdf-reset", reset_zoom)
    .command(PDF_ZOOM_RESET)
    .keys(&[Keys::at(
        Stage::ZoomShortcut,
        &[PDF_ZOOM_RESET.ignoring(Modifiers::empty())],
    )
    .when(in_pages)]);

fn in_pages(app: &App, at: &Situation) -> bool {
    matches!(app.preview.viewer, Viewer::Pdf(_)) && at.within(PDF_KEY)
}

fn reset_zoom(app: &mut App, _: &Call<'_>, tx: &mut Tx) -> Outcome {
    let Viewer::Pdf(pdf) = &mut app.preview.viewer else {
        return Outcome::Declined;
    };
    pdf.state.reset_zoom(tx);
    crate::tenxer::cancel_chord(app);
    Outcome::Ran
}

fn on_intent(app: &mut App, intent: PdfIntent, tx: &mut Tx) {
    match intent {
        PdfIntent::Open => super::open_current(app),
        PdfIntent::Leave => {
            super::return_to_files(app);
            tx.wake_after(std::time::Duration::ZERO);
        }
        PdfIntent::ContextMenu(at) => {
            super::open_text_context_menu(app, at, tx);
        }
        _ => {}
    }
}

fn pdf_ref(app: &App) -> &PdfState {
    match &app.preview.viewer {
        Viewer::Pdf(pdf) => &pdf.state,
        _ => viewer::spare("PDF", &app.preview.detached.pdf),
    }
}

fn pdf_mut(app: &mut App) -> &mut PdfState {
    let preview = &mut app.preview;
    match &mut preview.viewer {
        Viewer::Pdf(pdf) => &mut pdf.state,
        _ => viewer::spare("PDF", &mut preview.detached.pdf),
    }
}

fn viewer_mut(app: &mut App, id: u64) -> Option<&mut PdfViewer> {
    match &mut app.preview.viewer {
        Viewer::Pdf(pdf) if pdf.id == id => Some(pdf),
        _ => None,
    }
}

pub(super) fn poll(app: &mut App, clock: f64, tx: &mut Tx) -> bool {
    let hold = crate::menus::field_menu_open(app);
    let Viewer::Pdf(pdf) = &mut app.preview.viewer else {
        return false;
    };
    pdf.state.hold_page_input(hold);
    let changed = pdf.state.tick(clock) | pdf.state.flush(tx);
    changed | pump(app)
}

/// Starts a single-pass document-wide PDF text extraction when find is active on a multi-page PDF.
fn maybe_start_text_index(app: &mut App) -> bool {
    let (id, entry) = match &app.preview.viewer {
        Viewer::Pdf(pdf)
            if matches!(pdf.text_index, TextIndexState::Idle) && pdf.state.needs_text_index() =>
        {
            (pdf.id, pdf.entry.clone())
        }
        _ => return false,
    };
    let provider = app.preview.loader.provider();
    let later = app.preview.loader.later.clone();
    let emit = Rc::new(move |result: Result<PdfTextIndex, String>| {
        later.then(move |app: &mut App, tx: &mut Tx| on_text_index(app, id, result, tx));
    });
    match provider.extract_pdf_text(&entry, emit) {
        Some(handle) => {
            if let Some(pdf) = viewer_mut(app, id) {
                pdf.text_index = TextIndexState::Loading(handle);
                pdf.state.set_text_index_loading(Some(tr("Loading…")));
                return true;
            }
            false
        }
        None => {
            if let Some(pdf) = viewer_mut(app, id) {
                pdf.text_index = TextIndexState::Unsupported;
            }
            false
        }
    }
}

fn on_text_index(
    app: &mut App,
    id: u64,
    result: Result<PdfTextIndex, String>,
    tx: &mut Tx,
) -> bool {
    let Some(pdf) = viewer_mut(app, id) else {
        return false;
    };
    if !matches!(pdf.text_index, TextIndexState::Loading(_)) {
        return false;
    }
    match result {
        Ok(index) => {
            pdf.text_index = TextIndexState::Loaded;
            pdf.state.set_page_texts(index.pages);
            let _ = pdf.state.flush(tx);
        }
        Err(error) => {
            tracing::debug!(
                %error,
                "single-pass PDF text extraction failed; falling back to per-page text"
            );
            pdf.text_index = TextIndexState::Failed;
            pdf.state.set_text_index_loading(None);
        }
    }
    pump(app);
    true
}

fn pump(app: &mut App) -> bool {
    let started_index = maybe_start_text_index(app);
    let provider = app.preview.loader.provider();
    let later = app.preview.loader.later.clone();
    let Viewer::Pdf(pdf) = &mut app.preview.viewer else {
        return started_index;
    };
    let cancelled = pdf.state.take_cancelled();
    if pdf
        .busy
        .as_ref()
        .is_some_and(|busy| cancelled.contains(&busy.ticket))
    {
        pdf.busy = None;
    }
    if pdf.busy.is_some() {
        return started_index;
    }
    let Some(asked) = pdf.state.pop() else {
        return started_index;
    };
    let (id, ticket) = (pdf.id, asked.ticket);
    pdf.busy = Some(Busy {
        ticket,
        _load: None,
    });
    let request = page_request(pdf, &asked);
    let emit = Rc::new(move |event: PreviewEvent| {
        later.then(move |app: &mut App, tx: &mut Tx| on_page(app, (id, ticket), event, tx));
    });
    let load = provider.load(request, emit);
    if let Some(busy) = viewer_mut(app, id)
        .and_then(|pdf| pdf.busy.as_mut())
        .filter(|busy| busy.ticket == ticket)
    {
        busy._load = Some(load);
    }
    started_index
}

fn page_request(pdf: &mut PdfViewer, asked: &RenderRequest) -> PreviewRequest {
    let serial = pdf.next_request;
    pdf.next_request = serial.wrapping_add(1);
    let media_size = match asked.kind {
        RenderKind::TextOnly if asked.page == 0 => TEXT_ONLY_FIRST_PAGE_SIZE,
        RenderKind::TextOnly => TEXT_ONLY_SIZE,
        RenderKind::Page => {
            let [width, height] = asked
                .box_px
                .map(|edge| i32::try_from(edge).unwrap_or(i32::MAX));
            MediaPreviewSize::new(width, height)
        }
    };
    PreviewRequest {
        // Apart from the preview's own requests: each reply is routed by its closure.
        id: PreviewRequestId(u64::MAX / 2 + serial),
        entry: pdf.entry.clone(),
        text_byte_limit: TEXT_BYTE_LIMIT,
        render_document: false,
        pdf_page: i32::try_from(asked.page).unwrap_or(i32::MAX),
        media_size,
        model_palette: Default::default(),
        model_view: Default::default(),
        archive_password: None,
    }
}

fn on_page(app: &mut App, (id, ticket): (u64, Ticket), event: PreviewEvent, tx: &mut Tx) -> bool {
    if viewer_mut(app, id).is_none_or(|pdf| !is_busy_with(pdf, ticket)) {
        return false;
    }
    let (png, text) = match event {
        PreviewEvent::Ready(Preview {
            content: PreviewContent::Pdf {
                png, text_layer, ..
            },
            ..
        }) => (png, text_layer),
        PreviewEvent::Progress { .. } => return false,
        _ => return land(app, (id, ticket), PageReply::Failed, tx),
    };
    if ticket.kind() == RenderKind::TextOnly {
        return land(
            app,
            (id, ticket),
            PageReply::Text(text_layer(text.as_deref())),
            tx,
        );
    }
    let later = app.preview.loader.later.clone();
    glib::MainContext::default().spawn_local(async move {
        let decoded = gio::spawn_blocking(move || decode(&png, text.as_deref())).await;
        let reply = match decoded {
            Ok(Ok(reply)) => reply,
            _ => PageReply::Failed,
        };
        later.then(move |app: &mut App, tx: &mut Tx| land(app, (id, ticket), reply, tx));
    });
    false
}

fn is_busy_with(pdf: &PdfViewer, ticket: Ticket) -> bool {
    pdf.busy.as_ref().is_some_and(|busy| busy.ticket == ticket)
}

fn land(app: &mut App, (id, ticket): (u64, Ticket), reply: PageReply, tx: &mut Tx) -> bool {
    let Some(pdf) = viewer_mut(app, id) else {
        return false;
    };
    if is_busy_with(pdf, ticket) {
        pdf.busy = None;
    }
    let commits = pdf.state.complete(ticket, reply).commits();
    let flushed = pdf.state.flush(tx);
    let started_index = pump(app);
    commits | flushed | started_index
}
