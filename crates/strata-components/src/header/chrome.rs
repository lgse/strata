// SPDX-License-Identifier: MIT

use rune::{Cx, DecorationMode, TiledEdges};
use strata_kit::icons::IconName;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct WindowButtons {
    pub minimize: bool,
    pub maximize: bool,
    pub close: bool,
}

impl Default for WindowButtons {
    fn default() -> WindowButtons {
        WindowButtons {
            minimize: false,
            maximize: false,
            close: true,
        }
    }
}

impl WindowButtons {
    /// Every button on.
    pub const ALL: WindowButtons = WindowButtons {
        minimize: true,
        maximize: true,
        close: true,
    };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct WindowChrome {
    pub maximized: Option<bool>,
    pub fullscreen: Option<bool>,
    /// Whether the Minimize button can work. Only `Some(false)` hides it.
    pub can_minimize: Option<bool>,
    /// Whether the Maximize button can work. Only `Some(false)` hides it.
    pub can_maximize: Option<bool>,
    pub active: bool,
    pub decoration_mode: DecorationMode,
    pub tiled_edges: Option<TiledEdges>,
}

impl Default for WindowChrome {
    /// An active window that is known not to be maximised, with every capability unknown.
    fn default() -> WindowChrome {
        WindowChrome {
            maximized: Some(false),
            fullscreen: Some(false),
            can_minimize: None,
            can_maximize: None,
            active: true,
            decoration_mode: DecorationMode::Unknown,
            tiled_edges: None,
        }
    }
}

impl WindowChrome {
    /// Captures the reactive window-manager state from Rune's view context.
    pub fn from_cx<S>(cx: &Cx<S>, active: bool) -> WindowChrome {
        let actions = cx.window_actions();
        WindowChrome {
            maximized: Some(cx.is_maximized()),
            fullscreen: Some(cx.is_full_screen()),
            can_minimize: actions.minimize,
            can_maximize: actions.maximize,
            active,
            decoration_mode: cx.decoration_mode(),
            tiled_edges: cx.tiled_edges(),
        }
    }

    pub fn framed(&self) -> bool {
        self.maximized != Some(true)
            && self.fullscreen != Some(true)
            && self.decoration_mode != DecorationMode::Server
            && !self.tiled_edges.is_some_and(TiledEdges::any)
    }

    pub(super) fn shows_resize_bands(&self, borderless: bool) -> bool {
        borderless
            && self.maximized != Some(true)
            && self.fullscreen != Some(true)
            && self.decoration_mode != DecorationMode::Server
    }
}

/// What a window button asks the host to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WindowAction {
    Minimize,
    ToggleMaximize,
    Close,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeaderLabels {
    pub minimize: String,
    pub maximize: String,
    pub restore: String,
    pub maximize_or_restore: String,
    pub close: String,
}

impl Default for HeaderLabels {
    fn default() -> HeaderLabels {
        HeaderLabels {
            minimize: "Minimize window".into(),
            maximize: "Maximize window".into(),
            restore: "Restore window".into(),
            maximize_or_restore: "Maximize or restore window".into(),
            close: "Close window".into(),
        }
    }
}

impl HeaderLabels {
    /// The Maximize button's icon and name for the window's state.
    pub fn maximize_face(&self, maximized: Option<bool>) -> (IconName, &str) {
        match maximized {
            Some(true) => (IconName::Restore, &self.restore),
            Some(false) => (IconName::Maximize, &self.maximize),
            None => (IconName::Maximize, &self.maximize_or_restore),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shown {
    pub minimize: bool,
    pub maximize: bool,
    pub close: bool,
}

impl Shown {
    pub fn new(buttons: WindowButtons, chrome: &WindowChrome) -> Shown {
        Shown {
            minimize: buttons.minimize && chrome.can_minimize != Some(false),
            maximize: buttons.maximize && chrome.can_maximize != Some(false),
            close: buttons.close,
        }
    }
}
