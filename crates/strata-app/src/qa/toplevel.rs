// SPDX-License-Identifier: MIT

//! A chooser as its own offscreen toplevel, for the hosts that open one beside or instead of
//! the browser window: the portal's ([`super::portal`]) and the floating destination
//! chooser's ([`super::windows`]).

use std::{cell::Cell, rc::Rc};

use strata_core::model::Location;
use strata_platform::csd::{Backend, Csd, Policy};
use strata_qa::Keymap;

use super::{host::Host, input_log::InputLog, staged::Staged, tree::Node};
use crate::{
    App,
    chooser::{source::ChooserFileSource, state::DestinationHost},
    host::offscreen::Offscreen,
    portal::{ChooserRequest, Completion},
    settings::Saved,
    startup::UiThread,
};

/// What a chooser is built from.
pub(super) struct ChooserParts {
    /// The settings it reads: its browser window's, shared, or the portal process's own.
    pub(super) settings: Saved,
    pub(super) request: ChooserRequest,
    pub(super) source: Rc<ChooserFileSource>,
    pub(super) destination: Option<DestinationHost>,
    pub(super) completion: Completion,
}

/// Where and how a toplevel is staged.
pub(super) struct Placement<'a> {
    pub(super) ui: UiThread,
    pub(super) log: &'a InputLog,
    pub(super) keymap: Option<&'a Keymap>,
    /// Its tree's root path, which no other toplevel's shares.
    pub(super) root: String,
    pub(super) size: (i32, i32),
}

pub(super) struct Toplevel {
    pub(super) root: Node,
    pub(super) token: String,
    pub(super) staged: Rc<Staged>,
    /// Set by the chooser as its request finishes and it closes.
    closed: Rc<Cell<bool>>,
}

impl Toplevel {
    /// The chooser, open on its request's folder and turned once.
    pub(super) fn open(placement: Placement<'_>, parts: ChooserParts) -> Self {
        let Placement {
            ui,
            log,
            keymap,
            root,
            size: (width, height),
        } = placement;
        let token = parts.request.token.clone();
        let initial = Location::local(parts.request.initial_directory.clone());
        let closed = Rc::new(Cell::new(false));
        let mut app = App::new_chooser(
            &ui,
            Csd::new(Policy::Auto.frame(), Backend::Headless),
            parts.settings,
            (
                parts.request,
                parts.source,
                parts.destination,
                parts.completion,
            ),
        );
        if let Some(active) = app.chooser.active.as_mut() {
            let closed = closed.clone();
            active.close_window = Some(Box::new(move |_| closed.set(true)));
        }
        #[expect(clippy::cast_precision_loss, reason = "a chooser's size fits f32")]
        let size = (width as f32, height as f32);
        let host = Offscreen::from_app(ui, app, size).without_animations();
        host.app().open(initial);
        let staged = Rc::new(Staged::named(root.clone(), host, log.clone()));
        if let Some(keymap) = keymap {
            let _ = staged.set_layout(keymap);
        }
        staged.turn();
        Self {
            root: Node(root.into()),
            token,
            staged,
            closed,
        }
    }

    pub(super) fn is_closed(&self) -> bool {
        self.closed.get()
    }

    /// The window's title, as its title bar shows it.
    pub(super) fn title(&self) -> String {
        self.staged.app(|app| app.title().to_owned())
    }
}
