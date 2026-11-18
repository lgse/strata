// SPDX-License-Identifier: MIT

//! C12 [`PdfView`] adapter for PDF previews (`PreviewContent::Pdf`), as GTK's
//! `src/ui/preview/pdf.rs` hosts its pages.
//!
//! - **The first page** arrives with the preview, already rendered (`render_ready`), and opens
//!   the document at that page's shape: GTK's renderer says a page's size only when it renders
//!   it.
//! - **Every other page** is asked of the same [`PreviewProvider`] with its page number and the
//!   width the view wants, one render at a time across the preview, as GTK's page loader asks on
//!   bind. The PNG is decoded off the UI thread (`gio::spawn_blocking`) before the view takes it.
//!   A reply for a viewer that has gone, or a request the view cancelled, is dropped.
//! - **Zoom:** Ctrl+wheel over the pages is the view's own, and the window's zoom wheel leaves
//!   it to the view (`appearance::wheel`), as GTK's `claims_zoom` leaves the PDF's scroll.
//!   The reset is the key map's `pdf_zoom_reset` ([`RESET`]), read at the dispatcher's
//!   `zoom_shortcut` stage ahead of the window's `text-reset` while the keyboard is in the
//!   pages, as GTK's `zoom_shortcut` asks `reset_pdf_zoom` first. The view's own Ctrl+0 is off,
//!   so a rebound reset follows the key map and an unbound Ctrl+0 reaches the text size.
//! - **A password-protected PDF** keeps the preview's message card with its Open button
//!   (`load::render_failed`), as GTK's does.
//!
//! Every element is keyed under [`PDF_KEY`], so the preview's guards and focus memory cover it.

use std::{cell::Cell, rc::Rc, sync::Arc};

use rune::{Cx, El, Tx};
use strata_components::pdf_view::{
    PageReply, PdfDocument, PdfIntent, PdfLabels, PdfState, PdfView, RenderCaps, RenderRequest,
    TextLayer, Ticket,
};
use strata_core::{i18n::tr, model::FileEntry};
use strata_desktop::keys::{Modifiers, bindings::PDF_ZOOM_RESET};
use strata_fs::services::{
    LoadHandle, MediaPreviewSize, PdfTextLayer, Preview, PreviewContent, PreviewEvent,
    PreviewRequest, PreviewRequestId,
};

use super::{load::TEXT_BYTE_LIMIT, viewer::Viewer};
use crate::{
    App,
    commands::{Action, Call, Keys, Outcome, Situation, Stage},
};

/// Key of the PDF viewer inside the preview drawer.
pub(crate) const PDF_KEY: &str = "preview-pdf";
/// The key of [`pdf_mut`]'s stand-in, which no element is built from.
pub(super) const GONE_KEY: &str = "preview-pdf-gone";

/// The renderer's limits: GTK's `PdfRenderSize`, whose width comes through the request's
/// `MediaPreviewSize`, which stops at 1280 pixels.
const CAPS: RenderCaps = RenderCaps {
    max_width: MediaPreviewSize::MAX_EDGE as u32,
    max_height: 1_800,
    max_pixels: 2_500_000,
};

thread_local! {
    static NEXT_VIEWER: Cell<u64> = const { Cell::new(1) };
}

/// The render in flight: dropping it cancels the load.
struct Busy {
    ticket: Ticket,
    _load: Option<LoadHandle>,
}

pub(super) struct PdfViewer {
    pub(super) state: PdfState,
    id: u64,
    entry: FileEntry,
    busy: Option<Busy>,
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
        let first = usize::try_from(page.max(0)).unwrap_or(0);
        state.open_with(PdfDocument::new(pages, size), first, reply);
        Ok(Self {
            state,
            id: NEXT_VIEWER.with(|next| next.replace(next.get().wrapping_add(1))),
            entry,
            busy: None,
            next_request: 1,
        })
    }

    pub(super) fn stop(&mut self) {
        self.busy = None;
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
        text: text.map(|layer| {
            TextLayer::new(
                layer.width,
                layer.height,
                layer.text.clone(),
                layer.glyphs.clone(),
            )
        }),
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
    labels
}

pub(super) fn build_view(app: &App, cx: &mut Cx<App>) -> El {
    PdfView::new(pdf_ref, pdf_mut)
        .labels(labels())
        // The key map's reset is [`RESET`]'s, at the window's stage.
        .reset_key(|_| false)
        .on_intent(on_intent)
        .build(app, &app.look, cx)
}

/// `pdf_zoom_reset` while the keyboard is in the pages: the PDF back to its fitted width, ahead
/// of `text-reset` at the same stage (`reset_pdf_zoom` in `src/ui/preview/keyboard.rs`).
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

