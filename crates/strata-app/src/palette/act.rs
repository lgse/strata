// SPDX-License-Identifier: MIT

//! What opening a result does: GTK's `activate_result` and the palette's `reveal`
//! (`src/ui/window/composition/search.rs`), on the browser.

use rune::Tx;
use strata_core::model::Location;

use crate::{App, filter, preview, shell};

/// Enter or a click on the result `id`; the palette has closed. A folder is navigated to. A
/// file is opened when "Open files directly" is on, shown in the preview drawer otherwise, and
/// revealed in its folder either way (`src/ui/window/composition/search.rs`).
pub(super) fn open(app: &mut App, id: &str, tx: &mut Tx) {
    let Some(found) = app.palette.found.get(id).cloned() else {
        return;
    };
    let location = Location::local(found.path.clone());
    shell::publish(app);
    if found.is_directory {
        preview::clear_search_target(app, tx);
        app.browser.navigate(location);
    } else {
        // Turn the drawer on before revealing so the revealed selection re-targets it.
        if shell::preferences(app).search_open_files_directly {
            app.browser.open_location(location.clone());
        } else {
            preview::show_search_file(app, &found.path, found.name, tx);
        }
        reveal_location(app, location, tx);
    }
    shell::settle(app, tx);
}

/// Alt+Return or the result's menu on `id`: its folder, with it selected; the palette has
/// closed.
pub(super) fn reveal(app: &mut App, id: &str, tx: &mut Tx) {
    let Some(found) = app.palette.found.get(id).cloned() else {
        return;
    };
    shell::publish(app);
    reveal_location(app, Location::local(found.path), tx);
    shell::settle(app, tx);
}

/// `BrowserView::reveal_location`: the folder's filter is emptied, then the browser shows
/// the folder with `location` selected.
fn reveal_location(app: &mut App, location: Location, tx: &mut Tx) {
    let Some(folder) = location.parent() else {
        return;
    };
    if let Some(depth) = app.browser.open_depth(&folder) {
        filter::clear_for_reveal(app, depth, tx);
    }
    app.browser.reveal_locations(folder, vec![location]);
}
