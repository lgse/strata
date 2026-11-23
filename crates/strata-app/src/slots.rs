// SPDX-License-Identifier: MIT

use std::rc::Rc;

use rune::{Cx, El, Tx};
use strata_components::{contract::EntryId, rename::Polled};
use strata_desktop::settings::Preferences;
use strata_fs::app::VisibleDepths;

use crate::{
    App,
    commands::{Action, Holds},
    events::Change,
};

mod ids;
mod inbox;
mod registry;
mod worn;

pub(crate) use ids::Ids;
pub(crate) use inbox::{Heard, Inbox, Later, Listener, Queued, Source, TabKey, Update, WeakLater};
pub(crate) use registry::{Entry, Registry, ShownView};
pub(crate) use worn::{SavedGlyph, Worn, color_of, path_customization};

/// A place in the window. The module docs say where each is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Slot {
    Tabs,
    Location,
    Sidebar,
    View,
    Drawer,
    Footer,
    Overlays,
    Dialogs,
}

impl Slot {
    /// The slots the shell fills with one element: a second part there is a mistake.
    pub(crate) const fn holds_one(self) -> bool {
        matches!(
            self,
            Self::Tabs | Self::Location | Self::Sidebar | Self::Drawer
        )
    }
}

/// What a part is built from, and what its listener may keep.
pub(crate) struct Setup<'a> {
    /// The saved settings, as the file held them when the process started.
    pub(crate) settings: &'a Preferences,
    /// The one table that names an entry for the components.
    pub(crate) ids: &'a Ids,
    /// The saved item colours and icons, indexed by [`EntryId`].
    pub(crate) worn: &'a Rc<Worn>,
    /// Where the callback of a service other than `Browser` queues: clone it into the state.
    pub(crate) later: &'a Later<App>,
    /// Which view is shown, now and later: clone it into the state of what asks after start.
    pub(crate) shown: &'a ShownView,
}

/// What a part builds its element with, for one run of the view.
pub(crate) struct Frame<'a, S: 'static> {
    pub(crate) cx: &'a mut Cx<S>,
    pub(crate) layer: &'a mut Vec<El>,
}

/// Builds a part's element for its slot's place, or nothing this time.
pub(crate) type Build<S> = fn(&S, &mut Frame<'_, S>) -> Option<El>;

/// Builds the drawer around the files: the two beside each other, as one element.
pub(crate) type Wrap<S> = fn(&S, El, &mut Frame<'_, S>) -> El;

/// What a view gives the shell.
pub(crate) struct View<S: 'static> {
    pub(crate) build: Build<S>,
    pub(crate) shows: fn(&str) -> bool,
    /// The key of the element that takes the keyboard when a field or a menu gives it back.
    pub(crate) focus: &'static str,
    pub(crate) supports_type_grouping: bool,
    pub(crate) rechecks: VisibleDepths,
    pub(crate) apply: fn(&mut S, Change, &mut Tx) -> bool,
    pub(crate) returns: Option<fn(&S) -> bool>,
    pub(crate) pointer_at: Option<PointerAt<S>>,
    pub(crate) end_gestures: fn(&mut S),
    pub(crate) rows: Rows<S>,
    pub(crate) editing: Editing<S>,
    #[cfg(feature = "qa")]
    pub(crate) probe: Probe<S>,
}

/// A row of the shown view: the browser's column, and the entry's id.
pub(crate) type At = (usize, EntryId);

pub(crate) struct Rows<S: 'static> {
    /// The browser's position of a row.
    pub(crate) position: fn(&S, At) -> Option<usize>,
    /// The entry so many rows after a row, as the view lists them.
    pub(crate) neighbour: fn(&S, At, isize) -> Option<EntryId>,
    /// Opens a row's entry as a click on it does.
    pub(crate) activate: fn(&mut S, At),
    /// Selects a row alone, with the cursor and the keyboard on it.
    pub(crate) select_only: fn(&mut S, At, &mut Tx),
    /// Selects every row of one of the browser's columns.
    pub(crate) select_all: fn(&mut S, usize),
    pub(crate) reveal: fn(&mut S, At, &mut Tx),
}

/// What the rename editor asks of the view it edits in.
pub(crate) struct Editing<S: 'static> {
    pub(crate) begin: fn(&mut S, At, &mut Tx) -> Option<Polled>,
    /// One poll of the editor against the row's listing and viewport.
    pub(crate) tick: fn(&mut S) -> Polled,
    pub(crate) hold: Option<fn(&S, bool)>,
    /// For such a view: what follows the cursor follows it now, with no wait.
    pub(crate) follow_now: Option<fn(&S)>,
}

#[cfg(feature = "qa")]
pub(crate) struct Probe<S: 'static> {
    pub(crate) name: &'static str,
    /// Rune's role of an entry list, and of an entry in one.
    pub(crate) roles: (&'static str, &'static str),
    /// Whether the folder the launch opened has been listed, or could not be.
    pub(crate) listed: fn(&S) -> bool,
    /// The key of each entry list in the tree, the first pane's first.
    pub(crate) lists: fn(&S) -> Vec<String>,
    pub(crate) presented: fn(&S) -> crate::handover::Presented<'_>,
}

/// What the sidebar gives the shell.
pub(crate) struct Sidebar<S: 'static> {
    pub(crate) build: Build<S>,
    pub(crate) focus: fn(&S) -> Option<String>,
    pub(crate) held_back: fn(&S) -> f32,
}

