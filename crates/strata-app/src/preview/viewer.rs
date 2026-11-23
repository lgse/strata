// SPDX-License-Identifier: MIT

use rune::{Cx, El, Tx, col, text};
use rune_kit::{Button, Spinner};
use strata_components::{
    code_view::{CodeIntent, CodeState, CodeView, highlight::Highlighter},
    contract::EntryId,
    document_view::DocumentState,
    preview_drawer::{DocumentView, FindStatus, HostedPreview},
    sheet_grid::SheetGridState,
    viewers::{
        archive::{ArchiveDir, ArchiveState, ArchiveTree, PromptState},
        viewport::ViewportState,
    },
};
use strata_core::i18n::tr;
use strata_kit::look::Look;

use super::{
    archive,
    document::{self, DecodedMedia, MediaTask, RenderedDocument},
    image::{self, ImageViewer},
    labels,
    media::{self, MediaViewer},
    model::{self, ModelViewer},
    pdf::{self, PdfViewer},
    route::{self, RouteViewer},
    sheet,
};
use crate::{App, buttons};

pub(super) struct Detached {
    pub(super) viewport: ViewportState,
    pub(super) model: strata_components::viewers::model::ModelState,
    pub(super) route: strata_components::viewers::route::RouteState,
    pub(super) pdf: strata_components::pdf_view::PdfState,
    pub(super) media: strata_components::media_player::MediaState,
    code: CodeState,
    document: DocumentState,
    sheet: SheetGridState,
    archive: ArchiveState,
    prompt: PromptState,
}

impl Default for Detached {
    fn default() -> Self {
        // An empty tree reserves no row ids: only its children would.
        let tree = ArchiveTree::new(ArchiveDir::default(), || EntryId::new(u64::MAX));
        Self {
            viewport: ViewportState::default(),
            model: Default::default(),
            route: Default::default(),
            pdf: Default::default(),
            media: Default::default(),
            code: CodeState::default(),
            document: DocumentState::default(),
            sheet: SheetGridState::new("preview-detached-sheet"),
            archive: ArchiveState::new("preview-detached-archive", tree),
            prompt: PromptState::default(),
        }
    }
}

/// A viewer's accessor falling back to [`Detached`]: says so, and hands the spare state on.
pub(super) fn spare<T>(viewer: &'static str, state: T) -> T {
    tracing::debug!(
        viewer,
        "a handler of a viewer that has gone landed in its spare state"
    );
    state
}

/// Key of the read-only source/text viewer inside the preview drawer.
pub(crate) const CODE_KEY: &str = "preview-code";

pub(super) enum RenderedBody {
    Document(Box<RenderedDocument>),
    Sheet(Box<SheetGridState>),
}

pub(super) struct DocumentViewer {
    active: DocumentView,
    rendered: Option<RenderedBody>,
    source: Option<Box<CodeState>>,
    render_pending: bool,
}

/// The key of a feedback card, such as `No visual preview`, that takes the keyboard on Right.
pub(super) const MESSAGE_KEY: &str = "preview-message";

pub(super) enum Viewer {
    Empty,
    Loading,
    Code(Box<CodeState>),
    Document(DocumentViewer),
    Image(Box<ImageViewer>),
    Pdf(Box<PdfViewer>),
    Model(Box<ModelViewer>),
    Route(Box<RouteViewer>),
    Media(Box<MediaViewer>),
    /// A preview whose pictures are being decoded off the UI thread, shown as loading.
    Decoding(u64),
    Archive(Box<ArchiveState>),
    ArchivePrompt(Box<PromptState>),
    Message {
        title: String,
        detail: Option<String>,
        can_open: bool,
    },
}

/// Source text and metadata handed to [`Viewer::code`] or [`build_code_state`].
pub(super) struct CodeSource<'a> {
    pub(super) content: String,
    pub(super) truncated: bool,
    pub(super) wrap: bool,
    pub(super) file_name: &'a str,
    pub(super) fallback_reason: Option<&'a str>,
}

impl Viewer {
    pub(super) fn is_empty(&self) -> bool {
        matches!(self, Self::Empty)
    }

    #[cfg(test)]
    pub(super) fn is_sheet(&self) -> bool {
        matches!(
            self,
            Self::Document(DocumentViewer {
                rendered: Some(RenderedBody::Sheet(_)),
                ..
            })
        )
    }

