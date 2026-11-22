// SPDX-License-Identifier: MIT

//! C11 [`DocumentView`] adapter and asynchronous sandboxed media renderer (`MediaQueue`) for
//! Markdown, HTML, RTF, DOCX, and ODT previews.

use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use rune::{Cx, El, Pixels, Tx};
use strata_components::{
    code_view::highlight::CodeBlocks,
    document_view::{
        Cell, Document, DocumentIntent, DocumentState, DocumentView, ListChildKind, Media,
        MediaState, SpanStyle, Unit, UnitKind, char_spans,
    },
};
use strata_core::i18n::tr;
use strata_fs::services::{
    DocumentLayout, DocumentListChildKind, DocumentMedia, DocumentSpan, DocumentSpanStyle,
    DocumentTableCellLayout, DocumentUnitKind, has_web_scheme,
};
use strata_kit::{look::Look, theme::Rgba8};
use strata_preview::{
    sandbox::Cancellation,
    services::document_media::{self, DOCUMENT_MEDIA_LIMIT},
};

use super::labels;
use crate::{App, error_dialog, slots::Later};

/// Key of the rendered document viewer inside the preview drawer.
pub(crate) const DOCUMENT_KEY: &str = "preview-document";

const MEDIA_RENDER_SLOTS: usize = 4;

static NEXT_DOC_ID: AtomicU64 = AtomicU64::new(1);

/// Semantic sRGB endpoints GTK's `MediaPicture::snapshot` maps diagram and equation luminance onto:
/// black ink (`luma = 0`) becomes `text`, white background/box fills (`luma = 1`) become `surface`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct DiagramPalette {
    pub(super) text: Rgba8,
    pub(super) surface: Rgba8,
}

pub(super) fn diagram_palette(look: &Look) -> DiagramPalette {
    DiagramPalette {
        text: Rgba8::from_color(look.kit.c.text_base),
        surface: Rgba8::from_color(look.kit.c.surface_raised),
    }
}

/// Maps a straight-alpha sRGB RGBA8 equation or Mermaid raster's luminance onto `palette`
/// (`src/ui/document_media.rs` `MediaPicture::snapshot`), preserving alpha unchanged.
pub(super) fn tint_diagram_rgba(raw: &[u8], palette: DiagramPalette) -> Vec<u8> {
    let text = [
        f32::from(palette.text.r),
        f32::from(palette.text.g),
        f32::from(palette.text.b),
    ];
    let delta = [
        f32::from(palette.surface.r) - text[0],
        f32::from(palette.surface.g) - text[1],
        f32::from(palette.surface.b) - text[2],
    ];
    let mut out = Vec::with_capacity(raw.len());
    for &[r, g, b, a] in raw.as_chunks::<4>().0 {
        let luma = (0.2126 * f32::from(r) + 0.7152 * f32::from(g) + 0.0722 * f32::from(b)) / 255.0;
        out.push(remap_channel(text[0], delta[0], luma));
        out.push(remap_channel(text[1], delta[1], luma));
        out.push(remap_channel(text[2], delta[2], luma));
        out.push(a);
    }
    out
}

fn remap_channel(base: f32, delta: f32, luma: f32) -> u8 {
    (base + luma * delta).clamp(0.0, 255.0).round() as u8
}

fn is_tinted_media(source: &DocumentMedia) -> bool {
    match source {
        DocumentMedia::Math { .. } | DocumentMedia::Mermaid(_) => true,
        DocumentMedia::Image(_) => false,
    }
}

struct TintedMedia {
    width: u32,
    height: u32,
    raw: Vec<u8>,
    pixels: Pixels,
}

impl TintedMedia {
    fn apply(&self, palette: DiagramPalette) {
        self.pixels.set(
            self.width,
            self.height,
            tint_diagram_rgba(&self.raw, palette),
        );
    }
}

pub(super) struct MediaTask {
    pub(super) unit: usize,
    pub(super) source: DocumentMedia,
    pub(super) alt: String,
}

pub(super) type DecodedMedia = Result<(u32, u32, Vec<u8>), String>;

