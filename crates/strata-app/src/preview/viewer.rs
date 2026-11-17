// SPDX-License-Identifier: MIT

//! The hosted preview viewer seam inside [`super::Preview`].
//!
//! `A15` mounts C9's [`PreviewDrawer`](strata_components::preview_drawer::PreviewDrawer) once in
//! [`Place::Drawer`](crate::slots::Place::Drawer) and delegates header capabilities, find, and
//! content rendering through [`Viewer`]:
//! - `A15` owns plain text and code (`Viewer::Code`), rendered documents and spreadsheets
//!   (`Viewer::Document`), and still images in Slice 3.
//! - `A16` plugs PDF (`PdfView`), audio/video (`MediaPlayer`), 3D models (`ModelView`), and GPS
//!   routes (`RouteView`) in as new [`Viewer`] variants and arms in [`super::load`].
//! - `A13` plugs archive trees (`ArchiveTree`) and archive password prompts in as new [`Viewer`]
//!   variants and arms in [`super::load`].
//!
//! Every element key inside a viewer starts with `"preview-"` so tab-switch focus restoration
//! (`Registry::in_panes`) and command guards (`Situation::within("preview")`) cover it.

use rune::{Cx, El, Tx, col, text};
use rune_kit::{Button, Spinner};
use strata_components::{
    code_view::{CodeIntent, CodeState, CodeView, highlight::Highlighter},
    document_view::DocumentState,
    preview_drawer::{DocumentView, FindStatus, HostedPreview},
    sheet_grid::SheetGridState,
    viewers::{
        archive::{ArchiveState, PromptState},
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
    model::{self, ModelViewer},
    pdf::{self, PdfViewer},
    route::{self, RouteViewer},
    sheet,
};
use crate::{App, buttons};

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

    /// Focuses directly into an archive tree or password prompt when one is active, returning
    /// `true` if handled.
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
            _ => {}
        }
    }

    /// The source text shown now, which scrolls by lines (10xer's J and K): a text file, or a
    /// document in its source view.
    pub(super) fn code_state(&self) -> Option<&CodeState> {
        match self {
            Self::Code(code) => Some(code),
            Self::Document(doc) if doc.active == DocumentView::Source => doc.source.as_deref(),
            _ => None,
        }
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
    ) -> bool {
        let Self::Document(DocumentViewer {
            rendered: Some(RenderedBody::Document(doc)),
            ..
        }) = self
        else {
            return false;
        };
        doc.on_media_ready(doc_id, task, outcome)
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

impl DocumentViewer {
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
            _ => None,
        }
    }

    fn find_end(&mut self) {
        if let Some(hosted) = self.active_hosted_mut() {
            hosted.find_end();
        }
    }

    fn busy(&self) -> Option<&str> {
        match self {
            Self::Code(code) => code.busy(),
            Self::Model(model) => model.state.busy(),
            Self::Document(doc) => doc.active_hosted()?.busy(),
            _ => None,
        }
    }
}

fn code_ref(app: &App) -> &CodeState {
    match &app.preview.viewer {
        Viewer::Code(code) => code,
        _ => unreachable!("CodeView built only while Viewer::Code is active"),
    }
}

fn code_mut(app: &mut App) -> &mut CodeState {
    match &mut app.preview.viewer {
        Viewer::Code(code) => code,
        _ => unreachable!("CodeView mutated only while Viewer::Code is active"),
    }
}

fn doc_code_ref(app: &App) -> &CodeState {
    match &app.preview.viewer {
        Viewer::Document(DocumentViewer {
            source: Some(code), ..
        }) => code,
        _ => unreachable!("Document source built only while source CodeState exists"),
    }
}

fn doc_code_mut(app: &mut App) -> &mut CodeState {
    match &mut app.preview.viewer {
        Viewer::Document(DocumentViewer {
            source: Some(code), ..
        }) => code,
        _ => unreachable!("Document source mutated only while source CodeState exists"),
    }
}

fn doc_state_ref(app: &App) -> &DocumentState {
    match &app.preview.viewer {
        Viewer::Document(DocumentViewer {
            rendered: Some(RenderedBody::Document(doc)),
            ..
        }) => &doc.state,
        _ => unreachable!("DocumentView built only while RenderedBody::Document is active"),
    }
}

fn doc_state_mut(app: &mut App) -> &mut DocumentState {
    match &mut app.preview.viewer {
        Viewer::Document(DocumentViewer {
            rendered: Some(RenderedBody::Document(doc)),
            ..
        }) => &mut doc.state,
        _ => unreachable!("DocumentView mutated only while RenderedBody::Document is active"),
    }
}

fn sheet_state_ref(app: &App) -> &SheetGridState {
    match &app.preview.viewer {
        Viewer::Document(DocumentViewer {
            rendered: Some(RenderedBody::Sheet(sheet)),
            ..
        }) => sheet,
        _ => unreachable!("SheetGrid built only while RenderedBody::Sheet is active"),
    }
}

fn sheet_state_mut(app: &mut App) -> &mut SheetGridState {
    match &mut app.preview.viewer {
        Viewer::Document(DocumentViewer {
            rendered: Some(RenderedBody::Sheet(sheet)),
            ..
        }) => sheet,
        _ => unreachable!("SheetGrid mutated only while RenderedBody::Sheet is active"),
    }
}

fn image_viewport_mut(app: &mut App) -> &mut ViewportState {
    match &mut app.preview.viewer {
        Viewer::Image(img) => &mut img.viewport,
        _ => unreachable!("ImageView mutated only while Viewer::Image is active"),
    }
}

pub(super) fn tick_archive_if_active(app: &mut App, tx: &mut Tx) -> bool {
    if matches!(app.preview.viewer, Viewer::Archive(_)) {
        archive::tick_archive(app, archive_ref, archive_mut, tx)
    } else {
        false
    }
}

fn archive_ref(app: &App) -> &ArchiveState {
    match &app.preview.viewer {
        Viewer::Archive(state) => state,
        _ => unreachable!("ArchiveView built only while Viewer::Archive is active"),
    }
}

fn archive_mut(app: &mut App) -> &mut ArchiveState {
    match &mut app.preview.viewer {
        Viewer::Archive(state) => state,
        _ => unreachable!("ArchiveView mutated only while Viewer::Archive is active"),
    }
}

fn prompt_mut(app: &mut App) -> &mut PromptState {
    match &mut app.preview.viewer {
        Viewer::ArchivePrompt(prompt) => prompt,
        _ => unreachable!("ArchivePrompt mutated only while Viewer::ArchivePrompt is active"),
    }
}

fn code_view(
    app: &App,
    cx: &mut Cx<App>,
    get: fn(&App) -> &CodeState,
    get_mut: fn(&mut App) -> &mut CodeState,
) -> El {
    CodeView::new(get, get_mut)
        .labels(labels::code())
        .on_intent(|app: &mut App, intent, _| {
            if intent == CodeIntent::Leave {
                app.preview.drawer.focus_files();
            }
        })
        .build(app, &app.look, cx)
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
        card = card.focusable(true);
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