    pub(super) fn can_focus(&self) -> bool {
        match self {
            Self::Code(_)
            | Self::Document(_)
            | Self::Image(_)
            | Self::Pdf(_)
            | Self::Model(_)
            | Self::Route(_)
            | Self::Media(_)
            | Self::Archive(_)
            | Self::ArchivePrompt(_) => true,
            Self::Message { can_open, .. } => *can_open,
            Self::Empty | Self::Loading | Self::Decoding(_) => false,
        }
    }

    pub(super) fn needs_body_focus_owner(&self) -> bool {
        matches!(
            self,
            Self::Message {
                can_open: false,
                ..
            }
        )
    }

    pub(super) fn is_prompt_focused(&self) -> bool {
        matches!(self, Self::ArchivePrompt(prompt) if prompt.edit.focused)
    }

    pub(super) fn leaves_on_left(&self) -> bool {
        match self {
            Self::Archive(state) => state.at_root(),
            Self::ArchivePrompt(prompt) => !prompt.edit.focused,
            _ => true,
        }
    }

    pub(super) fn focus_into(&mut self, tx: &mut Tx) -> bool {
        match self {
            Self::Archive(state) => {
                if state.passive() {
                    state.enter();
                }
                tx.focus_when_shown(archive::ARCHIVE_KEY, true);
                true
            }
            Self::ArchivePrompt(prompt) => {
                prompt.focus();
                tx.focus_when_shown(archive::PROMPT_FIELD_KEY, true);
                true
            }
            _ => false,
        }
    }

    pub(super) fn code(source: CodeSource<'_>, highlighter: &Highlighter) -> Self {
        Self::Code(Box::new(build_code_state(source, highlighter)))
    }

    pub(super) fn rendered_only(rendered: RenderedBody) -> Self {
        Self::Document(DocumentViewer {
            active: DocumentView::Rendered,
            rendered: Some(rendered),
            source: None,
            render_pending: false,
        })
    }

    pub(super) fn document(
        active: DocumentView,
        rendered: Option<RenderedBody>,
        source: CodeState,
        render_pending: bool,
    ) -> Self {
        let initial = if active == DocumentView::Rendered && rendered.is_some() {
            DocumentView::Rendered
        } else {
            DocumentView::Source
        };
        Self::Document(DocumentViewer {
            active: initial,
            rendered,
            source: Some(Box::new(source)),
            render_pending,
        })
    }

    pub(super) fn stop(&mut self) {
        match self {
            Self::Document(doc) => doc.stop(),
            Self::Pdf(pdf) => pdf.stop(),
            Self::Model(model) => model.stop(),
            Self::Route(route) => route.state.pause(),
            Self::Media(media) => media.stop(),
            _ => {}
        }
    }

    pub(super) fn code_state(&self) -> Option<&CodeState> {
        match self {
            Self::Code(code) => Some(code),
            Self::Document(doc) if doc.active == DocumentView::Source => doc.source.as_deref(),
            _ => None,
        }
    }

    pub(super) fn scroll_target(&mut self) -> Option<ScrollTarget> {
        match self {
            Self::Code(code) => Some(ScrollTarget::code(code)),
            Self::Document(doc) => doc.scroll_target(),
            Self::Pdf(pdf) => {
                pdf.state.clear_anchor();
                Some(ScrollTarget {
                    key: pdf.state.scroll_key(),
                    pos: pdf.state.scroll_pos(),
                    kind: ScrollTargetKind::Pdf,
                })
            }
            _ => None,
        }
    }

    /// Current monotonic identity of the active text viewer, if one is shown.
    pub(super) fn text_viewer_stamp(&self) -> Option<crate::menus::TextViewerStamp> {
        match self {
            Self::Code(code) => Some(crate::menus::TextViewerStamp::Code(code.revision())),
            Self::Document(doc) => doc.text_viewer_stamp(),
            Self::Pdf(pdf) => Some(crate::menus::TextViewerStamp::Pdf(pdf.id())),
            _ => None,
        }
    }