/// What the tab strip gives the shell.
pub(crate) struct Strip<S: 'static> {
    pub(crate) build: fn(&S, Option<El>, &mut Frame<'_, S>) -> Option<El>,
    pub(crate) new_tab: Option<fn(&S, &'static str) -> El>,
    /// Whether the strip shows. The header then leaves out New tab and the window's Close.
    pub(crate) shows: fn(&S) -> bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Scope {
    Tab,
    Window,
}

impl Scope {
    pub(crate) const fn of_tab(self) -> bool {
        matches!(self, Self::Tab)
    }
}

/// Tells a view the pointer moved over the body, to this place in window points.
pub(crate) type PointerAt<S> = fn(&mut S, (f32, f32));

pub(crate) enum Place<S: 'static> {
    Tabs(Strip<S>),
    Location(Build<S>),
    Sidebar(Sidebar<S>),
    View(View<S>),
    Drawer(Wrap<S>),
    Footer(Build<S>),
    Overlays(Build<S>),
    Dialogs(Build<S>),
}

impl<S> Place<S> {
    pub(crate) const fn slot(&self) -> Slot {
        match self {
            Self::Tabs(_) => Slot::Tabs,
            Self::Location(_) => Slot::Location,
            Self::Sidebar(_) => Slot::Sidebar,
            Self::View(_) => Slot::View,
            Self::Drawer(_) => Slot::Drawer,
            Self::Footer(_) => Slot::Footer,
            Self::Overlays(_) => Slot::Overlays,
            Self::Dialogs(_) => Slot::Dialogs,
        }
    }

    pub(crate) const fn build(&self) -> Option<Build<S>> {
        match self {
            Self::View(view) => Some(view.build),
            Self::Sidebar(sidebar) => Some(sidebar.build),
            Self::Drawer(_) | Self::Tabs(_) => None,
            Self::Location(build)
            | Self::Footer(build)
            | Self::Overlays(build)
            | Self::Dialogs(build) => Some(*build),
        }
    }
}

pub(crate) trait Part: Sized + 'static {
    /// The part's slot, and what it gives the shell there.
    const PLACE: Place<App>;

    /// Whether the part keeps a state for each tab (the default), or one for the window.
    const SCOPE: Scope = Scope::Tab;

    fn new(setup: &Setup<'_>) -> Self;

    fn listen(_setup: &Setup<'_>) -> Option<Listener<App>> {
        None
    }

    /// Hands the browser what the user changed in this part since it last agreed with it.
    fn publish(_app: &mut App) {}

    /// The queue of the browser's events has been applied, this part's updates among them.
    fn caught_up(_app: &mut App) {}

    fn customized(_app: &mut App) {}

    fn switched(_app: &mut App, _tx: &mut Tx) {}

    fn tab_hidden(_app: &mut App, _tx: &mut Tx) {}

    fn tab_shown(_app: &mut App, _tx: &mut Tx) {}

    /// One poll: the part's own timers. True when the view has to run.
    fn poll(_app: &mut App, _clock: f64, _tx: &mut Tx) -> bool {
        false
    }

    const COMMANDS: &'static [Action<App>] = &[];

    fn keyboard(_app: &App) -> Option<Holds> {
        None
    }
}

macro_rules! parts {
    ($($field:ident: $part:ty,)*) => {
        pub(crate) struct App {
            pub(crate) look: ::strata_kit::look::Look,
            pub(crate) browser: ::std::rc::Rc<::strata_fs::app::Browser>,
            pub(crate) window: $crate::shell::Window,
            $(pub(crate) $field: $part,)*
        }

        /// Every registered part's state, built and not yet in an [`App`].
        pub(crate) struct Built {
            $($field: $part,)*
        }

        impl Built {
            pub(crate) fn new(
                tab: &$crate::slots::Setup<'_>,
                window: &$crate::slots::Setup<'_>,
            ) -> Self {
                Self {
                    $($field: {
                        let scope = <$part as $crate::slots::Part>::SCOPE;
                        <$part as $crate::slots::Part>::new(if scope.of_tab() { tab } else { window })
                    },)*
                }
            }
        }

        /// The state of each part a hidden tab keeps for itself: none of the window's parts.
        pub(crate) struct TabParts {
            $($field: ::std::option::Option<$part>,)*
        }

        impl TabParts {
            /// Builds the parts of a new tab, in the order they are registered.
            pub(crate) fn new(setup: &$crate::slots::Setup<'_>) -> Self {
                Self {
                    $($field: <$part as $crate::slots::Part>::SCOPE
                        .of_tab()
                        .then(|| <$part as $crate::slots::Part>::new(setup)),)*
                }
            }
        }

        impl App {
            pub(crate) fn swap_parts(&mut self, parked: &mut TabParts) {
                $(if let ::std::option::Option::Some(part) = parked.$field.as_mut() {
                    ::std::mem::swap(&mut self.$field, part);
                })*
            }
        }

        impl App {
            pub(crate) fn assemble(
                look: ::strata_kit::look::Look,
                browser: ::std::rc::Rc<::strata_fs::app::Browser>,
                window: $crate::shell::Window,
                parts: Built,
            ) -> Self {
                Self {
                    look,
                    browser,
                    window,
                    $($field: parts.$field,)*
                }
            }
        }

        /// The registered parts, in the order of their lines.
        pub(crate) const PARTS: &[$crate::slots::Entry<App>] = &[
            $($crate::slots::Entry::of::<$part>(stringify!($field)),)*
        ];

        #[cfg(test)]
        pub(crate) fn parts_and(
            more: Vec<$crate::slots::Entry<App>>,
        ) -> Vec<$crate::slots::Entry<App>> {
            let mut parts = vec![$($crate::slots::Entry::of::<$part>(stringify!($field)),)*];
            parts.extend(more);
            parts
        }
    };
}
pub(crate) use parts;

#[cfg(test)]
mod tests;
