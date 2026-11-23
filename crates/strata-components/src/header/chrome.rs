// SPDX-License-Identifier: MIT

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
