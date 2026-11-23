// SPDX-License-Identifier: MIT

//! The window's 1 px frame.

use rune::{El, col, el};
use strata_kit::look::Look;

use super::chrome::WindowChrome;

pub fn frame(key: &str, content: El, look: &Look, chrome: &WindowChrome) -> El {
    let root = col().key(key.to_owned()).w_full().h_full().child(content);
    if !chrome.framed() {
        return root;
    }
    let line = look.kit.c.border_base;
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