    /// Context-menu snapshot for read-only preview text viewers.
    pub(super) fn text_menu_info(
        &self,
        sel: Option<&rune::TextSelection>,
    ) -> Option<TextMenuSnapshot> {
        match self {
            Self::Code(code) => Some(code_menu_snapshot(code, sel)),
            Self::Document(doc) => doc.text_menu_info(sel),
            Self::Pdf(pdf) => {
                let selected = pdf.state.selected_text();
                Some(TextMenuSnapshot {
                    stamp: crate::menus::TextViewerStamp::Pdf(pdf.id()),
                    focus_key: pdf::PDF_KEY,
                    selected: (!selected.is_empty()).then_some(selected),
                    restore: None,
                    can_select_all: pdf.state.has_selectable_text(),
                })
            }
            _ => None,
        }
    }

    /// Restores a preview text selection that Rune cleared while the modal context menu was open.
    pub(super) fn restore_text_selection(
        &mut self,
        saved: &crate::menus::SavedPreviewSelection,
        tx: &mut Tx,
    ) {
        match saved {
            crate::menus::SavedPreviewSelection::All => {
                let _ = self.select_all_text(tx);
            }
            crate::menus::SavedPreviewSelection::Node(selector, range) => {
                tx.select_text(selector.clone(), range.clone());
            }
            crate::menus::SavedPreviewSelection::CodeRange(lo, hi) => {
                if let Some(code) = self.code_state_mut() {
                    code.restore_selection_range(*lo, *hi, tx);
                }
            }
        }
    }

    fn code_state_mut(&mut self) -> Option<&mut CodeState> {
        match self {
            Self::Code(code) => Some(code),
            Self::Document(doc) if doc.active == DocumentView::Source => doc.source.as_deref_mut(),
            _ => None,
        }
    }

    /// Selects all text in the active preview text viewer.
    pub(super) fn select_all_text(&mut self, tx: &mut Tx) -> bool {
        match self {
            Self::Code(code) => {
                code.select_all_text(tx);
                true
            }
            Self::Document(doc) => doc.select_all_text(tx),
            Self::Pdf(pdf) => pdf.state.select_all(),
            _ => false,
        }
    }

    /// Scrolls the active `CodeView` to its exact start or end via its position controller.
    pub(super) fn scroll_code_edge(&self, end: bool, tx: &mut Tx) -> bool {
        let Some(code) = self.code_state() else {
            return false;
        };
        code.scroll_to_edge(end, tx);
        true
    }

    pub(super) fn reset_viewport(&mut self, tx: &mut Tx) {
        if let Self::Image(img) = self {
            img.reset();
        }
        tx.clear_text_selection();
        for key in [
            CODE_KEY,
            super::document::DOCUMENT_KEY,
            super::sheet::SHEET_KEY,
        ] {
            tx.scroll_to_instant(format!("{key}-scroll"), 0.0, 0.0);
        }
    }

    pub(super) fn take_render_pending(&mut self) -> bool {
        match self {
            Self::Document(doc) => std::mem::replace(&mut doc.render_pending, false),
            _ => false,
        }
    }

    pub(super) fn show_view(&mut self, view: DocumentView) -> bool {
        match self {
            Self::Document(doc) => doc.show_view(view),
            _ => false,
        }
    }

    pub(super) fn on_media_ready(
        &mut self,
        doc_id: u64,
        task: &MediaTask,
        outcome: DecodedMedia,
        look: &Look,
    ) -> bool {
        let Self::Document(DocumentViewer {
            rendered: Some(RenderedBody::Document(doc)),
            ..
        }) = self
        else {
            return false;
        };
        doc.on_media_ready(doc_id, task, outcome, look)
    }

    pub(super) fn sync_theme(&mut self, look: &Look) -> bool {
        match self {
            Self::Document(doc) => doc.sync_theme(look),
            _ => false,
        }
    }

    pub(super) fn poll(&mut self, clock: f64, look: &Look, tx: &mut Tx) -> bool {
        match self {
            Self::Code(code) => code.poll(tx),
            Self::Document(doc) => doc.poll(clock, look, tx),
            _ => false,
        }
    }

    pub(super) fn poll_async(&mut self, tx: &mut Tx) -> bool {
        match self {
            Self::Code(code) => code.poll(tx),
            Self::Document(doc) => doc.poll_async(tx),
            _ => false,
        }
    }

    pub(super) fn set_wrap(&mut self, wrap: bool) {
        match self {
            Self::Code(code) => code.set_wrap(wrap),
            Self::Document(doc) => doc.set_wrap(wrap),
            _ => {}
        }
    }

