// SPDX-License-Identifier: MIT

//! The context menu's builder: the host's state by accessor, its model as data, its intents
//! out.

use std::rc::Rc;

use rune::{El, Tx, el};
use strata_kit::look::Look;

use super::state::{ContextMenuState, MenuIntent, MenuTarget};
use super::view::{MIN_WIDTH, Render, width_with_min};

type Emit<S> = Rc<dyn Fn(&mut S, MenuIntent, &mut Tx)>;
type Dismissed<S> = Rc<dyn Fn(&mut S, MenuTarget, &mut Tx)>;

/// A context menu over [`ContextMenuState`]:
///
/// ```ignore
/// ContextMenu::new("pane-menu", |s: &App| &s.menu, |s: &mut App| &mut s.menu)
///     .on_intent(|s: &mut App, intent, tx| s.run(intent, tx))
///     .build(s, &s.look, &mut layer)
/// ```
///
/// Place the element it returns anywhere in the view (it takes no room) and mount `layer` at
/// the root (`rune_kit::root`). While open, the menu holds the keyboard; when it closes, the
/// keyboard goes back to what had it (the collection, the field).
pub struct ContextMenu<S> {
    key: String,
    get: fn(&S) -> &ContextMenuState,
    get_mut: fn(&mut S) -> &mut ContextMenuState,
    name: String,
    min_width: f32,
    emit: Option<Emit<S>>,
    dismissed: Option<Dismissed<S>>,
}

impl<S: 'static> ContextMenu<S> {
    /// The menu keyed `key`, over the state `get` and `get_mut` reach. Its surface is
    /// `{key}-menu` and its rows `{key}-item{n}` (rune-kit's keys), `n` counting the header,
    /// labels and separators before them.
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
            emit: None,
            dismissed: None,
        }
    }

    /// The menu's accessible name: "Context menu" by default. The host names it in the
    /// interface language, after its target if it likes ("Notes.md").
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    /// The least width the top-level menu opens at, in window points (floored at 160 pt).
    pub fn min_width(mut self, width: f32) -> Self {
        self.min_width = width;
        self
    }

    /// Hears the menu's intents: an item chosen. The menu has closed by then.
    pub fn on_intent(mut self, f: impl Fn(&mut S, MenuIntent, &mut Tx) + 'static) -> Self {
        self.emit = Some(Rc::new(f));
        self
    }

    /// Hears the menu for a target close without a choice: Escape or a press outside it. The
    /// selection is never touched.
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
