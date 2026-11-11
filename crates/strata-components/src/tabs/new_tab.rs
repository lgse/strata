//! The New tab button: the strip's trailing action, or, with one tab and no strip, a header
//! action (`browser/tabs`: "With one tab the strip stays hidden during these drags; dropping
//! on the header's New tab button opens the tab").

use std::borrow::Cow;
use std::rc::Rc;

use rune::{El, Tx};
use strata_kit::icons::IconName;
use strata_kit::look::Look;
use strata_kit::slots::tabs as slot;

use super::drops::{self, Access, Ctx, Emit, OpensFn};
use super::intent::{DropSite, TabsIntent};
use super::{BrowserTabsState, new_tab_key};
use crate::contract::dnd::{Dnd, DropHover, DropOffer};
use crate::header;

/// The words the strip says, for the host to translate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TabsLabels {
    /// The tab list's accessible name.
    pub list: Cow<'static, str>,
    /// The New tab button's accessible name.
    pub new_tab: Cow<'static, str>,
    /// Its tooltip, with the host's shortcut.
    pub new_tab_help: Cow<'static, str>,
}

impl Default for TabsLabels {
    fn default() -> Self {
        TabsLabels {
            list: Cow::Borrowed("Tabs"),
            new_tab: Cow::Borrowed("New tab"),
            new_tab_help: Cow::Borrowed("New tab (Ctrl+T)"),
        }
    }
}

/// The New tab button on its own, for a header that holds it while the strip is hidden. It
/// reports [`TabsIntent::New`], and as a drop zone keyed [`new_tab_key`] it opens a drag the
/// host says opens tabs at the end of `len` tabs ([`TabsIntent::OpenTabs`]).
pub struct NewTabButton<'a, S: 'static> {
    state: &'a BrowserTabsState,
    access: Access<S>,
    len: usize,
    dnd: Dnd<S>,
    opens: Option<OpensFn<S>>,
    labels: TabsLabels,
    emit: Emit<S>,
}

impl<'a, S: 'static> NewTabButton<'a, S> {
    pub fn new(state: &'a BrowserTabsState, access: Access<S>, len: usize) -> Self {
        NewTabButton {
            state,
            access,
            len,
            dnd: Dnd::rune(),
            opens: None,
            labels: TabsLabels::default(),
            emit: Rc::new(|_, _, _| {}),
        }
    }

    /// The host's drag-and-drop binder (default Rune's).
    pub fn dnd(mut self, dnd: Dnd<S>) -> Self {
        self.dnd = dnd;
        self
    }

    /// Which drags open tabs: see [`super::BrowserTabs::opens_tabs`].
    pub fn opens_tabs(mut self, f: impl Fn(&S, &DropOffer) -> bool + 'static) -> Self {
        self.opens = Some(Rc::new(f));
        self
    }

    pub fn labels(mut self, labels: TabsLabels) -> Self {
        self.labels = labels;
        self
    }

    pub fn on_intent(mut self, f: impl Fn(&mut S, TabsIntent, &mut Tx) + 'static) -> Self {
        self.emit = Rc::new(f);
        self
    }

    pub fn build(self, look: &Look) -> El {
        let cx = Ctx {
            access: self.access,
            emit: self.emit,
            action: None,
            opens: self.opens,
            content: &[],
            spring: None,
            len: self.len,
        };
        button(&cx, self.state, &self.dnd, &self.labels, look)
    }
}

/// The button, lit while a drag that opens tabs is over it or runs in the window.
pub(super) fn button<S: 'static>(
    cx: &Ctx<S>,
    state: &BrowserTabsState,
    dnd: &Dnd<S>,
    labels: &TabsLabels,
    look: &Look,
) -> El {
    let emit = cx.emit.clone();
    let mut el = header::action(look, IconName::Plus, &labels.new_tab_help)
        .label(labels.new_tab.clone().into_owned())
        .on_press_tx(move |s: &mut S, tx: &mut Tx| emit(s, TabsIntent::New, tx))
        .build(&look.kit)
        .key(new_tab_key(state.key()));
    let hovered = state.drop_hover() == Some((DropSite::NewTab, DropHover::Accepting));
    if hovered || state.open_drag() {
        // Strata's `.tab-drop-hover`: the header action's radius, an accent ground.
        el = el.fill(slot::new_drop_fill(look)).radius(look.round(5.0));
    }
    dnd.drop_zone_keyed(
        el,
        new_tab_key(state.key()),
        drops::opener(cx, DropSite::NewTab),
    )
}
