//! The context menu's builder: the host's state by accessor, its model as data, its intents
//! out.

use std::rc::Rc;

use rune::{El, Tx, el};
use strata_kit::look::Look;

use super::state::{ContextMenuState, MenuIntent, MenuTarget};
use super::view::Render;

type Emit<S> = Rc<dyn Fn(&mut S, MenuIntent, &mut Tx)>;
type Dismissed<S> = Rc<dyn Fn(&mut S, MenuTarget)>;

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

    /// Hears the menu's intents: an item chosen. The menu has closed by then.
    pub fn on_intent(mut self, f: impl Fn(&mut S, MenuIntent, &mut Tx) + 'static) -> Self {
        self.emit = Some(Rc::new(f));
        self
    }

    /// Hears the menu for a target close without a choice: Escape or a press outside it. The
    /// selection is never touched.
    pub fn on_dismiss(mut self, f: impl Fn(&mut S, MenuTarget) + 'static) -> Self {
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
        Render {
            key: &self.key,
            at,
            name: &self.name,
            header: &model.header,
            sections: &model.sections,
            open: st.submenu(),
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
            on_submenu: Rc::new(move |s: &mut S, path: Vec<usize>, keys: bool| {
                let st = get_mut(s);
                if keys {
                    st.enter_submenu(&path)
                } else {
                    st.open_submenu(&path)
                }
            }),
            entering: Rc::new(move |s: &mut S| get_mut(s).take_entering()),
            close_to: Rc::new(move |s: &mut S, depth: usize| {
                get_mut(s).close_submenu(depth);
            }),
            on_dismiss: Rc::new(move |s: &mut S| {
                if let Some(target) = get_mut(s).close()
                    && let Some(f) = &dismissed
                {
                    f(s, target);
                }
            }),
        }
        .build(layer)
    }
}