    fn active_hosted_mut(&mut self) -> Option<&mut dyn HostedPreview> {
        match self {
            Self::Code(code) => Some(&mut **code),
            Self::Document(doc) => doc.active_hosted_mut(),
            Self::Pdf(pdf) => Some(&mut pdf.state),
            _ => None,
        }
    }

    pub(super) fn build(&self, app: &App, cx: &mut Cx<App>) -> Option<El> {
        match self {
            Self::Empty => None,
            Self::Loading | Self::Decoding(_) => Some(loading_view(&app.look)),
            Self::Code(_) => Some(code_view(app, cx, code_ref, code_mut)),
            Self::Document(doc) => doc.build(app, cx),
            Self::Image(img) => Some(image::build_view(img, app, cx, image_viewport_mut)),
            Self::Pdf(_) => Some(pdf::build_view(app, cx)),
            Self::Model(model) => Some(model::build_view(model, app, cx)),
            Self::Route(route) => Some(route::build_view(route, app, cx)),
            Self::Media(player) => Some(media::build_view(player, app, cx)),
            Self::Archive(_) => Some(archive::build_archive_view(app, archive_ref, archive_mut)),
            Self::ArchivePrompt(prompt) => {
                Some(archive::build_prompt_view(prompt, &app.look, prompt_mut))
            }
            Self::Message {
                title,
                detail,
                can_open,
            } => Some(message_view(app, title, detail.as_deref(), *can_open)),
        }
    }
}

pub(super) fn build_code_state(source: CodeSource<'_>, highlighter: &Highlighter) -> CodeState {
    let mut state = CodeState::new(source.content).key(CODE_KEY).highlighter(
        highlighter,
        Some(source.file_name),
        None,
    );
    state.set_wrap(source.wrap);
    let mut notices = Vec::new();
    if let Some(reason) = source.fallback_reason {
        notices.push(tr(reason));
    }
    if source.truncated {
        notices.push(tr("Preview limited to the first 1 MB"));
    }
    if !notices.is_empty() {
        state.set_notices(notices);
    }
    state
}

/// What a scrollable preview viewer exposes for keyboard scrolling.
pub(super) struct ScrollTarget {
    pub(super) key: String,
    pub(super) pos: rune::ScrollPos,
    pub(super) kind: ScrollTargetKind,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum ScrollTargetKind {
    Code,
    Document,
    Pdf,
}

impl ScrollTarget {
    fn code(code: &CodeState) -> Self {
        Self {
            key: code.scroll_key(),
            pos: code.scroll_pos(),
            kind: ScrollTargetKind::Code,
        }
    }
}

/// Captured selection metadata for opening a preview text viewer's context menu.
pub(super) struct TextMenuSnapshot {
    pub(super) stamp: crate::menus::TextViewerStamp,
    pub(super) focus_key: &'static str,
    pub(super) selected: Option<String>,
    pub(super) restore: Option<crate::menus::SavedPreviewSelection>,
    pub(super) can_select_all: bool,
}

fn single_node_selection(
    sel: Option<&rune::TextSelection>,
) -> Option<crate::menus::SavedPreviewSelection> {
    let sel = sel?;
    let mut selected = sel.texts().iter().filter(|t| t.is_selected());
    let first = selected.next()?;
    if selected.next().is_some() {
        return None;
    }
    let selector = first
        .path()
        .split('/')
        .filter(|seg| !seg.starts_with('#'))
        .collect::<Vec<_>>()
        .join("/");
    (!selector.is_empty())
        .then(|| crate::menus::SavedPreviewSelection::Node(selector, first.range()))
}

fn code_menu_snapshot(code: &CodeState, sel: Option<&rune::TextSelection>) -> TextMenuSnapshot {
    let selected = code.selected_text(sel);
    let restore = if selected.is_none() {
        None
    } else if code.selects_whole(sel) {
        Some(crate::menus::SavedPreviewSelection::All)
    } else if let Some((lo, hi)) = code.keyboard_selection_for(sel) {
        Some(crate::menus::SavedPreviewSelection::CodeRange(lo, hi))
    } else if let Some(node) = single_node_selection(sel) {
        Some(node)
    } else {
        code.selection_span(sel)
            .map(|(lo, hi)| crate::menus::SavedPreviewSelection::CodeRange(lo, hi))
    };
    TextMenuSnapshot {
        stamp: crate::menus::TextViewerStamp::Code(code.revision()),
        focus_key: CODE_KEY,
        selected,
        restore,
        can_select_all: code.has_text(),
    }
}

fn document_menu_snapshot(
    doc: &DocumentState,
    sel: Option<&rune::TextSelection>,
) -> TextMenuSnapshot {
    let selected = doc.selected_text(sel);
    let restore = if selected.is_none() {
        None
    } else if doc.selects_whole(sel) {
        Some(crate::menus::SavedPreviewSelection::All)
    } else {
        single_node_selection(sel)
    };
    TextMenuSnapshot {
        stamp: crate::menus::TextViewerStamp::Document(doc.content_id()),
        focus_key: document::DOCUMENT_KEY,
        selected,
        restore,
        can_select_all: doc.has_text(),
    }
}

impl DocumentViewer {
    fn text_viewer_stamp(&self) -> Option<crate::menus::TextViewerStamp> {
        match self.active {
            DocumentView::Rendered => match self.rendered.as_ref()? {
                RenderedBody::Document(doc) => Some(crate::menus::TextViewerStamp::Document(
                    doc.state.content_id(),
                )),
                RenderedBody::Sheet(_) => None,
            },
            DocumentView::Source => self
                .source
                .as_deref()
                .map(|code| crate::menus::TextViewerStamp::Code(code.revision())),
        }
    }
    #[cfg(test)]
    pub(super) fn is_rendered_document_for_test(&self) -> bool {
        self.active == DocumentView::Rendered
            && matches!(self.rendered, Some(RenderedBody::Document(_)))
    }

