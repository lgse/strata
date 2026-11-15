// SPDX-License-Identifier: MIT

//! The hosted preview viewer seam inside [`super::Preview`].
//!
//! `A15` mounts C9's [`PreviewDrawer`](strata_components::preview_drawer::PreviewDrawer) once in
//! [`Place::Drawer`](crate::slots::Place::Drawer) and delegates header capabilities, find, and
//! content rendering through [`Viewer`]:
//! - `A15` owns plain text and code (`Viewer::Code`), plus rendered documents, spreadsheets, and
//!   still images in subsequent `A15` slices.
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
    preview_drawer::{DocumentView, FindStatus, HostedPreview},
};
use strata_core::i18n::tr;
use strata_kit::look::Look;

use super::labels;
use crate::{App, buttons};

/// Key of the read-only source/text viewer inside the preview drawer.
pub(crate) const CODE_KEY: &str = "preview-code";

pub(super) enum Viewer {
    Empty,
    Loading,
    Code(Box<CodeState>),
    Message {
        title: String,
        detail: Option<String>,
        can_open: bool,
    },
}

/// Source text and metadata handed to [`Viewer::code`].
pub(super) struct CodeSource<'a> {
    pub(super) content: String,
    pub(super) truncated: bool,
    pub(super) wrap: bool,
    pub(super) file_name: &'a str,
}

impl Viewer {
    pub(super) fn is_empty(&self) -> bool {
        matches!(self, Self::Empty)
    }

    pub(super) fn code(source: CodeSource<'_>, highlighter: &Highlighter) -> Self {
        let mut state = CodeState::new(source.content).key(CODE_KEY).highlighter(
            highlighter,
            Some(source.file_name),
            None,
        );
        state.set_wrap(source.wrap);
        if source.truncated {
            state.set_notices(vec![tr("Preview limited to the first 1 MB")]);
        }
        Self::Code(Box::new(state))
    }

    pub(super) fn poll(&mut self, tx: &mut Tx) -> bool {
        match self {
            Self::Code(code) => code.poll(tx),
            Self::Empty | Self::Loading | Self::Message { .. } => false,
        }
    }

    pub(super) fn set_wrap(&mut self, wrap: bool) {
        if let Self::Code(code) = self {
            code.set_wrap(wrap);
        }
    }

    pub(super) fn build(&self, app: &App, cx: &mut Cx<App>) -> Option<El> {
        match self {
            Self::Empty => None,
            Self::Loading => Some(loading_view(&app.look)),
            Self::Code(_) => Some(code_view(app, cx)),
            Self::Message {
                title,
                detail,
                can_open,
            } => Some(message_view(&app.look, title, detail.as_deref(), *can_open)),
        }
    }
}

impl HostedPreview for Viewer {
    fn document_view(&self) -> Option<DocumentView> {
        match self {
            Self::Code(code) => code.document_view(),
            _ => None,
        }
    }

    fn wrap(&self) -> Option<bool> {
        match self {
            Self::Code(code) => code.wrap(),
            _ => None,
        }
    }

    fn searchable(&self) -> bool {
        match self {
            Self::Code(code) => code.searchable(),
            _ => false,
        }
    }

    fn find(&mut self, query: &str) {
        if let Self::Code(code) = self {
            code.find(query);
        }
    }

    fn find_step(&mut self, forward: bool) {
        if let Self::Code(code) = self {
            code.find_step(forward);
        }
    }

    fn find_status(&self) -> Option<FindStatus> {
        match self {
            Self::Code(code) => code.find_status(),
            _ => None,
        }
    }

    fn find_end(&mut self) {
        if let Self::Code(code) = self {
            code.find_end();
        }
    }

    fn busy(&self) -> Option<&str> {
        match self {
            Self::Code(code) => code.busy(),
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

fn code_view(app: &App, cx: &mut Cx<App>) -> El {
    CodeView::new(code_ref, code_mut)
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
        .w_full()
        .h_full()
        .center()
        .child(Spinner::new().build(&look.kit))
}

fn message_view(look: &Look, title: &str, detail: Option<&str>, can_open: bool) -> El {
    let mut card = col()
        .key("preview-message")
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
    if let Some(detail) = detail.filter(|detail| !detail.is_empty()) {
        card = card.child(
            text(detail.to_owned())
                .font(look.kit.ty.ui)
                .color(look.kit.c.text_muted),
        );
    }
    if can_open {
        let open_label = tr("Open Externally");
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
