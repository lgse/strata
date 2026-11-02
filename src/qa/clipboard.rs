// SPDX-License-Identifier: MIT

use gtk::{gdk, glib, prelude::*};

use super::protocol::{self, ClipboardContent};

pub(super) fn private_display() -> Result<gdk::Display, String> {
    let name = gdk::Display::default()
        .map(|display| display.name())
        .ok_or("the application has no display")?;
    gdk::Display::open(Some(&name)).ok_or_else(|| format!("cannot open the display {name}"))
}

pub(super) fn own_clipboard(
    display: &gdk::Display,
    content: ClipboardContent,
) -> Result<(), String> {
    let clipboard = display.clipboard();
    match content {
        ClipboardContent::Text(text) => clipboard.set_text(&text),
        ClipboardContent::Offers(offers) => {
            let providers = offers
                .offers
                .iter()
                .map(|offer| {
                    gdk::ContentProvider::for_bytes(
                        &offer.mime,
                        &glib::Bytes::from_owned(offer.text.clone().into_bytes()),
                    )
                })
                .collect::<Vec<_>>();
            clipboard
                .set_content(Some(&gdk::ContentProvider::new_union(&providers)))
                .map_err(|error| format!("cannot own the clipboard: {error}"))?;
        }
    }
    display.flush();
    Ok(())
}

pub(super) async fn read_clipboard(display: &gdk::Display, mime: Option<&str>) -> Option<String> {
    let clipboard = display.clipboard();
    match mime {
        Some(mime) => {
            let read = crate::ui::read_clipboard_mime_bytes(&clipboard, mime);
            match glib::future_with_timeout(protocol::DEFAULT_WAIT, read).await {
                Ok(Some(bytes)) => Some(String::from_utf8_lossy(&bytes).into_owned()),
                _ => None,
            }
        }
        None => {
            let read = clipboard.read_text_future();
            match glib::future_with_timeout(protocol::DEFAULT_WAIT, read).await {
                Ok(Ok(Some(text))) => Some(text.to_string()),
                _ => None,
            }
        }
    }
}

pub(super) fn abridged(text: &str) -> String {
    const LIMIT: usize = 200;
    match text.char_indices().nth(LIMIT) {
        Some((end, _)) => format!("{}… ({} characters)", &text[..end], text.chars().count()),
        None => text.to_owned(),
    }
}