    #[cfg(test)]
    pub(super) fn scroll_offset_for_test(&self) -> Option<f32> {
        self.scroll_target().map(|target| target.pos.offset[1])
    }

    fn scroll_target(&self) -> Option<ScrollTarget> {
        match self.active {
            DocumentView::Rendered => match self.rendered.as_ref()? {
                RenderedBody::Document(doc) => Some(ScrollTarget {
                    key: doc.state.scroll_key(),
                    pos: doc.state.scroll_pos(),
                    kind: ScrollTargetKind::Document,
                }),
                RenderedBody::Sheet(sheet) => Some(ScrollTarget {
                    key: sheet.scroll_key(),
                    pos: sheet.scroll_pos(),
                    kind: ScrollTargetKind::Document,
                }),
            },
            DocumentView::Source => self.source.as_deref().map(ScrollTarget::code),
        }
    }

    fn text_menu_info(&self, sel: Option<&rune::TextSelection>) -> Option<TextMenuSnapshot> {
        match self.active {
            DocumentView::Rendered => match self.rendered.as_ref()? {
                RenderedBody::Document(doc) => Some(document_menu_snapshot(&doc.state, sel)),
                RenderedBody::Sheet(_) => None,
            },
            DocumentView::Source => self
                .source
                .as_deref()
                .map(|code| code_menu_snapshot(code, sel)),
        }
    }

    fn select_all_text(&mut self, tx: &mut Tx) -> bool {
        match self.active {
            DocumentView::Rendered => match self.rendered.as_mut() {
                Some(RenderedBody::Document(doc)) => {
                    doc.state.select_all_text(tx);
                    true
                }
                _ => false,
            },
            DocumentView::Source => match self.source.as_deref_mut() {
                Some(code) => {
                    code.select_all_text(tx);
                    true
                }
                None => false,
            },
        }
    }

    fn active_hosted(&self) -> Option<&dyn HostedPreview> {
        match self.active {
            DocumentView::Rendered => match self.rendered.as_ref()? {
                RenderedBody::Document(doc) => Some(&doc.state),
                RenderedBody::Sheet(sheet) => Some(&**sheet),
            },
            DocumentView::Source => Some(&**self.source.as_ref()?),
        }
    }

    fn active_hosted_mut(&mut self) -> Option<&mut dyn HostedPreview> {
        match self.active {
            DocumentView::Rendered => match self.rendered.as_mut()? {
                RenderedBody::Document(doc) => Some(&mut doc.state),
                RenderedBody::Sheet(sheet) => Some(&mut **sheet),
            },
            DocumentView::Source => Some(&mut **self.source.as_mut()?),
        }
    }