pub(super) struct RenderedDocument {
    id: u64,
    pub(super) state: DocumentState,
    blocks: CodeBlocks,
    document_path: Option<PathBuf>,
    cancel: Cancellation,
    pending: VecDeque<MediaTask>,
    running: usize,
    later: Later<App>,
    palette: DiagramPalette,
    tinted: Vec<TintedMedia>,
}

impl Drop for RenderedDocument {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

pub(super) struct DocumentSpec {
    pub(super) layout: DocumentLayout,
    pub(super) warnings: Vec<String>,
    pub(super) wrap: Option<bool>,
    pub(super) source_available: bool,
    pub(super) document_path: Option<PathBuf>,
    pub(super) blocks: CodeBlocks,
    pub(super) later: Later<App>,
    pub(super) palette: DiagramPalette,
}

impl RenderedDocument {
    pub(super) fn new(spec: DocumentSpec) -> Self {
        let mut rendered = Self::from_spec(spec);
        rendered.pump_media();
        rendered
    }

    #[cfg(test)]
    pub(super) fn new_without_pump(spec: DocumentSpec) -> (Self, Vec<MediaTask>) {
        let mut rendered = Self::from_spec(spec);
        let tasks = rendered.pending.drain(..).collect();
        (rendered, tasks)
    }

    #[cfg(test)]
    pub(super) fn id(&self) -> u64 {
        self.id
    }

    #[cfg(test)]
    pub(super) fn has_active_media_tasks(&self) -> bool {
        self.running > 0 || !self.pending.is_empty()
    }

    fn from_spec(spec: DocumentSpec) -> Self {
        let id = NEXT_DOC_ID.fetch_add(1, Ordering::Relaxed);
        let (doc, media) = convert_layout(spec.layout, spec.warnings);
        let mut state = DocumentState::new(doc).key(DOCUMENT_KEY);
        state.set_wrap(spec.wrap);
        state.set_source_available(spec.source_available);
        let mut pending = VecDeque::new();
        for (idx, task) in media.into_iter().enumerate() {
            if idx >= DOCUMENT_MEDIA_LIMIT {
                let msg = format_media_error(
                    &task.source,
                    &task.alt,
                    "Preview limited to 16 images, diagrams, and equations",
                );
                state.set_media_failed(task.unit, msg);
            } else {
                pending.push_back(task);
            }
        }
        Self {
            id,
            state,
            blocks: spec.blocks,
            document_path: spec.document_path,
            cancel: Cancellation::default(),
            pending,
            running: 0,
            later: spec.later,
            palette: spec.palette,
            tinted: Vec::new(),
        }
    }

    pub(super) fn stop(&mut self) {
        self.cancel.cancel();
        self.pending.clear();
    }

    pub(super) fn sync_theme(&mut self, look: &Look) -> bool {
        let next = diagram_palette(look);
        if self.palette == next {
            return false;
        }
        self.palette = next;
        for media in &self.tinted {
            media.apply(next);
        }
        !self.tinted.is_empty()
    }

    pub(super) fn on_media_ready(
        &mut self,
        doc_id: u64,
        task: &MediaTask,
        outcome: DecodedMedia,
        look: &Look,
    ) -> bool {
        if self.id != doc_id {
            return false;
        }
        self.running = self.running.saturating_sub(1);
        self.sync_theme(look);
        match outcome {
            Ok((w, h, raw)) => self.install_media(task, w, h, raw),
            Err(reason) => {
                let formatted = format_media_error(&task.source, &task.alt, &reason);
                self.state.set_media_failed(task.unit, formatted);
            }
        }
        self.pump_media();
        true
    }

    fn install_media(&mut self, task: &MediaTask, width: u32, height: u32, raw: Vec<u8>) {
        let pixels = Pixels::new();
        if is_tinted_media(&task.source) {
            let entry = TintedMedia {
                width,
                height,
                raw,
                pixels: pixels.clone(),
            };
            entry.apply(self.palette);
            self.tinted.push(entry);
        } else {
            pixels.set(width, height, raw);
        }
        self.state.set_media(task.unit, MediaState::Ready(pixels));
    }

