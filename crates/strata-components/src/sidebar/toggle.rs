//! [`WidthToggle`]: the header's sidebar button, which steps full, rail and hidden.

use std::rc::Rc;

use rune::{El, Tx};
use rune_kit::IconButton;
use strata_kit::icons::IconName;
use strata_kit::look::Look;

use super::intent::SidebarIntent;
use super::state::SidebarState;
use super::{DEFAULT_KEY, Emit};

/// The sidebar toggle: its accessible name is constant, "Toggle sidebar (Ctrl+B)", so
/// assistive technology and tests find it at every step, and its tooltip names the next step,
/// such as "Hide sidebar (Ctrl+B)". The shortcut is the host's (Ctrl+N in 10xer mode).
/// Keyed `<key>-toggle`.
pub struct WidthToggle<'a, S> {
    key: String,
    state: &'a SidebarState,
    access: fn(&mut S) -> &mut SidebarState,
    shortcut: String,
    emit: Emit<S>,
}

impl<'a, S: 'static> WidthToggle<'a, S> {
    pub fn new(state: &'a SidebarState, access: fn(&mut S) -> &mut SidebarState) -> Self {
        WidthToggle {
            key: DEFAULT_KEY.into(),
            state,
            access,
            shortcut: "Ctrl+B".into(),
            emit: Rc::new(|_, _, _| {}),
        }
    }

    /// The sidebar's key (default `places`).
    pub fn key(mut self, key: impl Into<String>) -> Self {
        self.key = key.into();
        self
    }

    /// The shortcut the name and tooltip end with (default `Ctrl+B`).
    pub fn shortcut(mut self, shortcut: impl Into<String>) -> Self {
        self.shortcut = shortcut.into();
        self
    }

    /// Hears [`SidebarIntent::Stepped`].
    pub fn on_intent(mut self, f: impl Fn(&mut S, SidebarIntent, &mut Tx) + 'static) -> Self {
        self.emit = Rc::new(f);
        self
    }

    pub fn build(self, look: &Look) -> El {
        let access = self.access;
        let emit = self.emit;
        let shortcut = &self.shortcut;
        IconButton::new(look.icon(IconName::Sidebar))
            .label(format!("Toggle sidebar ({shortcut})"))
            .help(format!("{} ({shortcut})", self.state.width.next_label()))
            .on_press_tx(move |s: &mut S, tx: &mut Tx| {
                let step = access(s).width.advance();
                emit(s, SidebarIntent::Stepped(step), tx);
            })
            .build(&look.kit)
            .key(format!("{}-toggle", self.key))
    }
}