    fn stop(&mut self) {
        if let Some(body) = &mut self.rendered {
            match body {
                RenderedBody::Document(doc) => doc.stop(),
                RenderedBody::Sheet(sheet) => sheet.stop(),
            }
        }
    }

    fn show_view(&mut self, view: DocumentView) -> bool {
        let can_show = match view {
            DocumentView::Rendered => self.rendered.is_some(),
            DocumentView::Source => self.source.is_some(),
        };
        if !can_show || self.active == view {
            return false;
        }
        if let Some(RenderedBody::Sheet(sheet)) = &mut self.rendered {
            sheet.stop();
        }
        self.active = view;
        true
    }

    fn set_wrap(&mut self, wrap: bool) {
        if let Some(code) = &mut self.source {
            code.set_wrap(wrap);
        }
        if let Some(RenderedBody::Document(doc)) = &mut self.rendered
            && doc.state.wrap().is_some()
        {
            doc.state.set_wrap(Some(wrap));
        }
    }

    #[cfg(test)]
    pub(super) fn rendered_document_for_test(&self) -> Option<&RenderedDocument> {
        match self.rendered.as_ref() {
            Some(RenderedBody::Document(doc)) => Some(doc),
            _ => None,
        }
    }

    fn sync_theme(&mut self, look: &Look) -> bool {
        match &mut self.rendered {
            Some(RenderedBody::Document(doc)) => doc.sync_theme(look),
            _ => false,
        }
    }

    fn poll(&mut self, clock: f64, look: &Look, tx: &mut Tx) -> bool {
        match self.active {
            DocumentView::Rendered => match &mut self.rendered {
                Some(RenderedBody::Document(doc)) => {
                    let ticked = doc.state.tick(clock);
                    let flushed = doc.state.flush(tx);
                    ticked | flushed
                }
                Some(RenderedBody::Sheet(sheet)) => sheet.tick(clock, look, tx),
                None => false,
            },
            DocumentView::Source => self.source.as_mut().is_some_and(|code| code.poll(tx)),
        }
    }

    fn poll_async(&mut self, tx: &mut Tx) -> bool {
        match self.active {
            DocumentView::Rendered => match &mut self.rendered {
                Some(RenderedBody::Document(doc)) => doc.state.flush(tx),
                Some(RenderedBody::Sheet(_)) | None => false,
            },
            DocumentView::Source => self.source.as_mut().is_some_and(|code| code.poll(tx)),
        }
    }

    fn build(&self, app: &App, cx: &mut Cx<App>) -> Option<El> {
        match self.active {
            DocumentView::Rendered => match self.rendered.as_ref()? {
                RenderedBody::Document(doc) => Some(document::build_view(
                    app,
                    cx,
                    doc.blocks(),
                    (doc_state_ref, doc_state_mut),
                )),
                RenderedBody::Sheet(_) => {
                    Some(sheet::build_view(app, cx, sheet_state_ref, sheet_state_mut))
                }
            },
            DocumentView::Source => {
                self.source.as_ref()?;
                Some(code_view(app, cx, doc_code_ref, doc_code_mut))
            }
        }
    }
}

impl HostedPreview for Viewer {
    fn document_view(&self) -> Option<DocumentView> {
        match self {
            Self::Code(code) => code.document_view(),
            Self::Document(doc) => match doc.active {
                DocumentView::Rendered => doc.source.is_some().then_some(DocumentView::Rendered),
                DocumentView::Source => {
                    (doc.rendered.is_some() || doc.render_pending).then_some(DocumentView::Source)
                }
            },
            _ => None,
        }
    }

    fn wrap(&self) -> Option<bool> {
        match self {
            Self::Code(code) => code.wrap(),
            Self::Document(doc) => doc.active_hosted()?.wrap(),
            _ => None,
        }
    }

    fn searchable(&self) -> bool {
        match self {
            Self::Code(code) => code.searchable(),
            Self::Document(doc) => doc.active_hosted().is_some_and(HostedPreview::searchable),
            Self::Pdf(pdf) => pdf.state.searchable(),
            _ => false,
        }
    }

    fn find(&mut self, query: &str) {
        if let Some(hosted) = self.active_hosted_mut() {
            hosted.find(query);
        }
    }

    fn find_step(&mut self, forward: bool) {
        if let Some(hosted) = self.active_hosted_mut() {
            hosted.find_step(forward);
        }
    }

