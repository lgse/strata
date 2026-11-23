// SPDX-License-Identifier: MIT

use rune::Tx;
use strata_core::model::Location;

use crate::{App, filter, preview, shell};

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

pub(super) fn reveal(app: &mut App, id: &str, tx: &mut Tx) {
    let Some(found) = app.palette.found.get(id).cloned() else {
        return;
    };
    shell::publish(app);
    preview::clear_search_target(app, tx);
    reveal_location(app, Location::local(found.path), tx);
    shell::settle(app, tx);
}

fn reveal_location(app: &mut App, location: Location, tx: &mut Tx) {
    let Some(folder) = location.parent() else {
        return;
    };
    if let Some(depth) = app.browser.open_depth(&folder) {
        filter::clear_for_reveal(app, depth, tx);
    }
    app.browser.reveal_locations(folder, vec![location]);
}
