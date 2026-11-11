// SPDX-License-Identifier: MIT

use strata_desktop::settings::TextSize;

use crate::ui::keys::{
    KeyPress,
    bindings::{TEXT_LARGER, TEXT_RESET, TEXT_SMALLER},
};

/// The text-size shortcuts, which need the key catalog the saved size does not.
pub(in crate::ui) trait TextSizeShortcut: Sized {
    fn for_shortcut(self, press: &KeyPress) -> Option<Self>;
}

impl TextSizeShortcut for TextSize {
    fn for_shortcut(self, press: &KeyPress) -> Option<Self> {
        if press.is(TEXT_LARGER) {
            Some(self.stepped(1))
        } else if press.is(TEXT_SMALLER) {
            Some(self.stepped(-1))
        } else if press.is(TEXT_RESET) {
            Some(Self::default())
        } else {
            None
        }
    }
}
