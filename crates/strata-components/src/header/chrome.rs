//! What the header knows about its window, as plain data: Strata's window-button preferences,
//! the window manager's state and capabilities, and the actions the buttons ask for.

use strata_kit::icons::IconName;

/// Strata's window-button preferences (Settings → General → Window buttons). A fresh profile
/// shows only Close.
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

/// What the window manager says about the window. The host fills it in from its platform layer
/// (strata-platform's `csd::Csd`: `state()` and `capabilities()`), and from the root's
/// `on_window_focus`. `None` is unknown: on Wayland, Rune reports neither the maximised state nor
/// whether minimise can work (strata-platform drafts U6 and U7, not filed).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct WindowChrome {
    pub maximized: Option<bool>,
    pub fullscreen: Option<bool>,
    /// Whether the Minimize button can work. Only `Some(false)` hides it.
    pub can_minimize: Option<bool>,
    /// Whether the Maximize button can work. Only `Some(false)` hides it.
    pub can_maximize: Option<bool>,
    /// The window has keyboard focus. The header draws the same either way (an owner item:
    /// Strata's stylesheet has no backdrop rule), so this is carried for the host and for a
    /// later backdrop look.
    pub active: bool,
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
        }
    }
}

impl WindowChrome {
    /// Whether the window draws its 1 px frame: unless it is known to be maximised or full
    /// screen. Unknown (Wayland) draws it.
    pub fn framed(&self) -> bool {
        self.maximized != Some(true) && self.fullscreen != Some(true)
    }
}

/// What a window button asks the host to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WindowAction {
    Minimize,
    ToggleMaximize,
    Close,
}

/// The window buttons' names and tooltips. The default is Strata's English; Strata passes its
/// translations.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeaderLabels {
    pub minimize: String,
    pub maximize: String,
    pub restore: String,
    /// The Maximize button while the window's state is unknown (Wayland, U6): it toggles either
    /// way, so its name must fit both.
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

/// Which window buttons the header shows: those the user turned on, less any the window
/// manager says can't work. Unknown shows, and the request goes out.
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
