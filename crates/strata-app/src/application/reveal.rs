// SPDX-License-Identifier: MIT

//! A window opened on a FileManager1 call: its folder, with the items the call named selected,
//! as `present_reveal` in `src/ui/window/launch.rs` opens one. For `ShowItemProperties`, the
//! Properties of the entry the reveal leaves under the cursor show once the folder has listed,
//! as GTK's `pending_properties` waits for its load (`reveal_loaded_names` in
//! `src/ui/browser/events/listing.rs`), and forget it on a reset to another location or a
//! failed listing, as GTK's `on_reset` and `abandon_deferred_reveal` do. The window leaves
//! `tabs.toml` alone.

use std::{cell::Cell, rc::Rc};

use rune::Tx;
use strata_fs::{
    adapters::RevealRequest,
    app::{Browser, BrowserEvent},
};

use crate::{App, properties, shell};

/// Reveals `request` in the window's browser.
pub(crate) fn open(app: &mut App, request: RevealRequest) {
    let RevealRequest {
        directory,
        selection,
        properties,
    } = request;
    if selection.is_empty() {
        app.browser.navigate_location(directory, true);
        return;
    }
    if properties {
        show_properties_once_listed(app, directory.clone());
    }
    if app.browser.reveal_locations(directory, selection) && properties {
        // Revealed in place: no load is coming.
        shell::window_later(app).then(show_focused_properties);
    }
}

/// Has the browser queue the Properties of its focused entry when `directory` first lists. The
/// request is forgotten, as GTK forgets `pending_properties`, once the browser resets to
/// another location (`on_reset`) or the folder fails to list, so a later visit never shows
/// Properties unasked.
fn show_properties_once_listed(app: &App, directory: strata_core::model::Location) {
    let later = shell::window_later(app);
    let waiting = Rc::new(Cell::new(true));
    let browser = Rc::downgrade(&app.browser);
    app.browser.observe(move |event| {
        if !waiting.get() {
            return;
        }
        let Some(browser) = browser.upgrade() else {
            return;
        };
        let at = |depth: usize| browser.location_at(depth).as_ref() == Some(&directory);
        match event {
            BrowserEvent::LoadFinished { depth, .. } if at(*depth) => {
                waiting.set(false);
                later.then(show_focused_properties);
            }
            BrowserEvent::Reset if !at(0) => waiting.set(false),
            BrowserEvent::LoadFailed { depth, .. } if at(*depth) => waiting.set(false),
            _ => {}
        }
    });
}

fn show_focused_properties(app: &mut App, _: &mut Tx) -> bool {
    let Some((depth, _, entry)) = Browser::focused_item(&app.browser) else {
        return false;
    };
    properties::show_entry(app, entry, Some(depth));
    true
}
