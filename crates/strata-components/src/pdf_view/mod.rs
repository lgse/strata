// SPDX-License-Identifier: MIT

mod find;
mod layer;
mod message;
mod model;
mod page_nav;
mod queue;
mod render;
mod selection;
mod state;
pub mod zoom;

#[cfg(test)]
mod tests;

use std::rc::Rc;
use std::sync::Arc;

use rune::{Cx, El, KeyEvent, Tx};
use strata_kit::look::Look;

use crate::contract::Point;
pub use find::Hit;
pub use model::{
    Content, MAX_PAGES, PageReply, PdfDocument, RenderCaps, RenderKind, RenderRequest, TextLayer,
    Ticket,
};
pub use queue::{IMAGE_BUDGET, Landed, TEXT_BUDGET};
pub use state::{PdfState, page_row_identity};

use render::{Ctx, IntentFn, Paint};

/// What the view tells the host.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum PdfIntent {
    Open,
    Leave,
    /// Ctrl+C copied this (the clipboard already has it).
    Copied(Arc<str>),
    /// Secondary press (or Menu key) inside the PDF viewer, in window points.
    ContextMenu(Point),
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct PdfLabels {
    pub document: String,
    pub page: String,
    pub position: String,
    pub loading: String,
    pub failed_page: String,
    pub unavailable: String,
    pub locked: String,
    pub open: String,
    pub page_text: String,
    pub page_navigation: String,
    pub page_indicator: String,
    pub go_to_page: String,
    pub previous_page: String,
    pub next_page: String,
}

impl Default for PdfLabels {
    fn default() -> Self {
        PdfLabels {
            document: "PDF document".into(),
            page: "Page {page} of {pages}".into(),
            position: "Page {page} of {pages}, {zoom}%".into(),
            loading: "Loading page".into(),
            failed_page: "Unable to render this PDF page".into(),
            unavailable: "Preview unavailable".into(),
            locked: "This PDF is password protected.".into(),
            open: "Open".into(),
            page_text: String::new(),
            page_navigation: "Page navigation".into(),
            page_indicator: "Page {input} / {pages}".into(),
            go_to_page: "Go to page".into(),
            previous_page: "Previous page".into(),
            next_page: "Next page".into(),
        }
    }
}

/// The view: `PdfView::new(get, get_mut) … .build(s, look, cx)`. It fills its parent.
pub struct PdfView<S> {
    get: fn(&S) -> &PdfState,
    get_mut: fn(&mut S) -> &mut PdfState,
    labels: PdfLabels,
    intent: Option<IntentFn<S>>,
    open_button: bool,
    letters: bool,
    reset: fn(&KeyEvent) -> bool,
}

impl<S: 'static> PdfView<S> {
    /// The view of the [`PdfState`] that `get` and `get_mut` find in the app's state.
    pub fn new(get: fn(&S) -> &PdfState, get_mut: fn(&mut S) -> &mut PdfState) -> Self {
        PdfView {
            get,
            get_mut,
            labels: PdfLabels::default(),
            intent: None,
            open_button: true,
            letters: false,
            reset: default_reset,
        }
    }

    pub fn labels(mut self, labels: PdfLabels) -> Self {
        self.labels = labels;
        self
    }

    pub fn on_intent(mut self, f: impl Fn(&mut S, PdfIntent, &mut Tx) + 'static) -> Self {
        self.intent = Some(Rc::new(f));
        self
    }

    pub fn open_button(mut self, on: bool) -> Self {
        self.open_button = on;
        self
    }

    /// 10xer mode's letters on the message: `j` and `l` move to Open, `h` leaves.
    pub fn letters(mut self, on: bool) -> Self {
        self.letters = on;
        self
    }

    pub fn reset_key(mut self, f: fn(&KeyEvent) -> bool) -> Self {
        self.reset = f;
        self
    }

    pub fn build(self, s: &S, look: &Look, cx: &mut Cx<S>) -> El {
        let PdfView {
            get,
            get_mut,
            labels,
            intent,
            open_button,
            letters,
            reset,
        } = self;
        let st = get(s);
        let ctx = Rc::new(Ctx {
            get,
            get_mut,
            labels: Rc::new(labels),
            intent,
            letters,
            reset,
            paint: Paint::from_look(look),
            kit: look.kit.clone(),
        });
        let _ = cx;
        match st.content() {
            Content::Pages => render::pages(ctx, st, look),
            Content::Locked => {
                let message = ctx.labels.locked.clone();
                message::build(&ctx, st, &message, open_button, look)
            }
            Content::Failed(m) => message::build(&ctx, st, m, false, look),
        }
    }
}

fn default_reset(ev: &KeyEvent) -> bool {
    crate::viewers::viewport::ctrl_zero_key(ev).is_some()
}
