// SPDX-License-Identifier: MIT

use std::rc::Rc;

use rune::{El, Tx, el};
use strata_kit::look::Look;

use super::state::{ContextMenuState, MenuIntent, MenuTarget};
use super::view::{MIN_WIDTH, Render, width_with_min};

type Emit<S> = Rc<dyn Fn(&mut S, MenuIntent, &mut Tx)>;
type Dismissed<S> = Rc<dyn Fn(&mut S, MenuTarget, &mut Tx)>;

pub struct ContextMenu<S> {
    key: String,
    get: fn(&S) -> &ContextMenuState,
    get_mut: fn(&mut S) -> &mut ContextMenuState,
    name: String,
    min_width: f32,
    keeps_text_selection: bool,
    emit: Option<Emit<S>>,
    dismissed: Option<Dismissed<S>>,
}

impl<S: 'static> ContextMenu<S> {
    pub fn new(
        key: impl Into<String>,
        get: fn(&S) -> &ContextMenuState,
        get_mut: fn(&mut S) -> &mut ContextMenuState,
    ) -> Self {
        Self {
            key: key.into(),
            get,
            get_mut,
            name: "Context menu".to_owned(),
            min_width: MIN_WIDTH,
            keeps_text_selection: false,
            emit: None,
            dismissed: None,
        }
    }

    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    /// The least width the top-level menu opens at, in window points (floored at 160 pt).
    pub fn min_width(mut self, width: f32) -> Self {
        self.min_width = width;
        self
    }

    pub fn keeps_text_selection(mut self, on: bool) -> Self {
        self.keeps_text_selection = on;
        self
    }

    /// Hears the menu's intents: an item chosen. The menu has closed by then.
    pub fn on_intent(mut self, f: impl Fn(&mut S, MenuIntent, &mut Tx) + 'static) -> Self {
        self.emit = Some(Rc::new(f));
        self
    }

    pub fn on_dismiss(mut self, f: impl Fn(&mut S, MenuTarget) + 'static) -> Self {
        self.dismissed = Some(Rc::new(move |s, target, _| f(s, target)));
        self
    }

    /// Like [`Self::on_dismiss`], with the closing transaction's [`Tx`].
    pub fn on_dismiss_tx(mut self, f: impl Fn(&mut S, MenuTarget, &mut Tx) + 'static) -> Self {
        self.dismissed = Some(Rc::new(f));
        self
    }

    /// The menu's anchor element, with its surfaces pushed into `layer` while it is open.
    pub fn build(self, s: &S, look: &Look, layer: &mut Vec<El>) -> El {
        let st = (self.get)(s);
        let (Some(model), Some(at)) = (st.model(), st.at()) else {
            return el().absolute(0.0, 0.0);
        };
        let (get_mut, emit, dismissed) = (self.get_mut, self.emit, self.dismissed);
        let top_width = st.resolve_width(look.kit.ty.ui_compact.size, self.min_width, || {
            width_with_min(look, &model.header, &model.sections, self.min_width)
        });
        let open = st.submenu();
        let scroll_bars = (0..=open.len()).map(|depth| st.scroll_bar(depth)).collect();
        Render {
            key: &self.key,
            at,
            name: &self.name,
            header: &model.header,
            sections: &model.sections,
            open,
            top_width,
            keeps_text_selection: self.keeps_text_selection,
            scroll_bars,
            look,
            on_select: Rc::new(move |s: &mut S, path: &[usize], tx: &mut Tx| {
                let st = get_mut(s);
                let action = st
                    .model()
                    .and_then(|m| m.item_at(path))
                    .map(|i| i.action.clone());
                let target = st.close();
                if let (Some(action), Some(target), Some(emit)) = (action, target, &emit) {
                    emit(s, MenuIntent::Activate { target, action }, tx);
                }
            }),
            on_submenu: Rc::new(move |s: &mut S, path: Vec<usize>| {
                get_mut(s).open_submenu(&path);
            }),
            close_to: Rc::new(move |s: &mut S, depth: usize| {
                get_mut(s).close_submenu(depth);
            }),
            on_dismiss: Rc::new(move |s: &mut S, tx: &mut Tx| {
                if let Some(target) = get_mut(s).close()
                    && let Some(f) = &dismissed
                {
                    f(s, target, tx);
                }
            }),
        }
        .build(layer)
    }
}