    fn find_status(&self) -> Option<FindStatus> {
        match self {
            Self::Code(code) => code.find_status(),
            Self::Document(doc) => doc.active_hosted()?.find_status(),
            Self::Pdf(pdf) => pdf.state.find_status(),
            _ => None,
        }
    }

    fn find_end(&mut self) {
        match self {
            Self::Pdf(pdf) => pdf.find_end(),
            _ => {
                if let Some(hosted) = self.active_hosted_mut() {
                    hosted.find_end();
                }
            }
        }
    }

    fn busy(&self) -> Option<&str> {
        match self {
            Self::Code(code) => code.busy(),
            Self::Model(model) => model.state.busy(),
            Self::Document(doc) => doc.active_hosted()?.busy(),
            Self::Pdf(pdf) => pdf.state.busy(),
            _ => None,
        }
    }
}

fn shown<'a, T>(
    app: &'a App,
    viewer: &'static str,
    pick: fn(&Viewer) -> Option<&T>,
    spare_of: fn(&Detached) -> &T,
) -> &'a T {
    let preview = &app.preview;
    pick(&preview.viewer).unwrap_or_else(|| spare(viewer, spare_of(&preview.detached)))
}

/// [`shown`], to change.
fn shown_mut<'a, T>(
    app: &'a mut App,
    viewer: &'static str,
    pick: fn(&mut Viewer) -> Option<&mut T>,
    spare_of: fn(&mut Detached) -> &mut T,
) -> &'a mut T {
    let preview = &mut app.preview;
    match pick(&mut preview.viewer) {
        Some(state) => state,
        None => spare(viewer, spare_of(&mut preview.detached)),
    }
}

impl Viewer {
    fn document_source(&self) -> Option<&CodeState> {
        match self {
            Self::Document(doc) => doc.source.as_deref(),
            _ => None,
        }
    }

    fn document_source_mut(&mut self) -> Option<&mut CodeState> {
        match self {
            Self::Document(doc) => doc.source.as_deref_mut(),
            _ => None,
        }
    }

    fn rendered(&self) -> Option<&RenderedBody> {
        match self {
            Self::Document(doc) => doc.rendered.as_ref(),
            _ => None,
        }
    }

    fn rendered_mut(&mut self) -> Option<&mut RenderedBody> {
        match self {
            Self::Document(doc) => doc.rendered.as_mut(),
            _ => None,
        }
    }
}

fn code_ref(app: &App) -> &CodeState {
    let pick: fn(&Viewer) -> Option<&CodeState> = |viewer| match viewer {
        Viewer::Code(code) => Some(code),
        _ => None,
    };
    shown(app, "code", pick, |spare| &spare.code)
}

fn code_mut(app: &mut App) -> &mut CodeState {
    let pick: fn(&mut Viewer) -> Option<&mut CodeState> = |viewer| match viewer {
        Viewer::Code(code) => Some(code),
        _ => None,
    };
    shown_mut(app, "code", pick, |spare| &mut spare.code)
}

fn doc_code_ref(app: &App) -> &CodeState {
    shown(app, "document source", Viewer::document_source, |spare| {
        &spare.code
    })
}

fn doc_code_mut(app: &mut App) -> &mut CodeState {
    let pick = Viewer::document_source_mut;
    shown_mut(app, "document source", pick, |spare| &mut spare.code)
}

fn doc_state_ref(app: &App) -> &DocumentState {
    let pick: fn(&Viewer) -> Option<&DocumentState> = |viewer| match viewer.rendered()? {
        RenderedBody::Document(doc) => Some(&doc.state),
        RenderedBody::Sheet(_) => None,
    };
    shown(app, "document", pick, |spare| &spare.document)
}

fn doc_state_mut(app: &mut App) -> &mut DocumentState {
    let pick: fn(&mut Viewer) -> Option<&mut DocumentState> =
        |viewer| match viewer.rendered_mut()? {
            RenderedBody::Document(doc) => Some(&mut doc.state),
            RenderedBody::Sheet(_) => None,
        };
    shown_mut(app, "document", pick, |spare| &mut spare.document)
}

fn sheet_state_ref(app: &App) -> &SheetGridState {
    let pick: fn(&Viewer) -> Option<&SheetGridState> = |viewer| match viewer.rendered()? {
        RenderedBody::Sheet(sheet) => Some(sheet),
        RenderedBody::Document(_) => None,
    };
    shown(app, "sheet", pick, |spare| &spare.sheet)
}

