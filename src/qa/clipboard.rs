// SPDX-License-Identifier: MIT

use gtk::{gdk, glib, prelude::*};

use super::protocol;

pub(super) fn private_display() -> Result<gdk::Display, String> {
    let name = gdk::Display::default()
        .map(|display| display.name())
        .ok_or("the application has no display")?;
    gdk::Display::open(Some(&name)).ok_or_else(|| format!("cannot open the display {name}"))
}

pub(super) async fn read_clipboard(display: &gdk::Display) -> Option<String> {
    let read = display.clipboard().read_text_future();
    match glib::future_with_timeout(protocol::DEFAULT_WAIT, read).await {
        Ok(Ok(Some(text))) => Some(text.to_string()),
        _ => None,
    }
}

pub(super) fn abridged(text: &str) -> String {
    const LIMIT: usize = 200;
    match text.char_indices().nth(LIMIT) {
        Some((end, _)) => format!("{}… ({} characters)", &text[..end], text.chars().count()),
        None => text.to_owned(),
    }
}
