// SPDX-License-Identifier: MIT

//! The window's 1 px frame.

use rune::{El, col, el};
use strata_kit::look::Look;

use super::chrome::WindowChrome;

/// `content` in Strata's client-side frame: square corners and a 1 px line in the border
/// colour at 60 % around the window, as Strata's `window.csd` draws it, unless the window is
/// known to be maximised or full screen ([`WindowChrome::framed`]). `content` should fill the
/// window.
///
/// The frame is keyed `key`, and its line `{key}-line`. The line is drawn over the content's
/// edge, and takes no presses and says nothing to assistive technology.
pub fn frame(key: &str, content: El, look: &Look, chrome: &WindowChrome) -> El {
    let root = col().key(key.to_owned()).w_full().h_full().child(content);
    if !chrome.framed() {
        return root;
    }
    let line = look.kit.c.border_base;
    // Four 1 px edge strips rather than one full-window pinned box: `Tx::element_at_with` tests
    // box geometry in paint order and ignores `El::inert`, so a full-window overlay would mask
    // every drop target underneath it.
    root.child(
        el().key(format!("{key}-line"))
            .pin(Some(0.0), Some(0.0), Some(0.0), None)
            .h(1.0)
            .fill(line)
            .inert()
            .a11y_hidden(),
    )
    .child(
        el().pin(Some(0.0), None, Some(0.0), Some(0.0))
            .h(1.0)
            .fill(line)
            .inert()
            .a11y_hidden(),
    )
    .child(
        el().pin(Some(0.0), Some(1.0), None, Some(1.0))
            .w(1.0)
            .fill(line)
            .inert()
            .a11y_hidden(),
    )
    .child(
        el().pin(None, Some(1.0), Some(0.0), Some(1.0))
            .w(1.0)
            .fill(line)
            .inert()
            .a11y_hidden(),
    )
}