    fn pump_media(&mut self) {
        while self.running < MEDIA_RENDER_SLOTS {
            let Some(task) = self.pending.pop_front() else {
                break;
            };
            self.running += 1;
            let doc_id = self.id;
            let path = self.document_path.clone();
            let cancel = self.cancel.clone();
            let later = self.later.clone();
            glib::MainContext::default().spawn_local(async move {
                let source = task.source.clone();
                let joined =
                    gio::spawn_blocking(move || render_and_decode(&source, path, &cancel)).await;
                if cancel_done(&joined) {
                    return;
                }
                let outcome = joined
                    .unwrap_or_else(|_| Err("Image renderer stopped unexpectedly".to_owned()));
                later.then(move |app: &mut App, _: &mut Tx| {
                    app.preview
                        .viewer
                        .on_media_ready(doc_id, &task, outcome, &app.look)
                });
            });
        }
    }

    pub(super) fn blocks(&self) -> &CodeBlocks {
        &self.blocks
    }
}

fn cancel_done<E>(joined: &Result<DecodedMedia, E>) -> bool {
    matches!(joined, Ok(Err(msg)) if msg == "Preview cancelled")
}

fn render_and_decode(
    source: &DocumentMedia,
    path: Option<PathBuf>,
    cancel: &Cancellation,
) -> DecodedMedia {
    let bytes = document_media::render(source, path.as_deref(), cancel)?;
    let decoded = image::load_from_memory(&bytes)
        .map_err(|_| "Cannot display decoded image".to_owned())?
        .into_rgba8();
    let (w, h) = decoded.dimensions();
    if w == 0 || h == 0 {
        return Err("Cannot display decoded image".to_owned());
    }
    Ok((w, h, decoded.into_raw()))
}

fn format_media_error(source: &DocumentMedia, alt: &str, reason: &str) -> String {
    let alt = cap_alt(alt);
    let message = tr(reason);
    match source {
        DocumentMedia::Math { source: tex, .. } => {
            let head = rust_i18n::t!("Equation unavailable: %{message}", message = message);
            format!("{head}\n{tex}")
        }
        DocumentMedia::Image(_) => {
            rust_i18n::t!("Image: %{alt}\n%{message}", alt = alt, message = message).into_owned()
        }
        DocumentMedia::Mermaid(diagram) => {
            let head = rust_i18n::t!("Mermaid diagram unavailable: %{message}", message = message);
            format!("{head}\n{diagram}")
        }
    }
}

fn cap_alt(alt: &str) -> String {
    let mut chars = alt.chars();
    let mut out: String = chars.by_ref().take(512).collect();
    if chars.next().is_some() {
        out.push('…');
    }
    out
}

pub(super) type DocumentAccess = (
    fn(&App) -> &DocumentState,
    fn(&mut App) -> &mut DocumentState,
);

pub(super) fn build_view(
    app: &App,
    cx: &mut Cx<App>,
    blocks: &CodeBlocks,
    (get, get_mut): DocumentAccess,
) -> El {
    DocumentView::new(get, get_mut)
        .blocks(blocks)
        .labels(labels::document())
        .on_intent(on_document_intent)
        .build(app, &app.look, cx)
}

fn on_document_intent(app: &mut App, intent: DocumentIntent, tx: &mut Tx) {
    match intent {
        DocumentIntent::Leave => {
            super::return_to_files(app);
            tx.wake_after(std::time::Duration::ZERO);
        }
        DocumentIntent::ContextMenu(at) => {
            super::open_text_context_menu(app, at, tx);
        }
        DocumentIntent::OpenLink(uri) => {
            if has_web_scheme(&uri)
                && let Err(error) =
                    gio::AppInfo::launch_default_for_uri(&uri, gio::AppLaunchContext::NONE)
            {
                error_dialog::show(
                    app,
                    error_dialog::Error::new(tr("Unable to open link"), error.to_string()),
                );
            }
        }
        _ => {}
    }
}

fn convert_layout(
    layout: DocumentLayout,
    warnings: Vec<String>,
) -> (Arc<Document>, Vec<MediaTask>) {
    let mut units = Vec::with_capacity(layout.units.len());
    let mut media = Vec::new();
    for (idx, raw) in layout.units.into_iter().enumerate() {
        let (kind, task) = convert_kind(idx, raw.kind, &raw.text);
        if let Some(task) = task {
            media.push(task);
        }
        let unit = Unit::new(kind, raw.text)
            .copy_text(raw.copy_text)
            .part(raw.first, raw.last, raw.wrap)
            .from_char_spans(convert_spans(raw.spans));
        units.push(unit);
    }
    let notices = warnings.into_iter().map(|w| tr(&w));
    (Arc::new(Document::new(units).notices(notices)), media)
}

fn convert_kind(index: usize, kind: DocumentUnitKind, text: &str) -> (UnitKind, Option<MediaTask>) {
    match kind {
        DocumentUnitKind::Heading(level) => (UnitKind::Heading(level), None),
        DocumentUnitKind::Paragraph => (UnitKind::Paragraph, None),
        DocumentUnitKind::ListItem { depth } => (UnitKind::ListItem { depth }, None),
        DocumentUnitKind::ListChild { depth, kind } => (
            UnitKind::ListChild {
                depth,
                kind: convert_list_child(kind),
            },
            None,
        ),
        DocumentUnitKind::Quote => (UnitKind::Quote, None),
        DocumentUnitKind::Code {
            list_depth,
            language,
        } => (
            UnitKind::Code {
                list_depth,
                language: language.map(str::to_owned),
            },
            None,
        ),
        DocumentUnitKind::Rule { list_depth } => (UnitKind::Rule { list_depth }, None),
        DocumentUnitKind::Table { list_depth, rows } => (
            UnitKind::Table {
                list_depth,
                rows: rows.into_iter().map(convert_table_row).collect(),
            },
            None,
        ),
        DocumentUnitKind::Media { source, list_depth } => {
            let media = match &source {
                DocumentMedia::Image(_) => Media::Image,
                DocumentMedia::Mermaid(src) => Media::Diagram {
                    source: Arc::from(src.as_str()),
                },
                DocumentMedia::Math {
                    source: tex,
                    display,
                } => Media::Equation {
                    source: Arc::from(tex.as_str()),
                    display: *display,
                },
            };
            let task = MediaTask {
                unit: index,
                source,
                alt: text.to_owned(),
            };
            (UnitKind::Media { list_depth, media }, Some(task))
        }
        DocumentUnitKind::Gap => (UnitKind::Gap, None),
    }
}

fn convert_list_child(kind: DocumentListChildKind) -> ListChildKind {
    match kind {
        DocumentListChildKind::Paragraph => ListChildKind::Paragraph,
        DocumentListChildKind::Heading(level) => ListChildKind::Heading(level),
        DocumentListChildKind::Quote => ListChildKind::Quote,
        DocumentListChildKind::Code(lang) => ListChildKind::Code(lang.map(str::to_owned)),
    }
}

fn convert_table_row(row: Vec<DocumentTableCellLayout>) -> Vec<Cell> {
    row.into_iter()
        .map(|cell| {
            let spans = char_spans(&cell.text, convert_spans(cell.spans));
            let base = if cell.header {
                Cell::header(cell.text)
            } else {
                Cell::new(cell.text)
            };
            base.spans(spans)
        })
        .collect()
}

fn convert_spans(
    spans: Vec<DocumentSpan>,
) -> impl Iterator<Item = (std::ops::Range<usize>, SpanStyle)> {
    spans
        .into_iter()
        .map(|span| (span.range, convert_style(span.style)))
}

fn convert_style(style: DocumentSpanStyle) -> SpanStyle {
    match style {
        DocumentSpanStyle::Accent => SpanStyle::Accent,
        DocumentSpanStyle::Bold => SpanStyle::Bold,
        DocumentSpanStyle::Italic => SpanStyle::Italic,
        DocumentSpanStyle::Strikethrough => SpanStyle::Strike,
        DocumentSpanStyle::Monospace => SpanStyle::Mono,
        DocumentSpanStyle::Underline => SpanStyle::Underline,
        DocumentSpanStyle::Link(uri) => SpanStyle::Link(uri),
        DocumentSpanStyle::Math(tex) => SpanStyle::Math(tex),
    }
}