fn sheet_state_mut(app: &mut App) -> &mut SheetGridState {
    let pick: fn(&mut Viewer) -> Option<&mut SheetGridState> =
        |viewer| match viewer.rendered_mut()? {
            RenderedBody::Sheet(sheet) => Some(sheet),
            RenderedBody::Document(_) => None,
        };
    shown_mut(app, "sheet", pick, |spare| &mut spare.sheet)
}

fn image_viewport_mut(app: &mut App) -> &mut ViewportState {
    let pick: fn(&mut Viewer) -> Option<&mut ViewportState> = |viewer| match viewer {
        Viewer::Image(image) => Some(&mut image.viewport),
        _ => None,
    };
    shown_mut(app, "image", pick, |spare| &mut spare.viewport)
}

pub(super) fn tick_archive_if_active(app: &mut App, tx: &mut Tx) -> bool {
    if matches!(app.preview.viewer, Viewer::Archive(_)) {
        archive::tick_archive(app, archive_ref, archive_mut, tx)
    } else {
        false
    }
}

fn archive_ref(app: &App) -> &ArchiveState {
    let pick: fn(&Viewer) -> Option<&ArchiveState> = |viewer| match viewer {
        Viewer::Archive(state) => Some(state),
        _ => None,
    };
    shown(app, "archive", pick, |spare| &spare.archive)
}

fn archive_mut(app: &mut App) -> &mut ArchiveState {
    let pick: fn(&mut Viewer) -> Option<&mut ArchiveState> = |viewer| match viewer {
        Viewer::Archive(state) => Some(state),
        _ => None,
    };
    shown_mut(app, "archive", pick, |spare| &mut spare.archive)
}

fn prompt_mut(app: &mut App) -> &mut PromptState {
    let pick: fn(&mut Viewer) -> Option<&mut PromptState> = |viewer| match viewer {
        Viewer::ArchivePrompt(prompt) => Some(prompt),
        _ => None,
    };
    shown_mut(app, "archive prompt", pick, |spare| &mut spare.prompt)
}

fn code_view(
    app: &App,
    cx: &mut Cx<App>,
    get: fn(&App) -> &CodeState,
    get_mut: fn(&mut App) -> &mut CodeState,
) -> El {
    CodeView::new(get, get_mut)
        .labels(labels::code())
        .document_home_end(crate::shell::preferences(app).tenxer_mode)
        .on_intent(|app: &mut App, intent, tx| match intent {
            CodeIntent::Leave => {
                super::return_to_files(app);
                tx.wake_after(std::time::Duration::ZERO);
            }
            CodeIntent::ContextMenu(at) => {
                super::open_text_context_menu(app, at, tx);
            }
            _ => {}
        })
        .build(app, &app.look, cx)
        .no_focus_ring()
}

fn loading_view(look: &Look) -> El {
    let label = tr("Loading…");
    col()
        .key("preview-loading")
        .role("progressbar")
        .label(label)
        .busy(true)
        .w_full()
        .h_full()
        .center()
        .child(Spinner::new().build(&look.kit))
}

fn message_view(app: &App, title: &str, detail: Option<&str>, can_open: bool) -> El {
    let look = &app.look;
    let mut card = col()
        .key(MESSAGE_KEY)
        .w_full()
        .h_full()
        .px(look.space.xl)
        .py(look.space.xl)
        .gap(look.space.sm)
        .center()
        .child(
            text(title.to_owned())
                .font(look.kit.ty.ui_medium)
                .color(look.kit.c.text_base),
        );
    if !can_open && app.preview.content_focus != super::ContentFocus::None {
        card = card.focusable(true).no_focus_ring();
    }
    if let Some(detail) = detail.filter(|detail| !detail.is_empty()) {
        card = card.child(
            text(detail.to_owned())
                .font(look.kit.ty.ui)
                .color(look.kit.c.text_muted),
        );
    }
    if can_open {
        let open_label = tr("Open");
        let button = Button::new(&open_label)
            .on_press(|app: &mut App| super::open_current(app))
            .build(&look.kit);
        card = card.child(buttons::on_return(
            "preview-open-external".to_owned(),
            button,
        ));
    }
    card
}