fn on_intent(app: &mut App, intent: PdfIntent, _: &mut Tx) {
    match intent {
        PdfIntent::Open => super::open_current(app),
        PdfIntent::Leave => app.preview.drawer.focus_files(),
        _ => {}
    }
}

fn pdf_ref(app: &App) -> &PdfState {
    match &app.preview.viewer {
        Viewer::Pdf(pdf) => &pdf.state,
        _ => unreachable!("PdfView built only while Viewer::Pdf is active"),
    }
}

/// The pages' hover ends as they unmount, after the viewer they showed has gone: what that
/// handler changes then is a stand-in's, which nothing reads.
fn pdf_mut(app: &mut App) -> &mut PdfState {
    match &mut app.preview.viewer {
        Viewer::Pdf(pdf) => &mut pdf.state,
        _ => &mut app.preview.pdf_gone,
    }
}

fn viewer_mut(app: &mut App, id: u64) -> Option<&mut PdfViewer> {
    match &mut app.preview.viewer {
        Viewer::Pdf(pdf) if pdf.id == id => Some(pdf),
        _ => None,
    }
}

/// The view's frame work, from the preview's poll: its zoom's rest, a reveal, and the next
/// render. True when the view changed.
pub(super) fn poll(app: &mut App, clock: f64, tx: &mut Tx) -> bool {
    let Viewer::Pdf(pdf) = &mut app.preview.viewer else {
        return false;
    };
    let changed = pdf.state.tick(clock) | pdf.state.flush(tx);
    pump(app);
    changed
}

/// Drops a render the view no longer wants, and starts the next one when none runs.
fn pump(app: &mut App) {
    let provider = app.preview.loader.provider();
    let later = app.preview.loader.later.clone();
    let Viewer::Pdf(pdf) = &mut app.preview.viewer else {
        return;
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
        return;
    }
    let Some(asked) = pdf.state.pop() else {
        return;
    };
    let (id, ticket) = (pdf.id, asked.ticket);
    pdf.busy = Some(Busy {
        ticket,
        _load: None,
    });
    let request = page_request(pdf, &asked);
    let emit = Rc::new(move |event: PreviewEvent| {
        later.then(move |app: &mut App, _: &mut Tx| on_page(app, (id, ticket), event));
    });
    let load = provider.load(request, emit);
    if let Some(busy) = viewer_mut(app, id)
        .and_then(|pdf| pdf.busy.as_mut())
        .filter(|busy| busy.ticket == ticket)
    {
        busy._load = Some(load);
    }
}

fn page_request(pdf: &mut PdfViewer, asked: &RenderRequest) -> PreviewRequest {
    let serial = pdf.next_request;
    pdf.next_request = serial.wrapping_add(1);
    let [width, height] = asked
        .box_px
        .map(|edge| i32::try_from(edge).unwrap_or(i32::MAX));
    PreviewRequest {
        // Apart from the preview's own requests: each reply is routed by its closure.
        id: PreviewRequestId(u64::MAX / 2 + serial),
        entry: pdf.entry.clone(),
        text_byte_limit: TEXT_BYTE_LIMIT,
        render_document: false,
        pdf_page: i32::try_from(asked.page).unwrap_or(i32::MAX),
        media_size: MediaPreviewSize::new(width, height),
        model_palette: Default::default(),
        model_view: Default::default(),
        archive_password: None,
    }
}

/// A page render's answer: decoded off the UI thread, then handed to the view.
fn on_page(app: &mut App, (id, ticket): (u64, Ticket), event: PreviewEvent) -> bool {
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
        _ => return land(app, (id, ticket), PageReply::Failed),
    };
    let later = app.preview.loader.later.clone();
    glib::MainContext::default().spawn_local(async move {
        let decoded = gio::spawn_blocking(move || decode(&png, text.as_deref())).await;
        let reply = match decoded {
            Ok(Ok(reply)) => reply,
            _ => PageReply::Failed,
        };
        later.then(move |app: &mut App, _: &mut Tx| land(app, (id, ticket), reply));
    });
    false
}

fn is_busy_with(pdf: &PdfViewer, ticket: Ticket) -> bool {
    pdf.busy.as_ref().is_some_and(|busy| busy.ticket == ticket)
}

fn land(app: &mut App, (id, ticket): (u64, Ticket), reply: PageReply) -> bool {
    let Some(pdf) = viewer_mut(app, id) else {
        return false;
    };
    if is_busy_with(pdf, ticket) {
        pdf.busy = None;
    }
    let commits = pdf.state.complete(ticket, reply).commits();
    pump(app);
    commits
}
